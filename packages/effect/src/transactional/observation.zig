//! Observe committed transactional versions and publish a registry-local
//! invalidation hint outside the transaction commit gate.
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const tx = @import("transaction.zig");
const tx_ref = @import("tx_subscription_ref.zig");
const tx_error = @import("transaction_error.zig");
const registry = @import("../reactivity/atom_registry.zig");

/// Typed cause of a transaction change notification or resynchronization.
pub const ObservationCause = union(enum) {
    failure: registry.Error,
    defect: outcomes.Defect,
    interrupted,
};

/// Specialize the typed transactional error channel from its compile-time error type.
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// Returns (`type`): typed failure union for observing committed transaction changes.
pub fn Error(comptime T: type) type {
    return union(enum) {
        transaction: tx_error.Error(void),
        observation: struct { committed: tx.Observed(T), cause: ObservationCause },
    };
}

/// The committed version remains available even if local notification fails.
/// Repeating from the last observed version repairs a missed notification.
///
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `select_capture: comptime fn (*const R) *const tx_ref.SubscriptionRef(T).AwaitCapture`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_domain: comptime fn (*const R) *tx.Domain`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_registry: comptime fn (*const R) *registry.Registry`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_key: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// Returns (`effects.Effect(tx.Observed(T), Error(T), R)`): observed committed revision followed by reactive invalidation.
pub fn awaitAndInvalidate(
    comptime T: type,
    comptime R: type,
    comptime select_capture: fn (*const R) *const tx_ref.SubscriptionRef(T).AwaitCapture,
    comptime select_domain: fn (*const R) *tx.Domain,
    comptime select_registry: fn (*const R) *registry.Registry,
    comptime select_key: fn (*const R) []const u8,
) effects.Effect(tx.Observed(T), Error(T), R) {
    const Observed = tx.Observed(T);
    const Out = effects.Effect(Observed, Error(T), R);
    const Adapter = struct {
        fn transactionError(problem: tx_error.Error(void)) Error(T) {
            return .{ .transaction = problem };
        }

        fn committed(value: Observed, env: *const R) Out {
            return switch (select_registry(env).invalidate(select_key(env))) {
                .success => Out.fromOwnedSuccess(env, value),
                .fail => |cause| Out.fromOwnedOutcome(env, .{ .fail = .{
                    .observation = .{ .committed = value, .cause = .{ .failure = cause } },
                } }),
                .die => |defect| Out.fromOwnedOutcome(env, .{ .fail = .{
                    .observation = .{ .committed = value, .cause = .{ .defect = defect } },
                } }),
                .interrupted => Out.fromOwnedOutcome(env, .{ .fail = .{
                    .observation = .{ .committed = value, .cause = .interrupted },
                } }),
                .finalization_failed => unreachable,
            };
        }

        fn run(env: *const R) Out {
            var awaiting = tx_ref.SubscriptionRef(T).awaitAfter(R, select_capture(env), select_domain);
            var mapped = awaiting.mapErrorOwned(env, transactionError);
            return mapped.flatMapOwned(env, committed);
        }
    };
    return Out.deferEffect(Adapter.run);
}
