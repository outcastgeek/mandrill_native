//! Process-local resource acquisition for one entity activation. The owner
//! token is durable contract data; the live resource is never persisted.
//! Each new activation reacquires it and its finalizer runs exactly once.
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const capabilities = @import("cluster_capabilities.zig");

pub fn EntityResource(comptime Resource: type, comptime E: type, comptime R: type) type {
    return struct {
        const Self = @This();
        pub const AcquireEffect = effects.Effect(Resource, E, R);
        pub const ReleaseEffect = effects.Effect(void, E, R);

        userdata: ?*anyopaque,
        vtable: *const VTable,

        /// Acquire produces an owned Resource. Release consumes that value,
        /// including on body failure, interruption or defect. Its own failure
        /// is attached as finalization failure without erasing the body Exit.
        pub const VTable = struct {
            acquire: *const fn (?*anyopaque, *const R, capabilities.Token) AcquireEffect,
            release: *const fn (?*anyopaque, *const R, Resource, outcomes.PrimaryOutcome(E)) ReleaseEffect,
        };

        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }

        pub fn with(
            comptime A: type,
            comptime select_resource: fn (*const R) *const Self,
            comptime select_token: fn (*const R) capabilities.Token,
            comptime use: fn (Resource, *const R) effects.Effect(A, E, R),
        ) effects.Effect(A, E, R) {
            const acquire = AcquireEffect.deferEffect(struct {
                fn start(env: *const R) AcquireEffect {
                    const provider = select_resource(env);
                    return provider.vtable.acquire(provider.userdata, env, select_token(env));
                }
            }.start);
            return acquire.acquireRelease(use, struct {
                fn finish(resource: Resource, primary: outcomes.PrimaryOutcome(E), env: *const R) ReleaseEffect {
                    const provider = select_resource(env);
                    return provider.vtable.release(provider.userdata, env, resource, primary);
                }
            }.finish);
        }
    };
}
