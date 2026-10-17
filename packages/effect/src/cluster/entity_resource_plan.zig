//! Activation descriptions compose the existing EntityResource brackets. Their
//! sole effectful boundary is an ordinary Effect; providers retain fencing and
//! delivery authority. No activation resource is serialized by this module.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const capabilities = @import("cluster_capabilities.zig");
const resources = @import("entity_resource.zig");
const deadlines = @import("../deadline.zig");
const scopes = @import("../scope.zig");
const inference = @import("../dsl/inference.zig");
const ownership = @import("../dsl/ownership.zig");
const roles = @import("../dsl/roles.zig");

pub const Error = enum { invalid_token, stale, unavailable };
pub const Token = capabilities.Token;
pub const Fence = capabilities.Fence;

fn require(comptime T: type) void {
    if (@typeInfo(T) != .@"struct" or !@hasDecl(T, "activation_shape")) @compileError("activation composition requires a generated activation plan, not an ordinary Effect or foreign domain");
    const S = T.activation_shape;
    const Canonical = switch (S.kind) {
        .resource => Acquisition(S.selector),
        .use => Use(S.capture, S.operation),
        .bind => CapturedBound(S.source, S.capture, S.operation),
        .branch => Branch(S.predicate, S.left, S.right),
        else => @compileError("activation composition requires a known generated plan constructor"),
    };
    if (T != Canonical) @compileError("activation composition rejects structural lookalikes and copied markers");
}
fn Provider(comptime select: anytype) type {
    const R = inference.environment(select, "activation.acquire");
    const P = inference.unary(select, *const R, "activation.acquire");
    if (@typeInfo(P) != .pointer or @typeInfo(P).pointer.size != .one) @compileError("activation.acquire requires a pointer to EntityResource");
    const Service = @typeInfo(P).pointer.child;
    if (@typeInfo(Service) != .@"struct" or !@hasDecl(Service, "AcquireEffect")) @compileError("activation.acquire requires the existing EntityResource provider");
    const Acquire = Service.AcquireEffect;
    inference.requireEffect(Acquire, "activation.acquire provider");
    if (Service != resources.EntityResource(Acquire.Success, Acquire.Error, R)) @compileError("activation.acquire rejects a foreign resource/provider/environment");
    return Service;
}
pub fn acquire(comptime select: anytype) Acquisition(select) {
    return .{};
}
fn Acquisition(comptime select: anytype) type {
    const Service = Provider(select);
    const Acquire = Service.AcquireEffect;
    const A = Acquire.Success;
    const E = Acquire.Error;
    const R = Acquire.Environment;
    return struct {
        pub const Success = effects.ResourceValue(A);
        pub const Error = E;
        pub const Environment = R;
        pub const activation_shape = struct {
            pub const kind = .resource;
            pub const selector = select;
        };
        fn lower(_: *const @This(), context: anytype, env: *const R, tail: anytype) @TypeOf(tail).Result {
            const C = @TypeOf(context);
            const Tail = @TypeOf(tail);
            const Out = Tail.Result;
            const Unit = effects.Effect(void, E, R);
            const Captured = struct { context: C, tail: Tail, provider: *const Service };
            return Acquire.acquireReleaseFactoryCapturedOwned(env, Captured{ .context = context, .tail = tail, .provider = select(env) }, struct {
                fn start(stored: *const Captured, current: *const R) Acquire {
                    const valid = stored.context.check(current);
                    if (valid != .success) return Acquire.fromOwnedOutcome(current, outcomes.castFailure(A, valid));
                    return stored.provider.vtable.acquire(stored.provider.userdata, current, stored.context.token);
                }
            }.start, struct {
                fn use(stored: *const Captured, value: A, current: *const R) Out {
                    return stored.tail.call(Success{ .value = value }, stored.context, current);
                }
            }.use, struct {
                fn release(stored: *const Captured, value: A, primary: outcomes.PrimaryOutcome(E), current: *const R) Unit {
                    return stored.provider.vtable.release(stored.provider.userdata, current, value, primary);
                }
            }.release);
        }
    };
}

