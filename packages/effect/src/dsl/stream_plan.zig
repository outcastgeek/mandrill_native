//! Pull-demand composition using the existing Stream, Scope and Effect kernel.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const rt = @import("../internal/runtime.zig");
const scopes = @import("../scope.zig");
const inference = @import("inference.zig");
const ownership = @import("ownership.zig");
const roles = @import("roles.zig");

pub fn requireStream(comptime T: type) void {
    if (@typeInfo(T) != .@"struct" or !@hasDecl(T, "Item") or !@hasDecl(T, "Error") or !@hasDecl(T, "Environment")) @compileError("stream plan requires an exact Stream(Item,E,R)");
    if (T != effects.Stream(T.Item, T.Error, T.Environment)) @compileError("stream plan requires an exact Stream(Item,E,R)");
}

/// Append inert stream descriptions. Only the demanded member opens; each
/// member's lexical scope closes before the next member is initialized.
pub fn append(comptime sources: anytype) AppendType(sources) {
    const Source = AppendType(sources);
    const Index = struct { value: usize = 0 };
    return unfold(struct {
        fn initialize(_: *const Source.Environment) Index {
            return .{};
        }
    }.initialize, struct {
        fn next(index: *Index, env: *const Source.Environment) ?Source {
            inline for (sources, 0..) |source, n| if (index.value == n) {
                index.value += 1;
                return source;
            };
            _ = env;
            return null;
        }
    }.next);
}
fn AppendType(comptime sources: anytype) type {
    if (sources.len == 0) @compileError("stream append requires a source to infer Item/E/R");
    const Source = @TypeOf(sources[0]);
    requireStream(Source);
    for (sources) |source| if (@TypeOf(source) != Source) @compileError("stream append item/error/environment mismatch");
    return Source;
}

fn NextStream(comptime initialize: anytype, comptime next: anytype) type {
    const R = inference.environment(initialize, "stream.unfold.initialize");
    const S = inference.unary(initialize, *const R, "stream.unfold.initialize");
    const Selected = inference.result(next, &.{ *S, *const R }, "stream.unfold.next");
    const Optional = if (inference.isEffect(Selected)) Selected.Success else Selected;
    if (@typeInfo(Optional) != .optional) @compileError("stream.unfold.next requires ?Stream(Item,E,R)");
    const Stream = @typeInfo(Optional).optional.child;
    requireStream(Stream);
    if (Stream.Environment != R) @compileError("stream.unfold environment mismatch");
    if (inference.isEffect(Selected) and Selected != effects.Effect(Optional, Stream.Error, R)) @compileError("stream.unfold.next error/environment mismatch");
    ownership.validate(S);
    return Stream;
}

