//! Logical singleton execution using an injected ownership capability. This
//! grants one active lease, not exactly-once external side effects; callers
//! use stable request identities and saved results for replay safety.
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const ownership = @import("cluster_capabilities.zig");

pub fn withLease(
    comptime R: type,
    comptime E: type,
    comptime A: type,
    comptime ProviderError: type,
    comptime guarantee: ownership.Guarantee,
    comptime select_ownership: fn (*const R) *const ownership.Ownership(R, ProviderError, guarantee),
    comptime select_address: fn (*const R) ownership.EntityAddress,
    comptime select_owner_id: fn (*const R) []const u8,
    comptime select_policy: fn (*const R) ownership.LeasePolicy,
    comptime map_ownership_error: fn (ownership.Ownership(R, ProviderError, guarantee).Failure) E,
    comptime on_unavailable: fn () E,
    comptime on_release_problem: fn (ownership.Release) E,
    comptime use: fn (ownership.Lease, *const R) effects.Effect(A, E, R),
) effects.Effect(A, E, R) {
    const Service = ownership.Ownership(R, ProviderError, guarantee);
    const Acquire = effects.Effect(ownership.Lease, E, R);
    const Release = effects.Effect(void, E, R);
    const Adapter = struct {
        fn acquired(result: ownership.Acquisition, env: *const R) Acquire {
            return switch (result) {
                .acquired => |lease| Acquire.fromOwnedSuccess(env, lease),
                .unavailable => Acquire.fromOwnedOutcome(env, .{ .fail = on_unavailable() }),
            };
        }

        fn released(_: *const ownership.Lease, result: ownership.Release, env: *const R) Release {
            return switch (result) {
                .released, .already_released => Release.succeed({}),
                .stale, .possibly_released => Release.fromOwnedOutcome(env, .{ .fail = on_release_problem(result) }),
            };
        }
    };

    const acquire = Acquire.deferEffect(struct {
        fn start(env: *const R) Acquire {
            const provider: *const Service = select_ownership(env);
            var attempt = provider.vtable.acquire(provider.userdata, env, select_address(env), select_owner_id(env), select_policy(env));
            var mapped = attempt.mapErrorOwned(env, map_ownership_error);
            return mapped.flatMapOwned(env, Adapter.acquired);
        }
    }.start);
    return acquire.acquireRelease(use, struct {
        fn finish(lease: ownership.Lease, _: outcomes.PrimaryOutcome(E), env: *const R) Release {
            const provider: *const Service = select_ownership(env);
            var result = provider.vtable.release(provider.userdata, env, lease.token());
            var mapped = result.mapErrorOwned(env, map_ownership_error);
            return mapped.flatMapCapturedOwned(env, void, lease, Adapter.released);
        }
    }.finish);
}
