//! Inferred Ref operations use its existing mutex, state and scope authority.
const std = @import("std");
const effects = @import("../effect.zig");
const primitives = @import("../primitives.zig");
const scopes = @import("../scope.zig");
const outcomes = @import("../outcome.zig");
const asynchronous = @import("../async.zig");
const rt = @import("../internal/runtime.zig");
const inference = @import("inference.zig");
const roles = @import("roles.zig");
const ownership = @import("ownership.zig");

fn StateOf(comptime P: type) type {
    if (@typeInfo(P) != .pointer or @typeInfo(P).pointer.size != .one or @typeInfo(P).pointer.attrs.@"const") @compileError("Ref operation requires a mutable borrowed Ref pointer");
    const Core = @typeInfo(P).pointer.child;
    if (@typeInfo(Core) != .@"struct" or !@hasField(Core, "value")) @compileError("Ref operation requires the existing Ref authority");
    const A = @FieldType(Core, "value");
    if (ownership.containsOwned(A)) @compileError("Ref state must contain borrowed handles or ordinary copyable values");
    if (Core != primitives.Ref(A)) @compileError("Ref operation rejects forged Ref evidence");
    return A;
}

fn ScopeError(comptime P: type) type {
    if (@typeInfo(P) != .pointer or @typeInfo(P).pointer.size != .one or @typeInfo(P).pointer.attrs.@"const") @compileError("Ref.make requires a mutable Scope owner");
    const Scope = @typeInfo(P).pointer.child;
    if (@typeInfo(Scope) != .@"struct" or !@hasField(Scope, "result")) @compileError("Ref.make requires the existing Scope authority");
    const Exit = @FieldType(Scope, "result");
    if (!inference.isExit(Exit) or Scope != scopes.Scope(Exit.Error)) @compileError("Ref.make rejects forged Scope evidence");
    return Exit.Error;
}

/// A native result/replacement record. Generated ownership covers result
/// members; the synchronized replacement remains copyable Ref state.
pub fn Change(comptime A: type, comptime B: type) type {
    if (ownership.containsOwned(A)) @compileError("Ref replacement must be copyable state");
    return struct {
        pub const effect_owned_payload = ownership.containsOwned(B);
        result: B,
        value: A,
        pub fn deinit(self: *@This()) void {
            ownership.discard(&self.result);
        }
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            return .{ .result = try ownership.cloneValue(self.result), .value = self.value };
        }
    };
}
pub fn change(result: anytype, value: anytype) Change(@TypeOf(value), @TypeOf(result)) {
    return .{ .result = result, .value = value };
}

fn Result(comptime C: type, comptime A: type) type {
    if (inference.isEffect(C) or inference.isExit(C) or @typeInfo(C) != .@"struct" or !@hasField(C, "result") or !@hasField(C, "value")) @compileError("Ref transition requires a native result/value record; use modifyEffect for an Effect result");
    if (@typeInfo(C).@"struct".field_names.len != 2 or @FieldType(C, "value") != A) @compileError("Ref transition must return result and a replacement matching its state type");
    const B = @FieldType(C, "result");
    ownership.validate(B);
    if (outcomes.ownsPayload(C) and C != Change(A, B)) @compileError("Ref transition owning records require the generated RefChange contract");
    return B;
}

fn InitialState(comptime T: type, comptime E: type, comptime R: type, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "Ref.make", &.{"initialize"}, &.{});
    const Initial = inference.result(roles.get("initialize", supplied), &.{ *const T, *const R }, "Ref.initialize");
    const effectful = inference.isEffect(Initial);
    const outcome = inference.isExit(Initial);
    const A = if (effectful or outcome) Initial.Success else Initial;
    if (@typeInfo(A) == .error_union) @compileError("Ref.initialize native errors need an explicit Effect mapper");
    if (effectful and (Initial.Error != E or Initial.Environment != R)) @compileError("Ref.initialize Effect must match scope E and current R");
    if (outcome and Initial.Error != E) @compileError("Ref.initialize Exit must match scope E");
    if (ownership.containsOwned(A)) @compileError("Ref state must contain borrowed handles or ordinary copyable values");
    return A;
}

