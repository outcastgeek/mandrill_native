//! Result-producing pull folds. The ordinary Effect bracket owns reader and
//! state through suspended transitions, result production, and protected cleanup.
const std = @import("std");
const effects = @import("effect.zig");
const outcomes = @import("outcome.zig");
const rt = @import("internal/runtime.zig");
const scopes = @import("scope.zig");
const roles = @import("dsl/roles.zig");
const inference = @import("dsl/inference.zig");
const ownership = @import("dsl/ownership.zig");

pub const Control = enum { proceed, stop };
pub fn fold(comptime supplied: anytype) Fold(supplied) {
    return .{};
}
fn Fold(comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "fold", &.{ "initialize", "step", "finish" }, &.{"cleanup"});
    return struct {
        pub fn run(_: @This(), comptime source: anytype) Output(@TypeOf(source), supplied) {
            return runFold(source, supplied);
        }
        /// Capture a runtime stream description in the existing scoped owner.
        /// Its borrowed configuration must cover this operation's execution.
        pub fn runWith(_: @This(), env: anytype, source: anytype) Output(@TypeOf(source), supplied) {
            const Out = Output(@TypeOf(source), supplied);
            return Out.scopedCaptured(env, source, FoldFrame(@TypeOf(source), supplied).scopedSource);
        }
    };
}
fn StateType(comptime Source: type, comptime supplied: anytype) type {
    const T = inference.unary(roles.get("initialize", supplied), *const Source.Environment, "fold.initialize");
    if (inference.isEffect(T)) {
        @import("dsl.zig").checkContext(T, Source.Error, Source.Environment, "fold.initialize");
        return T.Success;
    }
    return T;
}
fn Output(comptime Source: type, comptime supplied: anytype) type {
    const S = StateType(Source, supplied);
    const R = Source.Environment;
    const finalizer = roles.get("finish", supplied);
    if (comptime isTransfer(@TypeOf(finalizer))) return effects.Effect(if (@TypeOf(finalizer).mode == .whole) ownership.Moved(S) else @FieldType(S, @TypeOf(finalizer).field), Source.Error, R);
    const Finished = inference.result(finalizer, &.{ *S, *const R }, "fold.finish");
    if (inference.isEffect(Finished)) {
        @import("dsl.zig").checkContext(Finished, Source.Error, R, "fold.finish");
        return Finished;
    }
    return effects.Effect(Finished, Source.Error, R);
}
fn runFold(comptime source: anytype, comptime supplied: anytype) Output(@TypeOf(source), supplied) {
    const Source = @TypeOf(source);
    const Out = Output(Source, supplied);
    const Frame = FoldFrame(Source, supplied);
    return Out.scoped(struct {
        fn scoped(scope: *scopes.Scope(Source.Error)) Out {
            return Out.deferBorrowed(scopes.Scope(Source.Error), scope, struct {
                fn build(selected: *const scopes.Scope(Source.Error), env: *const Source.Environment) Out {
                    return Frame.scopedSource(&source, @constCast(selected), env);
                }
            }.build);
        }
    }.scoped);
}
fn FoldFrame(comptime Source: type, comptime supplied: anytype) type {
    comptime @import("dsl/stream_plan.zig").requireStream(Source);
    const A = Source.Item;
    const E = Source.Error;
    const R = Source.Environment;
    const S = StateType(Source, supplied);
    const Out = Output(Source, supplied);
    const Unit = effects.Effect(void, E, R);
    const Init = effects.Effect(S, E, R);
    const Transition = effects.Effect(Control, E, R);
    const Open = effects.Effect(effects.StreamReader(A, E, R), E, R);
    const Pull = effects.Effect(effects.Chunk(A), E, R);
    const Scope = scopes.Scope(E);
    const init = roles.get("initialize", supplied);
    const step = roles.get("step", supplied);
    const finish = roles.get("finish", supplied);
    const StepReturn = comptime inference.result(step, &.{ *S, *const A, *const R }, "fold.step");
    if (comptime StepReturn != Control and StepReturn != Transition) @compileError("fold.step expected Control or exact Effect(Control,E,R)");
    comptime ownership.validate(S);
    return struct {
        const Self = @This();
        allocator: std.mem.Allocator,
        scope: *Scope,
        source: Source,
        parent: *Out.RuntimeAdapter.Job = undefined,
        inherited: rt.Services = undefined,
        state: S = undefined,
        initialized: bool = false,
        state_fields: [fieldCount(S)]bool = @splat(true),
        before_job: Unit.RuntimeAdapter.Job = undefined,
        before_exit: outcomes.Exit(void, E) = undefined,
        reader: ?effects.StreamReader(A, E, R) = null,
        chunk: effects.Chunk(A) = .end,
        index: usize = 0,
        initializing: Init.RuntimeAdapter.Job = undefined,
        initialized_exit: outcomes.Exit(S, E) = undefined,
        opening: Open.RuntimeAdapter.Job = undefined,
        opened: outcomes.Exit(effects.StreamReader(A, E, R), E) = undefined,
        pulling: Pull.RuntimeAdapter.Job = undefined,
        pulled: outcomes.Exit(effects.Chunk(A), E) = undefined,
        stepping: Transition.RuntimeAdapter.Job = undefined,
        stepped: outcomes.Exit(Control, E) = undefined,
        finishing: Out.RuntimeAdapter.Job = undefined,
        finished: outcomes.Exit(Out.Success, E) = undefined,
        closing_parent: *Unit.RuntimeAdapter.Job = undefined,
        closing: Unit.RuntimeAdapter.Job = undefined,
        closed: outcomes.Exit(void, E) = undefined,
        const Acquire = effects.Effect(*Self, E, R);
        const Inputs = struct { source: Source, scope: *Scope };

        fn allocate(inputs: *const Inputs, env: *const R) outcomes.Exit(*Self, E) {
            const frame = env.allocator.create(Self) catch return .{ .die = outcomes.Defect.out_of_memory };
            frame.* = .{ .allocator = env.allocator, .scope = inputs.scope, .source = inputs.source };
            return .{ .success = frame };
        }
        fn use(frame: *Self, _: *const R) Out {
            return .{ .value = .{ .borrowed = frame }, .start = start };
        }
        fn start(active: *rt.State, parent: *Out.RuntimeAdapter.Job) rt.Step {
            const frame: *Self = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
            frame.parent = parent;
            frame.inherited = active.services;
            const described = if (comptime inference.isEffect(inference.unary(init, *const R, "fold.initialize"))) init(parent.env) else Init.sync(init);
            frame.initializing = .{ .program = described, .owned = described.ownership, .env = parent.env, .inherited = active.services, .sink = .{ .output = &frame.initialized_exit, .next_step = .{ .data = frame, .advance = initializedStep } } };
            return frame.initializing.adapterStep();
        }
        fn initializedStep(active: *rt.State, data: ?*anyopaque) rt.Step {
            const frame: *Self = @ptrCast(@alignCast(data.?));
            const value = frame.initialized_exit;
            if (value != .success) return frame.parent.adapterFinish(outcomes.castFailure(Out.Success, active.publish(value)));
            // Install ownership before publishing late cancellation, including
            // composite states whose fields have individual ownership evidence.
            frame.state = value.success;
            frame.initialized = true;
            if (active.cancelled()) return frame.parent.adapterFinish(.interrupted);
            const described = frame.source.open(frame.scope);
            frame.opening = .{ .program = described, .owned = described.ownership, .env = frame.parent.env, .inherited = frame.inherited, .sink = .{ .output = &frame.opened, .next_step = .{ .data = frame, .advance = openedStep } } };
            return frame.opening.adapterStep();
        }
        fn openedStep(active: *rt.State, data: ?*anyopaque) rt.Step {
            const frame: *Self = @ptrCast(@alignCast(data.?));
            const value = active.publish(frame.opened);
            if (value != .success) return frame.parent.adapterFinish(outcomes.castFailure(Out.Success, value));
            frame.reader = value.success;
            return .{ .data = frame, .advance = pullStep };
        }
        fn pullStep(active: *rt.State, data: ?*anyopaque) rt.Step {
            const frame: *Self = @ptrCast(@alignCast(data.?));
            if (active.cancelled()) return frame.parent.adapterFinish(.interrupted);
            const described = frame.reader.?.pull.borrow();
            frame.pulling = .{ .program = described, .env = frame.parent.env, .inherited = frame.inherited, .sink = .{ .output = &frame.pulled, .next_step = .{ .data = frame, .advance = pulledStep } } };
            return frame.pulling.adapterStep();
        }
        fn pulledStep(active: *rt.State, data: ?*anyopaque) rt.Step {
            const frame: *Self = @ptrCast(@alignCast(data.?));
            const value = active.publish(frame.pulled);
            if (value != .success) return frame.parent.adapterFinish(outcomes.castFailure(Out.Success, value));
            frame.chunk = value.success;
            if (frame.chunk == .end) return frame.finishState(active);
            frame.index = 0;
            return .{ .data = frame, .advance = consumeStep };
        }
        fn consumeStep(active: *rt.State, data: ?*anyopaque) rt.Step {
            const frame: *Self = @ptrCast(@alignCast(data.?));
            if (active.cancelled()) return frame.parent.adapterFinish(.interrupted);
            if (frame.index == frame.chunk.items.values.len) {
                frame.chunk.deinit();
                return .{ .data = frame, .advance = pullStep };
            }
            const value = &frame.chunk.items.values[frame.index];
            const described = if (comptime StepReturn == Control) Transition.succeed(step(&frame.state, value, frame.parent.env)) else step(&frame.state, value, frame.parent.env);
            frame.stepping = .{ .program = described, .owned = described.ownership, .env = frame.parent.env, .inherited = frame.inherited, .sink = .{ .output = &frame.stepped, .next_step = .{ .data = frame, .advance = steppedStep } } };
            return frame.stepping.adapterStep();
        }
        fn steppedStep(active: *rt.State, data: ?*anyopaque) rt.Step {
            const frame: *Self = @ptrCast(@alignCast(data.?));
            const value = active.publish(frame.stepped);
            if (value != .success) return frame.parent.adapterFinish(outcomes.castFailure(Out.Success, value));
            if (value.success == .stop) return frame.finishState(active);
            frame.index += 1;
            return .{ .data = frame, .advance = consumeStep };
        }
        fn finishState(frame: *Self, _: *rt.State) rt.Step {
            if (comptime isTransfer(@TypeOf(finish))) {
                const Selection = @TypeOf(finish);
                if (comptime @TypeOf(Selection.before) == @TypeOf(null)) return frame.transferResult();
                const before = Selection.before;
                if (comptime inference.result(before, &.{ *S, *const R }, "fold.finish.before") != Unit) @compileError("fold transfer before role requires exact Effect(void,E,R)");
                const described = before(&frame.state, frame.parent.env);
                frame.before_job = .{ .program = described, .owned = described.ownership, .env = frame.parent.env, .inherited = frame.inherited, .sink = .{ .output = &frame.before_exit, .next_step = .{ .data = frame, .advance = transferredStep } } };
                return frame.before_job.adapterStep();
            }
            const Finished = comptime inference.result(finish, &.{ *S, *const R }, "fold.finish");
            if (comptime !inference.isEffect(Finished)) return frame.parent.adapterFinish(.{ .success = finish(&frame.state, frame.parent.env) });
            const described = finish(&frame.state, frame.parent.env);
            frame.finishing = .{ .program = described, .owned = described.ownership, .env = frame.parent.env, .inherited = frame.inherited, .sink = .{ .output = &frame.finished, .next_step = .{ .data = frame, .advance = finishedStep } } };
            return frame.finishing.adapterStep();
        }
        fn transferredStep(active: *rt.State, data: ?*anyopaque) rt.Step {
            const frame: *Self = @ptrCast(@alignCast(data.?));
            const value = active.publish(frame.before_exit);
            if (value != .success) return frame.parent.adapterFinish(outcomes.castFailure(Out.Success, value));
            return frame.transferResult();
        }
        fn transferResult(frame: *Self) rt.Step {
            const Selection = @TypeOf(finish);
            if (comptime Selection.mode == .whole) {
                frame.initialized = false;
                return frame.parent.adapterFinish(.{ .success = ownership.move(frame.state) });
            }
            if (comptime outcomes.ownsPayload(S)) @compileError("fold field transfer requires a composite state with per-field ownership evidence; use whole-state transfer for an opaque owning state");
            if (comptime @hasField(roles.recordType(supplied), "cleanup") or @hasDecl(roles.recordType(supplied), "cleanup")) @compileError("fold field transfer uses generated cleanup of remaining fields");
            const names = @typeInfo(S).@"struct".field_names;
            inline for (names, 0..) |name, index| if (comptime std.mem.eql(u8, name, Selection.field)) {
                frame.state_fields[index] = false;
            };
            return frame.parent.adapterFinish(.{ .success = @field(frame.state, Selection.field) });
        }
        fn finishedStep(_: *rt.State, data: ?*anyopaque) rt.Step {
            const frame: *Self = @ptrCast(@alignCast(data.?));
            return frame.parent.adapterFinish(frame.finished);
        }
        fn cleanup(frame: *const Self, env: *const R) outcomes.Exit(void, E) {
            _ = env;
            const mutable = @constCast(frame);
            if (comptime @typeInfo(S) == .@"struct" and !outcomes.ownsPayload(S)) {
                const names = @typeInfo(S).@"struct".field_names;
                inline for (0..names.len) |i| {
                    const index = names.len - i - 1;
                    if (mutable.state_fields[index]) ownership.discard(&@field(mutable.state, names[index]));
                }
            } else ownership.discard(&mutable.state);
            return .{ .success = {} };
        }
        fn release(frame: *Self, _: outcomes.PrimaryOutcome(E), _: *const R) Unit {
            return .{ .value = .{ .borrowed = frame }, .start = releaseStart };
        }
        fn releaseStart(active: *rt.State, parent: *Unit.RuntimeAdapter.Job) rt.Step {
            const frame: *Self = @ptrCast(@alignCast(@constCast(parent.program.value.borrowed)));
            frame.closing_parent = parent;
            if (frame.reader) |*reader| reader.deinit();
            frame.chunk.deinit();
            if (!frame.initialized) {
                const allocator = frame.allocator;
                allocator.destroy(frame);
                return parent.adapterFinish(.{ .success = {} });
            }
            const described = if (comptime @hasField(roles.recordType(supplied), "cleanup") or @hasDecl(roles.recordType(supplied), "cleanup")) blk: {
                const clean = roles.get("cleanup", supplied);
                const Returned = comptime inference.result(clean, &.{ *S, *const R }, "fold.cleanup");
                if (comptime Returned != Unit) @compileError("fold.cleanup requires exact Effect(void,E,R)");
                break :blk clean(&frame.state, parent.env);
            } else Unit.fromBorrowed(Self, frame, cleanup);
            frame.closing = .{ .program = described, .owned = described.ownership, .env = parent.env, .inherited = active.services, .sink = .{ .output = &frame.closed, .next_step = .{ .data = frame, .advance = releasedStep } } };
            return frame.closing.adapterStep();
        }
        fn releasedStep(_: *rt.State, data: ?*anyopaque) rt.Step {
            const frame: *Self = @ptrCast(@alignCast(data.?));
            const parent = frame.closing_parent;
            const value = frame.closed;
            frame.allocator.destroy(frame);
            return parent.adapterFinish(value);
        }
        pub fn scopedSource(source: *const Source, scope: *Scope, env: *const R) Out {
            var acquire = Acquire.fromOwned(env, Inputs{ .source = source.*, .scope = scope }, allocate);
            return acquire.acquireReleaseOwned(env, use, release);
        }
    };
}

