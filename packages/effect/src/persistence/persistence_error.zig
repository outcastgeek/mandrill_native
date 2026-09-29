//! Persistence failures distinguish absence, bad data and unknown commit state.
const outcomes = @import("../outcome.zig");
const Issue = @import("../schema/schema_issue.zig").Issue;

/// Typed version conflict record; release owned state with `deinit`.
pub const VersionConflict = struct { expected: u32, stored: u32 };
/// Typed claim conflict record; release owned state with `deinit`.
pub const ClaimConflict = struct { expected_generation: u64, actual_generation: u64 };

/// Specialize the typed persistence error channel from its compile-time error type.
/// Parameters:
/// - `DomainError: comptime type`: Application failure type produced by domain-level operations.
/// Returns (`type`): typed persistence failure union distinguishing domain, provider, and storage-contract failures.
pub fn Error(comptime DomainError: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        domain: DomainError,
        schema: Issue,
        missing,
        expired,
        malformed,
        incompatible_version: VersionConflict,
        unavailable,
        uncertain_commit,
        claim_conflict: ClaimConflict,
        identity_conflict,
        dead_letter,
        entry_exceeds_page_limit,
        invalid_policy,
        revision_exhausted,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            if (self.* == .domain) outcomes.discardPayload(&self.domain);
            if (self.* == .schema) self.schema.deinit();
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
                .schema => |value| .{ .schema = try value.clone() },
                else => self.*,
            };
        }
    };
}