pub fn make(env: anytype, scope: anytype, capture: anytype, comptime supplied: anytype) effects.Effect(*primitives.Ref(InitialState(@TypeOf(capture), ScopeError(@TypeOf(scope)), @typeInfo(@TypeOf(env)).pointer.child, supplied)), ScopeError(@TypeOf(scope)), @typeInfo(@TypeOf(env)).pointer.child) {
    const T = @TypeOf(capture);
    const E = ScopeError(@TypeOf(scope));
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    const A = InitialState(T, E, R, supplied);
    const Core = primitives.Ref(A);
    const Acquire = effects.Effect(*Core, E, R);
    const Initial = inference.result(roles.get("initialize", supplied), &.{ *const T, *const R }, "Ref.initialize");
    const Adapter = struct {
        fn allocate(current: *const R, value: A) outcomes.Exit(*Core, E) {
            const core = current.allocator.create(Core) catch return .{ .die = outcomes.Defect.out_of_memory };
            core.* = Core.init(current, value);
            return .{ .success = core };
        }
        fn acquire(stored: *const T, current: *const R) Acquire {
            if (comptime inference.isEffect(Initial)) {
                var initialized = roles.get("initialize", supplied)(stored, current);
                return initialized.flatMapOwned(current, struct {
                    fn next(value: A, execution_env: *const R) Acquire {
                        return Acquire.fromOwned(execution_env, value, struct {
                            fn create(snapshot: *const A, selected: *const R) outcomes.Exit(*Core, E) {
                                return allocate(selected, snapshot.*);
                            }
                        }.create);
                    }
                }.next);
            } else return Acquire.fromBorrowed(T, stored, struct {
                fn create(inputs: *const T, selected: *const R) outcomes.Exit(*Core, E) {
                    const initialized = roles.get("initialize", supplied)(inputs, selected);
                    if (comptime inference.isExit(Initial)) {
                        if (initialized != .success) return outcomes.castFailure(*Core, initialized);
                        return allocate(selected, initialized.success);
                    } else return allocate(selected, initialized);
                }
            }.create);
        }
        fn release(_: *const T, core: *Core, _: *const R) effects.Effect(void, E, R) {
            return effects.Effect(void, E, R).fromBorrowed(Core, core, struct {
                fn dispose(value: *const Core, selected: *const R) outcomes.Exit(void, E) {
                    selected.allocator.destroy(@constCast(value));
                    return .{ .success = {} };
                }
            }.dispose);
        }
    };
    return @import("resources.zig").acquire(env, scope, capture, .{ .acquire = Adapter.acquire, .release = Adapter.release });
}

pub fn get(comptime E: type, comptime R: type, reference: anytype) effects.Effect(StateOf(@TypeOf(reference)), E, R) {
    return effects.EffectRef(StateOf(@TypeOf(reference)), E, R).get(reference);
}

fn NativeChange(comptime A: type, comptime R: type, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "Ref.modify", &.{"run"}, &.{});
    const C = inference.continuation(roles.get("run", supplied), A, R, "Ref.modify.run");
    _ = Result(C, A);
    return C;
}
pub fn modify(comptime E: type, comptime R: type, reference: anytype, comptime supplied: anytype) effects.Effect(ownership.Moved(Result(NativeChange(StateOf(@TypeOf(reference)), R, supplied), StateOf(@TypeOf(reference)))), E, R) {
    const A = StateOf(@TypeOf(reference));
    const Core = primitives.Ref(A);
    const C = NativeChange(A, R, supplied);
    const Out = effects.Effect(ownership.Moved(Result(C, A)), E, R);
    return Out.fromBorrowed(Core, reference, struct {
        fn update(_: *const void, value: A, current: *const R) C {
            return inference.call(roles.get("run", supplied), value, current);
        }
        fn run(value: *const Core, current: *const R) outcomes.Exit(Out.Success, E) {
            return .{ .success = ownership.move(@constCast(value).modifyContext(@as(*const void, &{}), current, update)) };
        }
    }.run);
}

fn CapturedChange(comptime A: type, comptime T: type, comptime R: type, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "Ref.modifyCaptured", &.{"run"}, &.{});
    const C = inference.result(roles.get("run", supplied), &.{ *const T, A, *const R }, "Ref.modifyCaptured.run");
    _ = Result(C, A);
    return C;
}
pub fn modifyCaptured(comptime E: type, env: anytype, reference: anytype, capture: anytype, comptime supplied: anytype) effects.Effect(ownership.Moved(Result(CapturedChange(StateOf(@TypeOf(reference)), @TypeOf(capture), @typeInfo(@TypeOf(env)).pointer.child, supplied), StateOf(@TypeOf(reference)))), E, @typeInfo(@TypeOf(env)).pointer.child) {
    const T = @TypeOf(capture);
    const A = StateOf(@TypeOf(reference));
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    const C = CapturedChange(A, T, R, supplied);
    const Out = effects.Effect(ownership.Moved(Result(C, A)), E, R);
    const Stored = struct { reference: @TypeOf(reference), inputs: T };
    return @import("../dsl.zig").owned(env, Stored{ .reference = reference, .inputs = capture }, struct {
        fn body(stored: *const Stored, _: *const R) Out {
            return Out.fromBorrowed(Stored, stored, struct {
                fn run(value: *const Stored, current: *const R) outcomes.Exit(Out.Success, E) {
                    const next = value.reference.modifyContext(&value.inputs, current, roles.get("run", supplied));
                    return .{ .success = ownership.move(next) };
                }
            }.run);
        }
    }.body);
}