const TransferMode = enum { whole, field };
fn Transfer(comptime mode_value: TransferMode, comptime field_name: []const u8, comptime callback: anytype) type {
    return struct {
        pub const mode = mode_value;
        pub const field = field_name;
        pub const before = callback;
        pub const sink_transfer = true;
    };
}
fn isTransfer(comptime T: type) bool {
    return @typeInfo(T) == .@"struct" and @hasDecl(T, "sink_transfer") and T == Transfer(T.mode, T.field, T.before);
}
fn fieldCount(comptime T: type) usize {
    return if (@typeInfo(T) == .@"struct") @typeInfo(T).@"struct".field_names.len else 0;
}
/// Transfer the entire initialized state to the caller after successful folding.
pub fn takeState() Transfer(.whole, "", null) {
    return .{};
}
/// Suspend/evaluate before transferring state; a failed callback retains state
/// for cleanup, and the framework performs the move only after its success.
pub fn takeStateAfter(comptime before: anytype) Transfer(.whole, "", before) {
    return .{};
}
/// Move one composite field and generate cleanup for all remaining fields.
pub fn takeField(comptime name: []const u8) Transfer(.field, name, null) {
    return .{};
}
pub fn takeFieldAfter(comptime name: []const u8, comptime before: anytype) Transfer(.field, name, before) {
    return .{};
}
