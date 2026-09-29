//! Retry is a transaction control operation, never a generic storage error.
const outcomes = @import("../outcome.zig");

pub fn Error(comptime DomainError: type) type {
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = outcomes.ownsPayload(DomainError);
        domain: DomainError,
        cross_domain,
        mutable_alias,
        invalid_policy,
        unavailable,
        conflict_limit,
        version_exhausted,
        empty_retry_read_set,

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