/// A generated per-reader state machine. A next callback selects an inert
/// stream only when the consumer requests another chunk. State owns all values
/// it initializes; generated cleanup runs after the active child scope drains.
pub fn unfold(comptime initialize: anytype, comptime next: anytype) NextStream(initialize, next) {
    const Source = NextStream(initialize, next);
    const A = Source.Item;
    const E = Source.Error;
    const R = Source.Environment;
    const S = inference.unary(initialize, *const R, "stream.unfold.initialize");
    const Reader = effects.StreamReader(A, E, R);
    const Pull = effects.Effect(effects.Chunk(A), E, R);
    const Open = effects.Effect(Reader, E, R);
    const Unit = effects.Effect(void, E, R);
    const Next = effects.Effect(?Source, E, R);
    const NextResult = inference.result(next, &.{ *S, *const R }, "stream.unfold.next");
    const Scope = scopes.Scope(E);
    const Frame = struct {
        const Self = @This();
        allocator: std.mem.Allocator,
        state: S,
        scope: Scope,
        ended: bool = false,
        reader: ?Reader = null,
        parent: *Pull.RuntimeAdapter.Job = undefined,
        selecting: Next.RuntimeAdapter.Job = undefined,
        selected: outcomes.Exit(?Source, E) = undefined,
        opening: Open.RuntimeAdapter.Job = undefined,
        opened: outcomes.Exit(Reader, E) = undefined,
        pulling: Pull.RuntimeAdapter.Job = undefined,
        pulled: outcomes.Exit(effects.Chunk(A), E) = undefined,
        closing: Unit.RuntimeAdapter.Job = undefined,
        closed: outcomes.Exit(void, E) = undefined,
        releasing: *Unit.RuntimeAdapter.Job = undefined,
        const Acquire = effects.Effect(*Self, E, R);
        fn allocate(env: *const R) outcomes.Exit(*Self, E) {
            const frame = env.allocator.create(Self) catch return .{ .die = outcomes.Defect.out_of_memory };
            frame.* = .{ .allocator = env.allocator, .state = initialize(env), .scope = Scope.init(env) };
            return .{ .success = frame };
        }
        fn pull(frame: *Self) Pull {
            return .{ .value = .{ .borrowed = frame }, .start = start };
        }
        fn start(active: *rt.State, parent: *Pull.RuntimeAdapter.Job) rt.Step {
            const frame: *Self = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
            frame.parent = parent;
            return frame.demand(active);
        }
        fn demand(frame: *Self, active: *rt.State) rt.Step {
            if (active.cancelled()) return frame.parent.adapterFinish(.interrupted);
            if (frame.ended) return frame.parent.adapterFinish(.{ .success = .end });
            if (frame.reader != null) return frame.read(active);
            const described = if (comptime inference.isEffect(NextResult)) next(&frame.state, frame.parent.env) else Next.fromBorrowed(S, &frame.state, struct {
                fn select(state: *const S, env: *const R) outcomes.Exit(?Source, E) {
                    return .{ .success = next(@constCast(state), env) };
                }
            }.select);
            frame.selecting = .{ .program = described, .owned = described.ownership, .env = frame.parent.env, .inherited = active.services, .sink = .{ .output = &frame.selected, .next_step = .{ .data = frame, .advance = selectedStep } } };
            return frame.selecting.adapterStep();
        }
        fn selectedStep(active: *rt.State, data: ?*anyopaque) rt.Step {
            const frame: *Self = @ptrCast(@alignCast(data.?));
            const selected = active.publish(frame.selected);
            if (selected != .success) return frame.parent.adapterFinish(outcomes.castFailure(effects.Chunk(A), selected));
            const source = selected.success orelse {
                frame.ended = true;
                return frame.parent.adapterFinish(.{ .success = .end });
            };
            const described = source.open(&frame.scope);
            frame.opening = .{ .program = described, .owned = described.ownership, .env = frame.parent.env, .inherited = active.services, .sink = .{ .output = &frame.opened, .next_step = .{ .data = frame, .advance = openedStep } } };
            return frame.opening.adapterStep();
        }
        fn openedStep(active: *rt.State, data: ?*anyopaque) rt.Step {
            const frame: *Self = @ptrCast(@alignCast(data.?));
            const value = active.publish(frame.opened);
            if (value != .success) return frame.parent.adapterFinish(outcomes.castFailure(effects.Chunk(A), value));
            frame.reader = value.success;
            return frame.read(active);
        }
        fn read(frame: *Self, active: *rt.State) rt.Step {
            frame.pulling = .{ .program = frame.reader.?.pull.borrow(), .env = frame.parent.env, .inherited = active.services, .sink = .{ .output = &frame.pulled, .next_step = .{ .data = frame, .advance = pulledStep } } };
            return frame.pulling.adapterStep();
        }
        fn pulledStep(active: *rt.State, data: ?*anyopaque) rt.Step {
            const frame: *Self = @ptrCast(@alignCast(data.?));
            const value = active.publish(frame.pulled);
            if (value != .success or value.success != .end) return frame.parent.adapterFinish(value);
            const described = Unit.closeScope(&frame.scope);
            frame.closing = .{ .program = described, .env = frame.parent.env, .inherited = active.services, .sink = .{ .output = &frame.closed, .next_step = .{ .data = frame, .advance = closedStep } } };
            return frame.closing.adapterStep();
        }
        fn closedStep(active: *rt.State, data: ?*anyopaque) rt.Step {
            const frame: *Self = @ptrCast(@alignCast(data.?));
            frame.reader.?.deinit();
            frame.reader = null;
            frame.scope.deinit();
            const value = frame.closed;
            frame.scope = Scope.init(frame.parent.env);
            if (value != .success) return frame.parent.adapterFinish(outcomes.castFailure(effects.Chunk(A), value));
            return frame.demand(active);
        }
        fn release(frame: *Self) Unit {
            return .{ .value = .{ .borrowed = frame }, .start = releaseStart };
        }
        fn releaseStart(active: *rt.State, parent: *Unit.RuntimeAdapter.Job) rt.Step {
            const frame: *Self = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
            frame.releasing = parent;
            const described = Unit.closeScope(&frame.scope);
            frame.closing = .{ .program = described, .env = parent.env, .inherited = active.services, .sink = .{ .output = &frame.closed, .next_step = .{ .data = frame, .advance = releasedStep } } };
            return frame.closing.adapterStep();
        }
        fn releasedStep(_: *rt.State, data: ?*anyopaque) rt.Step {
            const frame: *Self = @ptrCast(@alignCast(data.?));
            const parent = frame.releasing;
            const value = frame.closed;
            if (frame.reader) |*reader| reader.deinit();
            frame.scope.deinit();
            ownership.discard(&frame.state);
            frame.allocator.destroy(frame);
            return parent.adapterFinish(value);
        }
    };
    return Source.fromPull(comptime Frame.Acquire.fromOutcome(Frame.allocate), Frame.pull, Frame.release);
}

