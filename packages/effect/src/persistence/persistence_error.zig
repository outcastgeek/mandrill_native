//! Persistence failures distinguish absence, bad data and unknown commit state.
const outcomes = @import("../outcome.zig");
const Issue = @import("../schema/schema_issue.zig").Issue;

pub const VersionConflict = struct { expected: u32, stored: u32 };
pub const ClaimConflict = struct { expected_generation: u64, actual_generation: u64 };

pub fn Error(comptime DomainError: type) type {
    return union(enum) {
        const Self = @This();
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

        pub fn deinit(self: *Self) void {
            if (self.* == .domain) outcomes.discardPayload(&self.domain);
            if (self.* == .schema) self.schema.deinit();
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .domain => |value| .{ .domain = try outcomes.clonePayload(value) },
                .schema => |value| .{ .schema = try value.clone() },
                else => self.*,
            };
        }
    };
}
