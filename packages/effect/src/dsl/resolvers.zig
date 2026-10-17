//! Inferred provider roles use the existing typed resolver as their anchor.
const resolvers = @import("../request_resolver.zig");
const outcomes = @import("../outcome.zig");
const inference = @import("inference.zig");
const roles = @import("roles.zig");
const ownership = @import("ownership.zig");

fn Factory(comptime Resolver: type, comptime T: type, comptime R: type, comptime supplied: anytype, comptime is_grouped: bool) type {
    if (@typeInfo(Resolver) != .@"struct" or !@hasDecl(Resolver, "RequestDefinition") or !@hasDecl(Resolver, "Environment") or !@hasDecl(Resolver, "ProviderFailure")) @compileError("resolver roles require the existing typed RequestResolver anchor");
    if (Resolver != resolvers.Resolver(Resolver.RequestDefinition, Resolver.Environment, Resolver.ProviderFailure)) @compileError("resolver roles reject forged resolver evidence");
    if (Resolver.Environment != R) @compileError("resolver environment mismatch; use an explicit projection");
    roles.validate(roles.recordType(supplied), "resolver", if (is_grouped) &.{ "jobs", "resolve", "group" } else &.{ "jobs", "resolve" }, &.{});
    ownership.validate(T);
    inline for (.{ Resolver.Success, Resolver.DomainError, Resolver.ProviderFailure }) |A| {
        ownership.validate(A);
        if (ownership.containsOwned(A) and !outcomes.ownsPayload(A)) @compileError("resolver owning result/error channels require declared payload ownership before composition");
    }
    const SelectedJobs = inference.result(roles.get("jobs", supplied), &.{ *const T, *const R }, "resolver.jobs");
    if (SelectedJobs != []const Resolver.Job and SelectedJobs != []Resolver.Job) @compileError("resolver.jobs requires a borrowed slice of the selected resolver Job");
    if (inference.result(roles.get("resolve", supplied), &.{ *const T, []const Resolver.Job, *const R }, "resolver.resolve") != Resolver.BatchEffect) @compileError("resolver.resolve requires the selected resolver's exact BatchEffect");
    if (is_grouped) {
        const Group = inference.result(roles.get("group", supplied), &.{ *const T, *const Resolver.Job, *const R }, "resolver.group");
        if (Group != []const u8 and Group != []u8) @compileError("resolver.group requires a borrowed encoded primary key");
    }
    return struct {
        pub const Out = Resolver.BatchEffect;
        fn jobs(inputs: *const T, current: *const R) []const Resolver.Job {
            return roles.get("jobs", supplied)(inputs, current);
        }
        fn resolve(inputs: *const T, selected: []const Resolver.Job, current: *const R) Out {
            return roles.get("resolve", supplied)(inputs, selected, current);
        }
        fn group(inputs: *const T, job: *const Resolver.Job, current: *const R) []const u8 {
            return roles.get("group", supplied)(inputs, job, current);
        }
        fn make(env: *const R, inputs: T) Out {
            if (comptime is_grouped) return Resolver.resolveGroupedCaptured(env, inputs, jobs, group, resolve);
            return Resolver.resolveBatchCaptured(env, inputs, jobs, resolve);
        }
    };
}

pub fn batch(comptime Resolver: type, env: anytype, inputs: anytype, comptime supplied: anytype) Factory(Resolver, @TypeOf(inputs), @typeInfo(@TypeOf(env)).pointer.child, supplied, false).Out {
    return Factory(Resolver, @TypeOf(inputs), @typeInfo(@TypeOf(env)).pointer.child, supplied, false).make(env, inputs);
}
pub fn grouped(comptime Resolver: type, env: anytype, inputs: anytype, comptime supplied: anytype) Factory(Resolver, @TypeOf(inputs), @typeInfo(@TypeOf(env)).pointer.child, supplied, true).Out {
    return Factory(Resolver, @TypeOf(inputs), @typeInfo(@TypeOf(env)).pointer.child, supplied, true).make(env, inputs);
}
