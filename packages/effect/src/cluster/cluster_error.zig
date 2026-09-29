//! Logical delivery, ownership and observer states are not interchangeable.
const outcomes = @import("../outcome.zig");

pub const OwnershipConflict = struct { expected_generation: u64, actual_generation: u64 };

pub fn Error(comptime DomainError: type) type {
    return union(enum) {
        const Self = @This();
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

        pub fn deinit(self: *Self) void {
            if (self.* == .domain) outcomes.discardPayload(&self.domain);
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .domain => |value| .{ .domain = try outcomes.clonePayload(value) },
                else => self.*,
            };
        }
    };
}
