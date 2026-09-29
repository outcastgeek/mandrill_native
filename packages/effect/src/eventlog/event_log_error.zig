//! Event journal authority is distinct from notifications and handler effects.
const outcomes = @import("../outcome.zig");

/// Specialize the typed eventlog error channel from its compile-time error type.
/// Parameters:
/// - `DomainError: comptime type`: Application failure type produced by domain-level operations.
/// Returns (`type`): typed event-log failure union separating domain and journal/replica faults.
pub fn Error(comptime DomainError: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = outcomes.ownsPayload(DomainError);
        domain: DomainError,
        rejected,
        uncertain_append,
        unavailable,
        malformed_entry,
        incompatible_version,
        identity_conflict,
        history_gap,
        compacted_cursor,
        replica_conflict,
        checkpoint_unavailable,
        invalid_policy,
        invalid_identity,
        cursor_exhausted,
        entry_exceeds_page_limit,
        session_rejected,
        protection_rejected,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            if (self.* == .domain) outcomes.discardPayload(&self.domain);
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .domain => |value| .{ .domain = try outcomes.clonePayload(value) },
                else => self.*,
            };
        }
    };
}
