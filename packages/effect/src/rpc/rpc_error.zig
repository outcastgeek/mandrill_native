//! RPC keeps domain, middleware, payload and session failures separate.
const outcomes = @import("../outcome.zig");
const Issue = @import("../schema/schema_issue.zig").Issue;

pub const RemoteDefect = struct {
    pub const effect_owned_payload = true;
    kind: outcomes.Defect.Kind,
    diagnostic: outcomes.OwnedSlice(u8),

    pub fn deinit(self: *RemoteDefect) void {
        self.diagnostic.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const RemoteDefect) error{OutOfMemory}!RemoteDefect {
        return .{ .kind = self.kind, .diagnostic = try self.diagnostic.clone() };
    }
};

pub const Protocol = union(enum) {
    malformed_envelope,
    unknown_operation,
    incompatible_version: struct { requested: u32, supported: u32 },
    duplicate_operation,
    correlation_conflict,
    invalid_idempotency_key,
    stream_sequence,
    frame_too_large,
};

pub fn Error(comptime DomainError: type, comptime MiddlewareError: type, comptime SessionError: type) type {
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = true;
        domain: DomainError,
        middleware: MiddlewareError,
        session: SessionError,
        serialization: Issue,
        protocol: Protocol,
        remote_defect: RemoteDefect,
        remote_interrupted,

        pub fn deinit(self: *Self) void {
            switch (self.*) {
                .domain => outcomes.discardPayload(&self.domain),
                .middleware => outcomes.discardPayload(&self.middleware),
                .session => outcomes.discardPayload(&self.session),
                .serialization => self.serialization.deinit(),
                .remote_defect => self.remote_defect.deinit(),
                else => {},
            }
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .domain => |value| .{ .domain = try outcomes.clonePayload(value) },
                .middleware => |value| .{ .middleware = try outcomes.clonePayload(value) },
                .session => |value| .{ .session = try outcomes.clonePayload(value) },
                .serialization => |value| .{ .serialization = try value.clone() },
                .remote_defect => |value| .{ .remote_defect = try value.clone() },
                else => self.*,
            };
        }
    };
}
