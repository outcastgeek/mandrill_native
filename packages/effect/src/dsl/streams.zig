//! Inferred stream roles lower through the existing scoped pull reader.
const effects = @import("../effect.zig");
const scopes = @import("../scope.zig");
const roles = @import("roles.zig");
const inference = @import("inference.zig");
const ownership = @import("ownership.zig");
const outcomes = @import("../outcome.zig");
pub const append = @import("stream_plan.zig").append;
pub const unfold = @import("stream_plan.zig").unfold;
pub const loop = @import("stream_plan.zig").loop;
pub const bind = @import("stream_plan.zig").bind;
pub const branch = @import("stream_plan.zig").branch;
pub const yieldEffect = @import("stream_plan.zig").yieldEffect;

fn IteratorRoles(comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "stream.iterator", &.{ "open", "next", "release" }, &.{});
    const R = inference.environment(roles.get("open", supplied), "stream.iterator.open");
    const Acquire = inference.unary(roles.get("open", supplied), *const R, "stream.iterator.open");
    inference.requireEffect(Acquire, "stream.iterator.open");
    const Selected = inference.continuation(roles.get("next", supplied), Acquire.Success, R, "stream.iterator.next");
    if (!inference.isEffect(Selected) and !inference.isExit(Selected)) @compileError("stream.iterator.next requires exact Exit(?Item,E) or Effect(?Item,E,R)");
    if (@typeInfo(Selected.Success) != .optional) @compileError("stream.iterator.next requires an optional item");
    const A = @typeInfo(Selected.Success).optional.child;
    const Next = effects.Effect(?A, Acquire.Error, R);
    if (Selected.Error != Acquire.Error or (inference.isEffect(Selected) and Selected != Next)) @compileError("stream.iterator.next error/environment mismatch");
    const Pull = effects.Effect(effects.Chunk(A), Acquire.Error, R);
    return struct {
        pub const open = roles.get("open", supplied);
        pub const release = roles.get("release", supplied);
        fn chunk(value: ?A, current: *const R) outcomes.Exit(effects.Chunk(A), Acquire.Error) {
            if (value) |item| {
                const items = current.allocator.alloc(A, 1) catch {
                    var abandoned = item;
                    outcomes.discardPayload(&abandoned);
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
                items[0] = item;
                return .{ .success = effects.Chunk(A).owned(.{ .items = items, .allocator = current.allocator }) };
            }
            return .{ .success = .end };
        }
        pub fn pull(resource: Acquire.Success, env: *const R) Pull {
            if (comptime inference.isExit(Selected)) {
                const Native = struct {
                    fn read(value: Acquire.Success, current: *const R) outcomes.Exit(effects.Chunk(A), Acquire.Error) {
                        const result = inference.call(roles.get("next", supplied), value, current);
                        if (result != .success) return outcomes.castFailure(effects.Chunk(A), result);
                        return chunk(result.success, current);
                    }
                };
                if (comptime @typeInfo(Acquire.Success) == .pointer and @typeInfo(Acquire.Success).pointer.size == .one) {
                    const Pointer = @typeInfo(Acquire.Success).pointer;
                    return Pull.fromBorrowed(Pointer.child, resource, struct {
                        fn read(value: *const Pointer.child, current: *const R) outcomes.Exit(effects.Chunk(A), Acquire.Error) {
                            return Native.read(if (comptime Pointer.attrs.@"const") value else @constCast(value), current);
                        }
                    }.read);
                }
                return Pull.fromOwned(env, resource, struct {
                    fn read(value: *const Acquire.Success, current: *const R) outcomes.Exit(effects.Chunk(A), Acquire.Error) {
                        return Native.read(value.*, current);
                    }
                }.read);
            }
            var described = inference.call(roles.get("next", supplied), resource, env);
            return described.flatMapOwned(env, struct {
                fn completed(value: ?A, current: *const R) Pull {
                    return Pull.fromOwnedOutcome(current, chunk(value, current));
                }
            }.completed);
        }
    };
}

/// Infer an iterator's item from its optional next result and its context from
/// acquisition. Native Exit and Effect next roles remain explicit categories.
pub fn iterator(comptime supplied: anytype) Context(IteratorRoles(supplied), null).Stream {
    return from(IteratorRoles(supplied));
}

