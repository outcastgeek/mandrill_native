//! Pool factories share one generated input owner and the existing lease kernel.
const effects = @import("../effect.zig");
const pools = @import("../pool.zig");
const outcomes = @import("../outcome.zig");
const inference = @import("inference.zig");
const roles = @import("roles.zig");
const ownership = @import("ownership.zig");

fn Factory(comptime T: type, comptime R: type, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "pool", &.{ "configure", "create", "validate", "dispose" }, &.{});
    ownership.validate(T);
    ownership.requireConcurrent(T, "pool");
    const Create = inference.result(roles.get("create", supplied), &.{ *const T, *const R }, "pool.create");
    inference.requireEffect(Create, "pool.create");
    if (Create.Environment != R) @compileError("pool.create environment mismatch; use an explicit projection");
    const A = Create.Success;
    const E = Create.Error;
    ownership.validate(A);
    if (ownership.containsOwned(A) and !outcomes.ownsPayload(A)) @compileError("pool.create owning success requires declared payload ownership before Effect composition; use dsl.ownership.Capture for a generated composite owner");
    if (inference.result(roles.get("configure", supplied), &.{ *const T, *const R }, "pool.configure") != pools.Config) @compileError("pool.configure requires the existing explicit PoolConfig");
    if (inference.result(roles.get("validate", supplied), &.{ *const T, A, *const R }, "pool.validate") != effects.Effect(bool, E, R)) @compileError("pool.validate requires exact Effect(bool,E,R) matching pool.create");
    if (inference.result(roles.get("dispose", supplied), &.{ *const T, A, *const R }, "pool.dispose") != effects.Effect(void, E, R)) @compileError("pool.dispose requires exact Effect(void,E,R) matching pool.create");
    const Inputs = T;
    const Operations = effects.EffectPool(A, E, R);
    return struct {
        pub const Out = effects.Effect(*pools.Pool(A, E), E, R);
        fn configure(inputs: *const Inputs, current: *const R) pools.Config {
            return roles.get("configure", supplied)(inputs, current);
        }
        fn create(inputs: *const Inputs, current: *const R) Create {
            return roles.get("create", supplied)(inputs, current);
        }
        fn validate(inputs: *const Inputs, value: A, current: *const R) effects.Effect(bool, E, R) {
            return roles.get("validate", supplied)(inputs, value, current);
        }
        fn dispose(inputs: *const Inputs, value: A, current: *const R) effects.Effect(void, E, R) {
            return roles.get("dispose", supplied)(inputs, value, current);
        }
        fn make(env: *const R, scope: anytype, inputs: T) Out {
            return Operations.makeCaptured(env, scope, inputs, configure, create, validate, dispose);
        }
    };
}

pub fn make(env: anytype, scope: anytype, inputs: anytype, comptime supplied: anytype) Factory(@TypeOf(inputs), @typeInfo(@TypeOf(env)).pointer.child, supplied).Out {
    return Factory(@TypeOf(inputs), @typeInfo(@TypeOf(env)).pointer.child, supplied).make(env, scope, inputs);
}