fn LoopStream(comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "stream.loop", &.{ "initialize", "condition", "body", "advance" }, &.{});
    const R = inference.environment(roles.get("initialize", supplied), "stream.loop.initialize");
    const S = inference.unary(roles.get("initialize", supplied), *const R, "stream.loop.initialize");
    if (inference.result(roles.get("condition", supplied), &.{ *const S, *const R }, "stream.loop.condition") != bool) @compileError("stream.loop.condition requires bool");
    if (inference.result(roles.get("advance", supplied), &.{ *S, *const R }, "stream.loop.advance") != void) @compileError("stream.loop.advance requires void; use unfold for Effect-driven iteration selection");
    const Stream = inference.result(roles.get("body", supplied), &.{ *S, *const R }, "stream.loop.body");
    requireStream(Stream);
    if (Stream.Environment != R) @compileError("stream.loop.body environment mismatch");
    return Stream;
}

/// Generate loop cursor and lexical iteration scopes from application roles.
/// Advance runs after the previous reader drains, never across a live yield.
pub fn loop(comptime supplied: anytype) LoopStream(supplied) {
    const Stream = LoopStream(supplied);
    const R = Stream.Environment;
    const S = inference.unary(roles.get("initialize", supplied), *const R, "stream.loop.initialize");
    const State = struct {
        pub const effect_owned_payload = true;
        value: S,
        first: bool = true,
        pub fn deinit(self: *@This()) void {
            ownership.discard(&self.value);
        }
    };
    return unfold(struct {
        fn initialize(env: *const R) State {
            return .{ .value = roles.get("initialize", supplied)(env) };
        }
    }.initialize, struct {
        fn next(state: *State, env: *const R) ?Stream {
            if (!state.first) roles.get("advance", supplied)(&state.value, env);
            state.first = false;
            if (!roles.get("condition", supplied)(&state.value, env)) return null;
            return roles.get("body", supplied)(&state.value, env);
        }
    }.next);
}

fn BoundStream(comptime source: anytype, comptime continuation: anytype) type {
    const Source = @TypeOf(source);
    inference.requireEffect(Source, "stream.bind source");
    const Stream = inference.result(continuation, &.{ *const Source.Success, *const Source.Environment }, "stream.bind continuation");
    requireStream(Stream);
    if (Stream.Error != Source.Error or Stream.Environment != Source.Environment) @compileError("stream.bind item/error/environment mismatch");
    return Stream;
}