fn Context(comptime supplied: anytype, comptime P: ?type) type {
    roles.validate(roles.recordType(supplied), "stream", &.{ "open", "pull", "release" }, &.{});
    const open = roles.get("open", supplied);
    const R = inference.environment(open, "stream.open");
    const Acquire = if (P) |Pointer| inference.result(open, &.{ Pointer, *const R }, "stream.open") else inference.unary(open, *const R, "stream.open");
    inference.requireEffect(Acquire, "stream.open");
    if (Acquire.Environment != R) @compileError("stream.open environment mismatch; use explicit projection");
    const Pull = if (P) |Pointer| inference.result(roles.get("pull", supplied), &.{ Pointer, Acquire.Success, *const R }, "stream.pull") else inference.continuation(roles.get("pull", supplied), Acquire.Success, R, "stream.pull");
    inference.requireEffect(Pull, "stream.pull");
    const Chunk = Pull.Success;
    if (@typeInfo(Chunk) != .@"union" or !@hasDecl(Chunk, "Item") or Chunk != effects.Chunk(Chunk.Item)) @compileError("stream.pull requires exact Effect(Chunk(Item),E,R)");
    const Release = if (P) |Pointer| inference.result(roles.get("release", supplied), &.{ Pointer, Acquire.Success, *const R }, "stream.release") else inference.continuation(roles.get("release", supplied), Acquire.Success, R, "stream.release");
    if (Release != effects.Effect(void, Acquire.Error, R)) @compileError("stream.release requires exact Effect(void,E,R)");
    if (Pull.Error != Acquire.Error or Pull.Environment != R) @compileError("stream.pull error/environment mismatch; use explicit mapping/projection");
    return struct {
        pub const Acquisition = Acquire;
        pub const Stream = effects.Stream(Chunk.Item, Acquire.Error, R);
    };
}

// Store one borrowed resource handle with its configuration pointer. The
// factory runs on every demanded pull, rather than once while opening a reader.
fn deferredPull(env: anytype, capture: anytype, resource: anytype, comptime invoke: anytype) inference.result(invoke, &.{ *const @TypeOf(capture), @TypeOf(resource), @TypeOf(env) }, "stream.pull") {
    const C = @TypeOf(capture);
    const Resource = @TypeOf(resource);
    const Out = inference.result(invoke, &.{ *const C, Resource, @TypeOf(env) }, "stream.pull");
    const Inputs = struct { capture: C, resource: effects.ResourceValue(Resource) };
    return @import("../dsl.zig").owned(env, Inputs{ .capture = capture, .resource = .{ .value = resource } }, struct {
        fn next(input: *const Inputs, current: *const Out.Environment) Out {
            return invoke(&input.capture, input.resource.value, current);
        }
    }.next);
}

/// Construct a lazy stream from named Effect factory, pull, and release roles.
/// R/E/resource/item types come from the roles; no callback runs at construction.
pub fn from(comptime supplied: anytype) Context(supplied, null).Stream {
    const C = Context(supplied, null);
    const Acquire = C.Acquisition;
    const R = Acquire.Environment;
    const Resource = Acquire.Success;
    // Native pointer resources already have stable scope-owned storage. Reuse
    // the static fromPull authority without a second reader-opening owner.
    if (comptime @typeInfo(Resource) == .pointer and @typeInfo(Resource).pointer.size == .one and !@typeInfo(Resource).pointer.attrs.@"volatile") {
        const Pointer = @typeInfo(Resource).pointer;
        const Adapter = struct {
            fn original(pointer: *const Pointer.child) Resource {
                return @ptrCast(@alignCast(if (comptime Pointer.attrs.@"const") pointer else @constCast(pointer)));
            }
            fn pull(resource: Resource) effects.Effect(effects.Chunk(C.Stream.Item), Acquire.Error, R) {
                const Pull = effects.Effect(effects.Chunk(C.Stream.Item), Acquire.Error, R);
                return Pull.deferBorrowed(Pointer.child, resource, struct {
                    fn invoke(pointer: *const Pointer.child, env: *const R) Pull {
                        return inference.call(roles.get("pull", supplied), original(pointer), env);
                    }
                }.invoke);
            }
            fn release(resource: Resource) effects.Effect(void, Acquire.Error, R) {
                const Unit = effects.Effect(void, Acquire.Error, R);
                return Unit.deferBorrowed(Pointer.child, resource, struct {
                    fn invoke(pointer: *const Pointer.child, env: *const R) Unit {
                        return inference.call(roles.get("release", supplied), original(pointer), env);
                    }
                }.invoke);
            }
        };
        return C.Stream.fromPull(comptime Acquire.deferEffect(roles.get("open", supplied)), Adapter.pull, Adapter.release);
    }
    return .{ .open_fn = struct {
        fn pull(resource: Acquire.Success, env: *const R) effects.Effect(effects.Chunk(C.Stream.Item), Acquire.Error, R) {
            return deferredPull(env, @as(void, {}), resource, struct {
                fn invoke(_: *const void, value: Acquire.Success, current: *const R) effects.Effect(effects.Chunk(C.Stream.Item), Acquire.Error, R) {
                    return inference.call(roles.get("pull", supplied), value, current);
                }
            }.invoke);
        }
        fn release(resource: Acquire.Success, env: *const R) effects.Effect(void, Acquire.Error, R) {
            return inference.call(roles.get("release", supplied), resource, env);
        }
        fn open(scope: *scopes.Scope(Acquire.Error)) effects.Effect(effects.StreamReader(C.Stream.Item, Acquire.Error, R), Acquire.Error, R) {
            const described = comptime Acquire.deferEffect(roles.get("open", supplied));
            var acquired = described.scopeAcquire(scope, release);
            return C.Stream.openAcquired(&acquired, scope.allocator, pull);
        }
    }.open };
}