fn UseEffect(comptime C: type, comptime callback: anytype) type {
    const R = inference.environment(callback, "activation.use");
    const Out = inference.result(callback, &.{ *const C, capabilities.Token, *const R }, "activation.use");
    inference.requireEffect(Out, "activation.use");
    if (Out.Environment != R) @compileError("activation.use environment mismatch");
    return Out;
}
/// The callback receives application inputs and the activation's actual token.
/// Fenced providers remain responsible for atomic validation at commit time.
pub fn use(capture: anytype, comptime callback: anytype) Use(@TypeOf(capture), callback) {
    return .{ .inputs = capture };
}
fn Use(comptime C: type, comptime callback: anytype) type {
    const Effect = UseEffect(C, callback);
    const E = Effect.Error;
    const R = Effect.Environment;
    return struct {
        pub const Success = Effect.Success;
        pub const Error = E;
        pub const Environment = R;
        pub const activation_shape = struct {
            pub const kind = .use;
            pub const capture = C;
            pub const operation = callback;
        };
        inputs: C,
        fn lower(self: *const @This(), context: anytype, env: *const R, tail: anytype) @TypeOf(tail).Result {
            const Tail = @TypeOf(tail);
            const Out = Tail.Result;
            const valid = context.check(env);
            if (valid != .success) return Out.fromOwnedOutcome(env, outcomes.castFailure(Out.Success, valid));
            var described = callback(&self.inputs, context.token, env);
            const Captured = struct { context: @TypeOf(context), tail: Tail };
            return described.flatMapCapturedOwned(env, Out.Success, Captured{ .context = context, .tail = tail }, struct {
                fn next(stored: *const Captured, value: Success, current: *const R) Out {
                    return stored.tail.call(value, stored.context, current);
                }
            }.next);
        }
    };
}

pub fn bind(source: anytype, comptime callback: anytype) Bound(@TypeOf(source), callback) {
    return .{ .source = source, .inputs = {} };
}
fn Bound(comptime S: type, comptime callback: anytype) type {
    require(S);
    const N = inference.continuation(callback, S.Success, S.Environment, "activation.bind");
    return CapturedBound(S, void, struct {
        fn invoke(_: *const void, value: S.Success, env: *const S.Environment) N {
            return inference.call(callback, value, env);
        }
    }.invoke);
}
pub fn bindCaptured(source: anytype, capture: anytype, comptime callback: anytype) CapturedBound(@TypeOf(source), @TypeOf(capture), callback) {
    return .{ .source = source, .inputs = capture };
}
fn CapturedBound(comptime S: type, comptime Capture: type, comptime callback: anytype) type {
    require(S);
    const N = inference.result(callback, &.{ *const Capture, S.Success, *const S.Environment }, "activation.bind");
    require(N);
    if (S.Error != N.Error or S.Environment != N.Environment) @compileError("activation.bind error/environment mismatch");
    const R = S.Environment;
    return struct {
        pub const Success = N.Success;
        pub const Error = S.Error;
        pub const Environment = R;
        pub const activation_shape = struct {
            pub const kind = .bind;
            pub const source = S;
            pub const operation = callback;
            pub const capture = Capture;
        };
        source: S,
        inputs: Capture,
        fn lower(self: *const @This(), context: anytype, env: *const R, tail: anytype) @TypeOf(tail).Result {
            const C = @TypeOf(context);
            const Tail = @TypeOf(tail);
            const Continuation = struct {
                pub const Result = Tail.Result;
                tail: Tail,
                inputs: *const Capture,
                fn call(selected: @This(), value: S.Success, current_context: C, current: *const R) Result {
                    const Stored = struct {
                        pub const effect_owned_payload = true;
                        context: C,
                        tail: Tail,
                        plan: ownership.Capture(N),
                        pub fn deinit(disposable: *@This()) void {
                            disposable.plan.deinit();
                        }
                    };
                    var seed = effects.Effect(void, S.Error, R).succeed({});
                    return seed.flatMapCapturedOwned(current, Result.Success, Stored{ .context = current_context, .tail = selected.tail, .plan = .{ .value = callback(selected.inputs, value, current) } }, struct {
                        fn next(stored: *const Stored, _: void, latest: *const R) Result {
                            return stored.plan.value.lower(stored.context, latest, stored.tail);
                        }
                    }.next);
                }
            };
            return self.source.lower(context, env, Continuation{ .tail = tail, .inputs = &self.inputs });
        }
    };
}