/// Bind an ordinary Effect into a stream without running it before demand.
/// Its value lives in the generated reader state through all yielded work.
pub fn bind(comptime source: anytype, comptime continuation: anytype) BoundStream(source, continuation) {
    const Stream = BoundStream(source, continuation);
    const A = @TypeOf(source).Success;
    const R = Stream.Environment;
    const Next = effects.Effect(?Stream, Stream.Error, R);
    const State = struct {
        pub const effect_owned_payload = true;
        initialized: bool = false,
        value: A = undefined,
        pub fn deinit(self: *@This()) void {
            if (self.initialized) outcomes.discardPayload(&self.value);
        }
    };
    return unfold(struct {
        fn initialize(_: *const R) State {
            return .{};
        }
    }.initialize, struct {
        fn next(state: *State, env: *const R) Next {
            if (state.initialized) return Next.succeed(null);
            var described = source;
            return described.flatMapCapturedOwned(env, ?Stream, state, struct {
                fn completed(pointer: *const *State, value: A, current: *const R) Next {
                    pointer.*.value = value;
                    pointer.*.initialized = true;
                    return Next.succeed(continuation(&pointer.*.value, current));
                }
            }.completed);
        }
    }.next);
}

/// Validate both branches, and select exactly one after the first demand.
pub fn branch(comptime condition: anytype, comptime when_true: anytype, comptime when_false: @TypeOf(when_true)) @TypeOf(when_true) {
    const Source = @TypeOf(condition);
    inference.requireEffect(Source, "stream.branch condition");
    if (Source.Success != bool) @compileError("stream.branch condition requires Effect(bool,E,R)");
    return bind(condition, struct {
        fn select(value: *const bool, _: *const Source.Environment) @TypeOf(when_true) {
            return if (value.*) when_true else when_false;
        }
    }.select);
}

/// Yield one value on demand. The generated binding owns it until the yielded
/// chunk has been consumed and the lexical child reader has closed.
fn YieldType(comptime Source: type) type {
    inference.requireEffect(Source, "stream.yield");
    return effects.Stream(Source.Success, Source.Error, Source.Environment);
}
pub fn yieldEffect(comptime source: anytype) YieldType(@TypeOf(source)) {
    const Effect = @TypeOf(source);
    comptime inference.requireEffect(Effect, "stream.yield");
    const A = Effect.Success;
    const E = Effect.Error;
    const R = Effect.Environment;
    const Stream = effects.Stream(A, E, R);
    const Cursor = struct { allocator: std.mem.Allocator, item: *const A, emitted: bool = false };
    const Acquire = effects.Effect(*Cursor, E, R);
    const Pull = effects.Effect(effects.Chunk(A), E, R);
    const Unit = effects.Effect(void, E, R);
    const Roles = struct {
        fn open(value: *const A, _: *const R) Acquire {
            return Acquire.fromBorrowed(A, value, struct {
                fn allocate(item: *const A, env: *const R) outcomes.Exit(*Cursor, E) {
                    const cursor = env.allocator.create(Cursor) catch return .{ .die = outcomes.Defect.out_of_memory };
                    cursor.* = .{ .allocator = env.allocator, .item = item };
                    return .{ .success = cursor };
                }
            }.allocate);
        }
        fn pull(_: *const A, cursor: *Cursor, _: *const R) Pull {
            return Pull.fromBorrowed(Cursor, cursor, struct {
                fn produce(value: *const Cursor, _: *const R) outcomes.Exit(effects.Chunk(A), E) {
                    if (value.emitted) return .{ .success = .end };
                    @constCast(value).emitted = true;
                    const one: *const [1]A = @ptrCast(value.item);
                    return .{ .success = effects.Chunk(A).borrowed(one) };
                }
            }.produce);
        }
        fn release(_: *const A, cursor: *Cursor, _: *const R) Unit {
            return Unit.fromBorrowed(Cursor, cursor, struct {
                fn destroy(value: *const Cursor, _: *const R) outcomes.Exit(void, E) {
                    value.allocator.destroy(@constCast(value));
                    return .{ .success = {} };
                }
            }.destroy);
        }
    };
    return bind(source, struct {
        fn select(value: *const A, _: *const R) Stream {
            return @import("streams.zig").borrowed(value, .{ .open = Roles.open, .pull = Roles.pull, .release = Roles.release });
        }
    }.select);
}