/// Borrow original typed runtime configuration. It must outlive opening,
/// pending pulls, and scope close. Mutable pointer qualifiers are preserved.
pub fn borrowed(capture: anytype, comptime supplied: anytype) Context(supplied, @TypeOf(capture)).Stream {
    const P = @TypeOf(capture);
    if (@typeInfo(P) != .pointer or @typeInfo(P).pointer.size != .one) @compileError("stream.borrowed requires a single-item capture pointer");
    const C = Context(supplied, P);
    const Acquire = C.Acquisition;
    const R = Acquire.Environment;
    const E = Acquire.Error;
    const Adapter = struct {
        fn acquire(pointer: *const P, env: *const R) Acquire {
            return roles.get("open", supplied)(pointer.*, env);
        }
        fn release(pointer: *const P, resource: Acquire.Success, env: *const R) effects.Effect(void, E, R) {
            return roles.get("release", supplied)(pointer.*, resource, env);
        }
        fn pull(pointer: *const P, resource: Acquire.Success, env: *const R) effects.Effect(effects.Chunk(C.Stream.Item), E, R) {
            return deferredPull(env, pointer.*, resource, struct {
                fn invoke(configuration: *const P, value: Acquire.Success, current: *const R) effects.Effect(effects.Chunk(C.Stream.Item), E, R) {
                    return roles.get("pull", supplied)(configuration.*, value, current);
                }
            }.invoke);
        }
        fn open(raw: *const anyopaque, scope: *scopes.Scope(E)) effects.Effect(effects.StreamReader(C.Stream.Item, E, R), E, R) {
            const pointer: P = @ptrCast(@alignCast(@constCast(raw)));
            var acquired = Acquire.scopeAcquireFactoryCapturedAllocated(scope.allocator, scope, pointer, acquire, release);
            return C.Stream.openAcquiredCaptured(&acquired, scope.allocator, pointer, pull);
        }
    };
    return .{ .captured = capture, .captured_open_fn = Adapter.open };
}

fn Lifecycle(comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "stream.scoped", &.{ "open", "pull", "release", "body" }, &.{});
    return struct {
        pub const open = roles.get("open", supplied);
        pub const pull = roles.get("pull", supplied);
        pub const release = roles.get("release", supplied);
    };
}
fn ScopedResult(comptime T: type, comptime supplied: anytype) type {
    const C = Context(Lifecycle(supplied), *const T);
    const Out = inference.result(roles.get("body", supplied), &.{ C.Stream, *const C.Acquisition.Environment }, "stream.scoped.body");
    inference.requireEffect(Out, "stream.scoped.body");
    if (Out.Error != C.Acquisition.Error or Out.Environment != C.Acquisition.Environment) @compileError("stream.scoped.body error/environment mismatch");
    return Out;
}

/// Move configuration once into generated storage and retain it through the
/// body's reader scopes, pending work, and protected release. Every execution
/// opens fresh resources; the configuration itself need not support Clone.
pub fn scoped(env: anytype, capture: anytype, comptime supplied: anytype) ScopedResult(@TypeOf(capture), supplied) {
    const T = @TypeOf(capture);
    const Out = ScopedResult(T, supplied);
    const Stored = ownership.Capture(T);
    const R = Out.Environment;
    return Out.scopedCaptured(env, Stored{ .value = capture }, struct {
        fn body(stored: *const Stored, _: *scopes.Scope(Out.Error), current: *const R) Out {
            const source = borrowed(&stored.value, Lifecycle(supplied));
            return roles.get("body", supplied)(source, current);
        }
    }.body);
}

