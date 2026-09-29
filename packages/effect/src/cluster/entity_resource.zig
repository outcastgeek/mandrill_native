//! Process-local resource acquisition for one entity activation. The owner
//! token is durable contract data; the live resource is never persisted.
//! Each new activation reacquires it and its finalizer runs exactly once.
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const capabilities = @import("cluster_capabilities.zig");

/// Specialize scoped acquisition and release of an entity resource.
/// Parameters:
/// - `Resource: comptime type`: Compile-time resource type acquired and released within the scope.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): scoped entity resource binding that pairs acquisition and release.
pub fn EntityResource(comptime Resource: type, comptime E: type, comptime R: type) type {
    return struct {
        const Self = @This();
        /// Lazy Effect for acquire effect; success, error, and environment types are fixed by
        /// `effects.Effect(Resource, E, R)`.
        pub const AcquireEffect = effects.Effect(Resource, E, R);
        /// Lazy Effect for release effect; success, error, and environment types are fixed by
        /// `effects.Effect(void, E, R)`.
        pub const ReleaseEffect = effects.Effect(void, E, R);

        userdata: ?*anyopaque,
        vtable: *const VTable,

        /// Acquire produces an owned Resource. Release consumes that value,
        /// including on body failure, interruption or defect. Its own failure
        /// is attached as finalization failure without erasing the body Exit.
        /// Typed function-pointer interface for this capability. Implement every
        /// hook with the exact signature below; keep the table valid while the
        /// capability or pending work can invoke it.
        pub const VTable = struct {
            /// Acquire through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: capabilities.Token`: typed input consumed by this hook.
            /// Returns (`AcquireEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            acquire: *const fn (?*anyopaque, *const R, capabilities.Token) AcquireEffect,
            /// Release through this provider.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: Resource`: typed domain value consumed by this hook.
            /// - `4: outcomes.PrimaryOutcome(E)`: typed input consumed by this hook.
            /// Returns (`ReleaseEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            release: *const fn (?*anyopaque, *const R, Resource, outcomes.PrimaryOutcome(E)) ReleaseEffect,
        };

        /// Wrap a caller-supplied provider implementation.
        /// Parameters:
        /// - `data: ?*anyopaque`: Borrowed provider context passed unchanged to every hook.
        ///   Its owner keeps it alive through any pending effect or operation.
        /// - `vtable: *const VTable`: Borrowed table of exactly typed callbacks; keep it
        ///   alive for every use of this capability, including deferred execution.
        /// Returns (`Self`): A non-owning adapter referencing `data` and `vtable`; no
        ///   provider state is allocated or transferred by this constructor.
        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }

        /// Run a body while the entity resource is acquired.
        /// Parameters:
        /// - `A: comptime type`: Value type produced by a successful computation.
        /// - `select_resource: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_token: comptime fn (*const R) capabilities.Token`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `use: comptime fn (Resource, *const R) effects.Effect(A, E, R)`: Compile-time typed function selected at comptime; no runtime callback lookup
        ///   is required.
        /// Returns (`effects.Effect(A, E, R)`): body result while the entity resource is acquired and released by its scope.
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
