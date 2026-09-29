//! An RPC session is supplied by the application environment. The core
//! protocol never constructs a socket, transport loop or execution engine.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const envelope = @import("envelope.zig");

pub fn Error(comptime ProviderError: type) type {
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = true;
        provider: ProviderError,
        disconnected,
        capacity_exhausted,
        invalid_configuration,

        pub fn deinit(self: *Self) void {
            if (self.* == .provider) outcomes.discardPayload(&self.provider);
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .provider => |value| .{ .provider = try outcomes.clonePayload(value) },
                else => self.*,
            };
        }
    };
}

pub const SendAcknowledgement = enum {
    /// Accepted into the provider's outbound path; remote execution is not
    /// implied. A timeout after this point requires an idempotency policy.
    accepted,
    /// The provider cannot tell whether the remote end received the frame.
    possibly_sent,
};

/// Server-side one-chunk window. The next chunk cannot be sent until its
/// sequence is acknowledged; cancellation closes the window permanently.
pub const StreamWindow = struct {
    next_sequence: envelope.Sequence = 0,
    awaiting_acknowledgement: ?envelope.Sequence = null,
    closed: bool = false,

    pub fn beginChunk(self: *StreamWindow) outcomes.Result(envelope.Sequence, @import("rpc_error.zig").Protocol) {
        if (self.closed or self.awaiting_acknowledgement != null or
            self.next_sequence == std.math.maxInt(envelope.Sequence))
            return .{ .err = .stream_sequence };
        self.awaiting_acknowledgement = self.next_sequence;
        return .{ .ok = self.next_sequence };
    }

    pub fn acknowledge(self: *StreamWindow, sequence: envelope.Sequence) outcomes.Result(void, @import("rpc_error.zig").Protocol) {
        if (self.closed or self.awaiting_acknowledgement == null or
            self.awaiting_acknowledgement.? != sequence)
            return .{ .err = .stream_sequence };
        self.awaiting_acknowledgement = null;
        self.next_sequence += 1;
        return .{ .ok = {} };
    }

    pub fn finish(self: *StreamWindow) outcomes.Result(envelope.Sequence, @import("rpc_error.zig").Protocol) {
        if (self.closed or self.awaiting_acknowledgement != null)
            return .{ .err = .stream_sequence };
        self.closed = true;
        return .{ .ok = self.next_sequence };
    }

    pub fn cancel(self: *StreamWindow) void {
        self.closed = true;
        self.awaiting_acknowledgement = null;
    }
};

pub const Received = union(enum) {
    pub const effect_owned_payload = true;
    frame: outcomes.OwnedSlice(u8),
    closed,

    pub fn deinit(self: *@This()) void {
        if (self.* == .frame) self.frame.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .frame => |value| .{ .frame = try value.clone() },
            .closed => .closed,
        };
    }
};

pub const Policy = struct {
    max_inflight: usize,
    max_pending_frames: usize,
    frame_limits: @import("framing.zig").Limits,
    envelope_limits: envelope.Limits,

    pub fn valid(self: Policy) bool {
        const frame_capacity = std.math.add(usize, self.envelope_limits.max_envelope_bytes, @import("framing.zig").header_bytes) catch return false;
        return self.max_inflight > 0 and self.max_pending_frames > 0 and
            self.frame_limits.valid() and self.envelope_limits.valid() and
            self.frame_limits.max_frame_bytes >= self.envelope_limits.max_envelope_bytes and
            self.frame_limits.max_buffered_bytes >= frame_capacity;
    }
};

/// Frames are complete, length-prefixed values; send does not borrow its
/// input after the Effect completes. An implementation must never report
/// `accepted` before it has copied or taken ownership of the bytes. Receive
/// returns bytes allocated using the caller-supplied allocator in R.
pub fn Session(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        pub const Failure = Error(ProviderError);
        pub const SendEffect = effects.Effect(SendAcknowledgement, Failure, R);
        pub const ReceiveEffect = effects.Effect(Received, Failure, R);
        pub const ControlEffect = effects.Effect(void, Failure, R);

        userdata: ?*anyopaque,
        vtable: *const VTable,

        pub const VTable = struct {
            send: *const fn (?*anyopaque, *const R, []const u8) SendEffect,
            receive: *const fn (?*anyopaque, *const R) ReceiveEffect,
            acknowledge: *const fn (?*anyopaque, *const R, envelope.Correlation, envelope.Sequence) ControlEffect,
            cancel: *const fn (?*anyopaque, *const R, envelope.Correlation) ControlEffect,
        };

        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }

        pub fn send(comptime select: fn (*const R) *const Self, comptime select_frame: fn (*const R) []const u8) SendEffect {
            return SendEffect.deferEffect(struct {
                fn run(env: *const R) SendEffect {
                    const session = select(env);
                    return session.vtable.send(session.userdata, env, select_frame(env));
                }
            }.run);
        }

        pub fn receive(comptime select: fn (*const R) *const Self) ReceiveEffect {
            return ReceiveEffect.deferEffect(struct {
                fn run(env: *const R) ReceiveEffect {
                    const session = select(env);
                    return session.vtable.receive(session.userdata, env);
                }
            }.run);
        }

        pub fn acknowledge(
            comptime select: fn (*const R) *const Self,
            comptime correlation: fn (*const R) envelope.Correlation,
            comptime sequence: fn (*const R) envelope.Sequence,
        ) ControlEffect {
            return ControlEffect.deferEffect(struct {
                fn run(env: *const R) ControlEffect {
                    const session = select(env);
                    return session.vtable.acknowledge(session.userdata, env, correlation(env), sequence(env));
                }
            }.run);
        }

        pub fn cancel(comptime select: fn (*const R) *const Self, comptime correlation: fn (*const R) envelope.Correlation) ControlEffect {
            return ControlEffect.deferEffect(struct {
                fn run(env: *const R) ControlEffect {
                    const session = select(env);
                    return session.vtable.cancel(session.userdata, env, correlation(env));
                }
            }.run);
        }
    };
}