fn ConsumeResult(comptime Source: type, comptime supplied: anytype) type {
    @import("stream_plan.zig").requireStream(Source);
    roles.validate(roles.recordType(supplied), "stream.consume", &.{"consume"}, &.{});
    const Out = inference.result(roles.get("consume", supplied), &.{ *const Source.Item, *const Source.Environment }, "stream.consume");
    if (Out != effects.Effect(void, Source.Error, Source.Environment)) @compileError("stream.consume requires exact Effect(void,E,R)");
    return Out;
}

/// Consume a runtime stream through the same result-producing fold frame.
pub fn consume(env: anytype, source: anytype, comptime supplied: anytype) ConsumeResult(@TypeOf(source), supplied) {
    const Source = @TypeOf(source);
    const R = Source.Environment;
    const Unit = ConsumeResult(Source, supplied);
    const Control = @import("../sink_builder.zig").Control;
    const Transition = effects.Effect(Control, Source.Error, R);
    const Adapter = struct {
        fn initialize(_: *const R) void {}
        fn step(_: *void, item: *const Source.Item, current: *const R) Transition {
            var described = roles.get("consume", supplied)(item, current);
            return described.mapOwned(current, struct {
                fn proceed(_: void) Control {
                    return .proceed;
                }
            }.proceed);
        }
        fn finish(_: *void, _: *const R) void {}
    };
    _ = Unit;
    const sink = comptime @import("../sink_builder.zig").fold(.{ .initialize = Adapter.initialize, .step = Adapter.step, .finish = Adapter.finish });
    return sink.runWith(env, source);
}

fn Mapper(comptime Source: type, comptime T: type, comptime supplied: anytype) type {
    @import("stream_plan.zig").requireStream(Source);
    roles.validate(roles.recordType(supplied), "stream.map", &.{ "map", "body" }, &.{});
    const Result = inference.result(roles.get("map", supplied), &.{ *const T, Source.Item, *const Source.Environment }, "stream.map");
    const Out = if (inference.isEffect(Result)) Result else effects.Effect(Result, Source.Error, Source.Environment);
    if (Out.Error != Source.Error or Out.Environment != Source.Environment) @compileError("stream.map error/environment mismatch");
    return Out;
}
fn MapResult(comptime Source: type, comptime T: type, comptime supplied: anytype) type {
    const Mapped = Mapper(Source, T, supplied);
    const Out = inference.result(roles.get("body", supplied), &.{ effects.Stream(Mapped.Success, Source.Error, Source.Environment), *const Source.Environment }, "stream.map.body");
    inference.requireEffect(Out, "stream.map.body");
    if (Out.Error != Source.Error or Out.Environment != Source.Environment) @compileError("stream.map.body error/environment mismatch");
    return Out;
}

/// Capture mapping inputs once and retain them through suspended transforms,
/// consumption, and close. Pure and Effect-producing mappers infer their result.
pub fn map(env: anytype, source: anytype, capture: anytype, comptime supplied: anytype) MapResult(@TypeOf(source), @TypeOf(capture), supplied) {
    const Source = @TypeOf(source);
    const T = @TypeOf(capture);
    const Transform = Mapper(Source, T, supplied);
    const Out = MapResult(Source, T, supplied);
    const R = Source.Environment;
    const E = Source.Error;
    const Mapped = effects.Stream(Transform.Success, E, R);
    const Stored = struct {
        pub const effect_owned_payload = true;
        source: Source,
        inputs: ownership.Capture(T),
        pub fn deinit(self: *@This()) void {
            self.inputs.deinit();
        }
    };
    const Adapter = struct {
        fn transform(pointer: *const *const T, item: Source.Item, current: *const R) Transform {
            const Result = comptime inference.result(roles.get("map", supplied), &.{ *const T, Source.Item, *const R }, "stream.map");
            if (comptime inference.isEffect(Result)) return roles.get("map", supplied)(pointer.*, item, current);
            const result = roles.get("map", supplied)(pointer.*, item, current);
            return if (comptime @import("../outcome.zig").ownsPayload(Result)) Transform.fromOwnedSuccess(current, result) else Transform.succeed(result);
        }
        fn open(stored: *const Stored, scope: *scopes.Scope(E)) effects.Effect(effects.StreamReader(Transform.Success, E, R), E, R) {
            return stored.source.openMappedCaptured(scope, &stored.inputs.value, transform);
        }
        fn body(stored: *const Stored, _: *scopes.Scope(E), current: *const R) Out {
            const mapped = Mapped.fromCaptured(Stored, stored, open);
            return roles.get("body", supplied)(mapped, current);
        }
    };
    return Out.scopedCaptured(env, Stored{ .source = source, .inputs = .{ .value = capture } }, Adapter.body);
}

