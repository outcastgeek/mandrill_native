//! RPC keeps domain, middleware, payload and session failures separate.
const outcomes = @import("../outcome.zig");
const Issue = @import("../schema/schema_issue.zig").Issue;

/// Defect information received from a remote peer without recasting it as domain `E`.
pub const RemoteDefect = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    kind: outcomes.Defect.Kind,
    diagnostic: outcomes.OwnedSlice(u8),

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *RemoteDefect`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *RemoteDefect) void {
        self.diagnostic.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const RemoteDefect`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!RemoteDefect`): A `RemoteDefect` on success; `OutOfMemory` if required allocation fails.
    pub fn clone(self: *const RemoteDefect) error{OutOfMemory}!RemoteDefect {
        return .{ .kind = self.kind, .diagnostic = try self.diagnostic.clone() };
    }
};

/// Typed RPC protocol incompatibility or malformed-message failure.
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

/// Specialize the typed rpc error channel from its compile-time error type.
/// Parameters:
/// - `DomainError: comptime type`: Application failure type produced by domain-level operations.
/// - `MiddlewareError: comptime type`: Compile-time typed failure introduced by middleware.
/// - `SessionError: comptime type`: Compile-time typed session failure.
/// Returns (`type`): typed RPC failure union separating protocol, transport, and domain errors.
pub fn Error(comptime DomainError: type, comptime MiddlewareError: type, comptime SessionError: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        domain: DomainError,
        middleware: MiddlewareError,
        session: SessionError,
        serialization: Issue,
        protocol: Protocol,
        remote_defect: RemoteDefect,
        remote_interrupted,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
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

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
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
