//! Logical delivery, ownership and observer states are not interchangeable.
const outcomes = @import("../outcome.zig");

/// Typed ownership conflict record; release owned state with `deinit`.
pub const OwnershipConflict = struct { expected_generation: u64, actual_generation: u64 };

/// Specialize the typed cluster error channel from its compile-time error type.
/// Parameters:
/// - `DomainError: comptime type`: Application failure type produced by domain-level operations.
/// Returns (`type`): tagged cluster failure union separating domain errors, ownership loss, uncertain delivery, and protocol faults.
pub fn Error(comptime DomainError: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = outcomes.ownsPayload(DomainError);
        domain: DomainError,
        rejected_before_acceptance,
        possibly_accepted,
        unavailable,
        malformed,
        invalid_policy,
        incompatible_version,
        conflicting_request,
        conflicting_chunk,
        invalid_chunk_sequence,
        chunk_exceeds_page_limit,
        ownership_lost: OwnershipConflict,
        uncertain_completion,
        reply_expired,

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