const Adaptation = enum { map_error, project };
fn AdaptedStream(comptime mode: Adaptation, comptime Source: type, comptime T: type, comptime supplied: anytype) type {
    @import("stream_plan.zig").requireStream(Source);
    const role = if (mode == .map_error) "map_error" else "project";
    roles.validate(roles.recordType(supplied), "stream." ++ role, &.{ role, "body" }, &.{});
    const callback = roles.get(role, supplied);
    if (mode == .map_error) {
        const F = inference.result(callback, &.{ *const T, Source.Error }, "stream.map_error");
        if (inference.isEffect(F) or inference.isExit(F)) @compileError("stream.map_error requires a native error value");
        return effects.Stream(Source.Item, F, Source.Environment);
    }
    const Large = inference.environment(callback, "stream.project");
    if (inference.result(callback, &.{ *const T, *const Large }, "stream.project") != *const Source.Environment) @compileError("stream.project requires a pointer to the source environment");
    return effects.Stream(Source.Item, Source.Error, Large);
}
fn AdaptedResult(comptime mode: Adaptation, comptime Source: type, comptime T: type, comptime supplied: anytype) type {
    const Mapped = AdaptedStream(mode, Source, T, supplied);
    const Out = inference.result(roles.get("body", supplied), &.{ Mapped, *const Mapped.Environment }, "stream.adaptation.body");
    inference.requireEffect(Out, "stream.adaptation.body");
    if (Out.Error != Mapped.Error or Out.Environment != Mapped.Environment) @compileError("stream.adaptation.body error/environment mismatch");
    return Out;
}
fn adapt(comptime mode: Adaptation, env: anytype, source: anytype, capture: anytype, comptime supplied: anytype) AdaptedResult(mode, @TypeOf(source), @TypeOf(capture), supplied) {
    const Source = @TypeOf(source);
    const T = @TypeOf(capture);
    const Mapped = AdaptedStream(mode, Source, T, supplied);
    const Out = AdaptedResult(mode, Source, T, supplied);
    const Stored = struct {
        pub const effect_owned_payload = true;
        source: Source,
        inputs: ownership.Capture(T),
        pub fn deinit(self: *@This()) void {
            self.inputs.deinit();
        }
    };
    const Adapter = struct {
        fn mapFailure(pointer: *const *const T, err: Source.Error) Mapped.Error {
            return roles.get("map_error", supplied)(pointer.*, err);
        }
        fn open(stored: *const Stored, scope: *scopes.Scope(Mapped.Error)) effects.Effect(effects.StreamReader(Mapped.Item, Mapped.Error, Mapped.Environment), Mapped.Error, Mapped.Environment) {
            return if (comptime mode == .map_error)
                stored.source.openErrorMappedCaptured(scope, &stored.inputs.value, mapFailure)
            else
                stored.source.openProjectedCaptured(scope, &stored.inputs.value, roles.get("project", supplied));
        }
        fn body(stored: *const Stored, _: *scopes.Scope(Mapped.Error), current: *const Mapped.Environment) Out {
            return roles.get("body", supplied)(Mapped.fromCaptured(Stored, stored, open), current);
        }
    };
    return Out.scopedCaptured(env, Stored{ .source = source, .inputs = .{ .value = capture } }, Adapter.body);
}
pub fn mapError(env: anytype, source: anytype, capture: anytype, comptime supplied: anytype) AdaptedResult(.map_error, @TypeOf(source), @TypeOf(capture), supplied) {
    return adapt(.map_error, env, source, capture, supplied);
}
pub fn project(env: anytype, source: anytype, capture: anytype, comptime supplied: anytype) AdaptedResult(.project, @TypeOf(source), @TypeOf(capture), supplied) {
    return adapt(.project, env, source, capture, supplied);
}