pub fn branch(predicate: anytype, when_true: anytype, when_false: anytype) Branch(@TypeOf(predicate), @TypeOf(when_true), @TypeOf(when_false)) {
    return .{ .predicate = predicate, .left = when_true, .right = when_false };
}
fn Branch(comptime P: type, comptime L: type, comptime N: type) type {
    require(P);
    require(L);
    require(N);
    if (P.Success != bool or L.Success != N.Success or P.Error != L.Error or L.Error != N.Error or P.Environment != L.Environment or L.Environment != N.Environment) @compileError("activation branch requires bool and matching result/error/environment joins");
    return struct {
        pub const Success = L.Success;
        pub const Error = L.Error;
        pub const Environment = L.Environment;
        pub const activation_shape = struct {
            pub const kind = .branch;
            pub const predicate = P;
            pub const left = L;
            pub const right = N;
        };
        predicate: P,
        left: L,
        right: N,
        fn lower(self: *const @This(), context: anytype, env: *const Environment, tail: anytype) @TypeOf(tail).Result {
            const Tail = @TypeOf(tail);
            const C = @TypeOf(context);
            const Continuation = struct {
                pub const Result = Tail.Result;
                tail: Tail,
                left: *const L,
                right: *const N,
                fn call(selected: @This(), value: bool, current_context: C, current: *const Environment) Result {
                    return if (value) selected.left.lower(current_context, current, selected.tail) else selected.right.lower(current_context, current, selected.tail);
                }
            };
            return self.predicate.lower(context, env, Continuation{ .tail = tail, .left = &self.left, .right = &self.right });
        }
    };
}

/// Identify ordinary borrowed resource wrappers at safe persistence/result
/// boundaries. Native pointers retain their explicit application obligations.
pub fn containsResource(comptime T: type) bool {
    return switch (@typeInfo(T)) {
        .@"struct" => |s| blk: {
            if (@hasDecl(T, "AlreadyOwnedResource") and T == effects.ResourceValue(T.AlreadyOwnedResource)) break :blk true;
            inline for (s.field_types) |Field| if (containsResource(Field)) break :blk true;
            break :blk false;
        },
        .optional => |o| containsResource(o.child),
        .array => |a| containsResource(a.child),
        .@"union" => |u| blk: {
            inline for (u.field_types) |Field| if (containsResource(Field)) break :blk true;
            break :blk false;
        },
        else => false,
    };
}
fn RunResult(comptime P: type) type {
    require(P);
    if (containsResource(P.Success)) @compileError("activation resources cannot escape their activation; return durable state or an explicitly retained native owner");
    return effects.Effect(P.Success, P.Error, P.Environment);
}

