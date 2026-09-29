//! Event journal authority is distinct from notifications and handler effects.
const outcomes = @import("../outcome.zig");

pub fn Error(comptime DomainError: type) type {
    return union(enum) {
        const Self = @This();
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