fn EffectChange(comptime A: type, comptime T: type, comptime R: type, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "Ref.modifyEffect", &.{"run"}, &.{});
    const P = inference.result(roles.get("run", supplied), &.{ *const T, A, *const R }, "Ref.modifyEffect.run");
    inference.requireEffect(P, "Ref.modifyEffect.run");
    if (P.Environment != R) @compileError("Ref.modifyEffect environment mismatch; use an explicit projection");
    const B = Result(P.Success, A);
    if (ownership.containsOwned(B) and P.Success != Change(A, B)) @compileError("Ref.modifyEffect owning transition results require canonical RefChange(A,B) before composition");
    return P;
}
pub fn modifyEffect(env: anytype, reference: anytype, capture: anytype, comptime supplied: anytype) effects.Effect(ownership.Moved(Result(EffectChange(StateOf(@TypeOf(reference)), @TypeOf(capture), @typeInfo(@TypeOf(env)).pointer.child, supplied).Success, StateOf(@TypeOf(reference)))), EffectChange(StateOf(@TypeOf(reference)), @TypeOf(capture), @typeInfo(@TypeOf(env)).pointer.child, supplied).Error, @typeInfo(@TypeOf(env)).pointer.child) {
    const T = @TypeOf(capture);
    const A = StateOf(@TypeOf(reference));
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    const Core = primitives.Ref(A);
    const Body = EffectChange(A, T, R, supplied);
    const C = Body.Success;
    const E = Body.Error;
    const Out = effects.Effect(ownership.Moved(Result(C, A)), E, R);
    const Acquire = effects.Effect(void, E, R);
    const Stored = struct { reference: *Core, inputs: T };
    const Adapter = struct {
        const NativeEnv = struct { allocator: std.mem.Allocator, io: std.Io };
        const Frame = struct {
            allocator: std.mem.Allocator,
            parent: *Out.RuntimeAdapter.Job,
            stored: *const Stored,
            native_env: NativeEnv,
            acquiring: Acquire.RuntimeAdapter.Job = undefined,
            acquired: outcomes.Exit(void, E) = undefined,
            locked: bool = false,
            child: Body.RuntimeAdapter.Job = undefined,
            result: outcomes.Exit(C, E) = undefined,
            fn acquiredStep(state: *rt.State, data: ?*anyopaque) rt.Step {
                const frame: *@This() = @ptrCast(@alignCast(data.?));
                if (frame.acquired != .success) return frame.finish(outcomes.castFailure(Out.Success, frame.acquired));
                frame.locked = true;
                if (state.cancelled()) return frame.finish(.interrupted);
                state.committed_delivery = false;
                const program = roles.get("run", supplied)(&frame.stored.inputs, frame.stored.reference.value, frame.parent.env);
                frame.child = .{ .program = program, .owned = program.ownership, .inherited = state.services, .env = frame.parent.env, .sink = .{ .output = &frame.result, .next_step = .{ .data = frame, .advance = complete } } };
                return frame.child.adapterStep();
            }
            fn complete(state: *rt.State, data: ?*anyopaque) rt.Step {
                const frame: *@This() = @ptrCast(@alignCast(data.?));
                state.committed_delivery = false;
                if (frame.result != .success) return frame.finish(outcomes.castFailure(Out.Success, frame.result));
                var next = frame.result.success;
                if (state.cancelled()) {
                    ownership.discard(&next.result);
                    return frame.finish(.interrupted);
                }
                frame.stored.reference.value = next.value;
                return frame.finish(.{ .success = ownership.move(next.result) });
            }
            fn finish(frame: *@This(), result: outcomes.Exit(Out.Success, E)) rt.Step {
                if (frame.locked) frame.stored.reference.mutex.unlock(frame.stored.reference.io);
                const parent = frame.parent;
                frame.allocator.destroy(frame);
                return parent.adapterFinish(result);
            }
        };
        fn start(state: *rt.State, parent: *Out.RuntimeAdapter.Job) rt.Step {
            const stored: *const Stored = @ptrCast(@alignCast(parent.program.value.borrowed));
            const allocator = state.services.allocatorValue() orelse return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
            const frame = allocator.create(Frame) catch return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
            frame.* = .{ .allocator = allocator, .parent = parent, .stored = stored, .native_env = .{ .allocator = allocator, .io = stored.reference.io } };
            const acquire = Acquire.fromAsyncBorrowed(Frame, frame, struct {
                fn start(value: *const Frame, _: *const R) asynchronous.Async(void, E).Start {
                    return asynchronous.Async(void, E).nativeCancellable(&value.native_env, value.stored.reference, struct {
                        fn lock(core: *Core, _: *const NativeEnv, token: ?*const rt.Cancellation) outcomes.Exit(void, E) {
                            if (token) |cancellation| if (cancellation.isRequested()) return .interrupted;
                            core.mutex.lock(core.io) catch return .interrupted;
                            return .{ .success = {} };
                        }
                    }.lock);
                }
            }.start);
            frame.acquiring = .{ .program = acquire, .inherited = state.services, .env = parent.env, .sink = .{ .output = &frame.acquired, .next_step = .{ .data = frame, .advance = Frame.acquiredStep } } };
            return frame.acquiring.adapterStep();
        }
        fn body(stored: *const Stored, _: *const R) Out {
            return .{ .value = .{ .borrowed = stored }, .start = start };
        }
    };
    return @import("../dsl.zig").owned(env, Stored{ .reference = reference, .inputs = capture }, Adapter.body);
}
