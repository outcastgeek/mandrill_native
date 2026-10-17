//! Named cache roles retain their inputs in the existing managed scope.
const effects = @import("../effect.zig");
const cache = @import("../cache.zig");
const dependencies = @import("../dependency_cache.zig");
const outcomes = @import("../outcome.zig");
const inference = @import("inference.zig");
const roles = @import("roles.zig");
const ownership = @import("ownership.zig");

fn PolicyItem(comptime Policy: type) type {
    if (@typeInfo(Policy) != .@"struct" or !@hasField(Policy, "retain")) @compileError("cache.configure requires the existing typed cache key/dependency policy");
    const Retain = @FieldType(Policy, "retain");
    if (@typeInfo(Retain) != .pointer or @typeInfo(@typeInfo(Retain).pointer.child) != .@"fn") @compileError("cache.configure retention policy must have a typed retain callback");
    const params = @typeInfo(@typeInfo(Retain).pointer.child).@"fn".param_types;
    if (params.len != 2 or params[1] == null) @compileError("cache.configure retention policy needs an unambiguous item type");
    return params[1].?;
}

fn Factory(comptime T: type, comptime R: type, comptime supplied: anytype, comptime dependent: bool) type {
    roles.validate(roles.recordType(supplied), "cache", &.{ "configure", "load" }, &.{});
    ownership.validate(T);
    ownership.requireConcurrent(T, "cache.load");
    const Config = inference.result(roles.get("configure", supplied), &.{ *const T, *const R }, "cache.configure");
    if (@typeInfo(Config) != .@"struct") @compileError("cache.configure requires the existing native cache configuration");
    if (dependent and !@hasField(Config, "retention")) @compileError("dependencyCache.configure requires DependencyCacheConfig");
    const Retention = if (dependent) @FieldType(Config, "retention") else Config;
    if (@typeInfo(Retention) != .@"struct" or !@hasField(Retention, "keys")) @compileError("cache.configure requires the existing typed cache configuration");
    const Key = PolicyItem(@FieldType(Retention, "keys"));
    const Load = inference.result(roles.get("load", supplied), &.{ *const T, Key, *const R }, "cache.load");
    inference.requireEffect(Load, "cache.load");
    if (Load.Environment != R) @compileError("cache.load environment mismatch; use an explicit projection");
    const E = Load.Error;
    const D = if (dependent) blk: {
        if (!@hasField(Config, "dependency_policy")) @compileError("dependencyCache.configure requires an explicit dependency retention policy");
        break :blk PolicyItem(@FieldType(Config, "dependency_policy"));
    } else void;
    const A = if (dependent) blk: {
        if (@typeInfo(Load.Success) != .@"struct" or !@hasField(Load.Success, "value")) @compileError("dependencyCache.load requires the existing Loaded dependency result");
        break :blk @FieldType(Load.Success, "value");
    } else Load.Success;
    if (dependent) {
        if (Config != dependencies.Config(Key, D, E) or Load.Success != dependencies.Loaded(D, A)) @compileError("dependencyCache roles must agree with the existing Config and Loaded contracts");
    } else if (Config != cache.Config(Key, E)) @compileError("cache.configure key/error policy must match cache.load");
    ownership.validate(A);
    if (ownership.containsOwned(A) and !outcomes.ownsPayload(A)) @compileError("cache.load owning success requires declared payload ownership before Effect composition; use dsl.ownership.Capture for a generated composite owner");
    const Inputs = T;
    const Operations = if (dependent) effects.DependencyCache(Key, D, A, E, R) else effects.EffectCache(Key, A, E, R);
    const Core = if (dependent) Operations.Core else cache.Cache(Key, A, E);
    return struct {
        pub const Out = effects.Effect(*Core, E, R);
        fn configure(inputs: *const Inputs, current: *const R) Config {
            return roles.get("configure", supplied)(inputs, current);
        }
        fn load(inputs: *const Inputs, key: Key, current: *const R) Load {
            return roles.get("load", supplied)(inputs, key, current);
        }
        fn make(env: *const R, scope: anytype, inputs: T) Out {
            const owned: Inputs = inputs;
            if (comptime dependent) return Operations.makeDependentCaptured(env, scope, owned, configure, load);
            return Operations.makeCaptured(env, scope, owned, configure, load);
        }
    };
}

pub fn make(env: anytype, scope: anytype, inputs: anytype, comptime supplied: anytype) Factory(@TypeOf(inputs), @typeInfo(@TypeOf(env)).pointer.child, supplied, false).Out {
    return Factory(@TypeOf(inputs), @typeInfo(@TypeOf(env)).pointer.child, supplied, false).make(env, scope, inputs);
}
pub fn makeDependent(env: anytype, scope: anytype, inputs: anytype, comptime supplied: anytype) Factory(@TypeOf(inputs), @typeInfo(@TypeOf(env)).pointer.child, supplied, true).Out {
    return Factory(@TypeOf(inputs), @typeInfo(@TypeOf(env)).pointer.child, supplied, true).make(env, scope, inputs);
}