/// Move owner/configuration once, select one token and fence per activation,
/// then lower every nested plan into the existing scoped Effect/bracket path.
pub fn run(env: anytype, capture: anytype, plan: anytype, comptime supplied: anytype) RunResult(@TypeOf(plan)) {
    const P = @TypeOf(plan);
    const Out = RunResult(P);
    const E = P.Error;
    const R = P.Environment;
    const C = @TypeOf(capture);
    comptime roles.validate(roles.recordType(supplied), "activation.run", &.{ "token", "fence", "now", "map_error" }, &.{});
    if (inference.result(roles.get("token", supplied), &.{ *const C, *const R }, "activation.token") != capabilities.Token) @compileError("activation.token requires the existing ownership Token");
    if (inference.result(roles.get("fence", supplied), &.{ *const C, *const R }, "activation.fence") != capabilities.Fence) @compileError("activation.fence requires the existing Fence");
    if (inference.unary(roles.get("now", supplied), *const R, "activation.now") != deadlines.WallInstant) @compileError("activation.now requires WallInstant");
    if (inference.unary(roles.get("map_error", supplied), Error, "activation.map_error") != E) @compileError("activation.map_error must preserve the plan error channel");
    const Stored = struct { inputs: ownership.Capture(C), plan: ownership.Capture(P) };
    const Context = struct {
        allocator: std.mem.Allocator,
        scope: *scopes.Scope(E),
        stored: *const Stored,
        token: capabilities.Token,
        fence: capabilities.Fence,
        live: std.atomic.Value(bool) = .init(true),
        fn check(self: *@This(), current: *const R) outcomes.Exit(void, E) {
            const map_error = roles.get("map_error", supplied);
            if (!self.live.load(.acquire)) return .{ .fail = map_error(.stale) };
            if (!self.token.address.valid() or self.token.owner_id.len == 0 or self.token.generation == 0) {
                self.live.store(false, .release);
                return .{ .fail = map_error(.invalid_token) };
            }
            switch (self.fence.enter(self.fence.userdata, self.token, roles.get("now", supplied)(current))) {
                .held => |guard| {
                    guard.release();
                    return .{ .success = {} };
                },
                .stale => {
                    self.live.store(false, .release);
                    return .{ .fail = map_error(.stale) };
                },
                .unavailable => return .{ .fail = map_error(.unavailable) },
            }
        }
    };
    const Acquire = effects.Effect(*Context, E, R);
    return Out.scopedCaptured(env, ownership.Capture(Stored){ .value = .{ .inputs = .{ .value = capture }, .plan = .{ .value = plan } } }, struct {
        fn body(stored: *const ownership.Capture(Stored), scope: *scopes.Scope(E), current: *const R) Out {
            const Inputs = struct { stored: *const Stored, scope: *scopes.Scope(E) };
            var acquisition = Acquire.fromOwned(current, Inputs{ .stored = &stored.value, .scope = scope }, struct {
                fn allocate(input: *const Inputs, latest: *const R) outcomes.Exit(*Context, E) {
                    const ctx = latest.allocator.create(Context) catch return .{ .die = outcomes.Defect.out_of_memory };
                    ctx.* = .{ .allocator = latest.allocator, .scope = input.scope, .stored = input.stored, .token = roles.get("token", supplied)(&input.stored.inputs.value, latest), .fence = roles.get("fence", supplied)(&input.stored.inputs.value, latest) };
                    return .{ .success = ctx };
                }
            }.allocate);
            return acquisition.acquireReleaseOwned(current, struct {
                fn use(ctx: *Context, latest: *const R) Out {
                    const Finish = struct {
                        pub const Result = Out;
                        fn call(_: @This(), value: P.Success, _: *Context, newest: *const R) Out {
                            return if (comptime outcomes.ownsPayload(P.Success)) Out.fromOwnedSuccess(newest, value) else Out.succeed(value);
                        }
                    };
                    return ctx.stored.plan.value.lower(ctx, latest, Finish{});
                }
            }.use, struct {
                fn release(ctx: *Context, _: outcomes.PrimaryOutcome(E)) effects.Effect(void, E, R) {
                    return effects.Effect(void, E, R).fromBorrowed(Context, ctx, struct {
                        fn destroy(value: *const Context, _: *const R) outcomes.Exit(void, E) {
                            value.allocator.destroy(@constCast(value));
                            return .{ .success = {} };
                        }
                    }.destroy);
                }
            }.release);
        }
    }.body);
}
