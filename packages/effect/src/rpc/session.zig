//! An RPC session is supplied by the application environment. The core
//! protocol never constructs a socket, transport loop or execution engine.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const envelope = @import("envelope.zig");

/// Specialize the typed rpc error channel from its compile-time error type.
/// Parameters:
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// Returns (`type`): typed session failure union retaining transport and protocol errors.
pub fn Error(comptime ProviderError: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        provider: ProviderError,
        disconnected,
        capacity_exhausted,
        invalid_configuration,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            if (self.* == .provider) outcomes.discardPayload(&self.provider);
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .provider => |value| .{ .provider = try outcomes.clonePayload(value) },
                else => self.*,
            };
        }
    };
}

/// `SendAcknowledgement` cases: `accepted`, `possibly_sent`.
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

    /// Begin a sequenced RPC stream chunk.
    /// Parameters:
    /// - `self: *StreamWindow`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`outcomes.Result(envelope.Sequence, @import("rpc_error.zig").Protocol)`): An ok/err Result; owned payloads must be released according to their type contract.
    pub fn beginChunk(self: *StreamWindow) outcomes.Result(envelope.Sequence, @import("rpc_error.zig").Protocol) {
        if (self.closed or self.awaiting_acknowledgement != null or
            self.next_sequence == std.math.maxInt(envelope.Sequence))
            return .{ .err = .stream_sequence };
        self.awaiting_acknowledgement = self.next_sequence;
        return .{ .ok = self.next_sequence };
    }

    /// Acknowledge processing through the selected capability.
    /// Parameters:
    /// - `self: *StreamWindow`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `sequence: envelope.Sequence`: Protocol sequence number used for ordering.
    /// Returns (`outcomes.Result(void, @import("rpc_error.zig").Protocol)`): An ok/err Result; owned payloads must be released according to their type contract.
    pub fn acknowledge(self: *StreamWindow, sequence: envelope.Sequence) outcomes.Result(void, @import("rpc_error.zig").Protocol) {
        if (self.closed or self.awaiting_acknowledgement == null or
            self.awaiting_acknowledgement.? != sequence)
            return .{ .err = .stream_sequence };
        self.awaiting_acknowledgement = null;
        self.next_sequence += 1;
        return .{ .ok = {} };
    }

    /// Finish the current RPC stream or session operation.
    /// Parameters:
    /// - `self: *StreamWindow`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`outcomes.Result(envelope.Sequence, @import("rpc_error.zig").Protocol)`): An ok/err Result; owned payloads must be released according to their type contract.
    pub fn finish(self: *StreamWindow) outcomes.Result(envelope.Sequence, @import("rpc_error.zig").Protocol) {
        if (self.closed or self.awaiting_acknowledgement != null)
            return .{ .err = .stream_sequence };
        self.closed = true;
        return .{ .ok = self.next_sequence };
    }

    /// Request cancellation of the selected operation.
    /// Parameters:
    /// - `self: *StreamWindow`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn cancel(self: *StreamWindow) void {
        self.closed = true;
        self.awaiting_acknowledgement = null;
    }
};

/// Framed session receive result, including the decoded response or transport closure.
pub const Received = union(enum) {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    frame: outcomes.OwnedSlice(u8),
    closed,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        if (self.* == .frame) self.frame.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .frame => |value| .{ .frame = try value.clone() },
            .closed => .closed,
        };
    }
};

/// Caller-selected session bounds and transport behavior.
pub const Policy = struct {
    max_inflight: usize,
    max_pending_frames: usize,
    frame_limits: @import("framing.zig").Limits,
    envelope_limits: envelope.Limits,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Policy`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` when admission and pending-frame bounds are positive and envelope/frame limits fit.
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
///
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// Returns (`type`): framed RPC session that owns pending calls and applies explicit limits.
pub fn Session(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        /// Typed failure channel combining domain and provider errors.
        pub const Failure = Error(ProviderError);
        /// Lazy Effect for send effect; success, error, and environment types are fixed by
        /// `effects.Effect(SendAcknowledgement, Failure, R)`.
        pub const SendEffect = effects.Effect(SendAcknowledgement, Failure, R);
        /// Lazy Effect for receive effect; success, error, and environment types are fixed by
        /// `effects.Effect(Received, Failure, R)`.
        pub const ReceiveEffect = effects.Effect(Received, Failure, R);
        /// Lazy Effect for control effect; success, error, and environment types are fixed by
        /// `effects.Effect(void, Failure, R)`.
        pub const ControlEffect = effects.Effect(void, Failure, R);

        userdata: ?*anyopaque,
        vtable: *const VTable,

        /// Typed function-pointer interface for this capability. Implement every
        /// hook with the exact signature below; keep the table valid while the
        /// capability or pending work can invoke it.
        pub const VTable = struct {
            /// Send a frame or message.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// Returns (`SendEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            send: *const fn (?*anyopaque, *const R, []const u8) SendEffect,
            /// Receive a frame or message.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// Returns (`ReceiveEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            receive: *const fn (?*anyopaque, *const R) ReceiveEffect,
            /// Acknowledge completed work.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: envelope.Correlation`: stable typed identity for the selected operation.
            /// - `4: envelope.Sequence`: typed domain value consumed by this hook.
            /// Returns (`ControlEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            acknowledge: *const fn (?*anyopaque, *const R, envelope.Correlation, envelope.Sequence) ControlEffect,
            /// Request cancellation.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: envelope.Correlation`: stable typed identity for the selected operation.
            /// Returns (`ControlEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            cancel: *const fn (?*anyopaque, *const R, envelope.Correlation) ControlEffect,
        };

        /// Wrap a caller-supplied provider implementation.
        /// Parameters:
        /// - `data: ?*anyopaque`: Borrowed provider context passed unchanged to every hook.
        ///   Its owner keeps it alive through any pending effect or operation.
        /// - `vtable: *const VTable`: Borrowed table of exactly typed callbacks; keep it
        ///   alive for every use of this capability, including deferred execution.
        /// Returns (`Self`): A non-owning adapter referencing `data` and `vtable`; no
        ///   provider state is allocated or transferred by this constructor.
        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }

        /// Send the typed or encoded message.
        /// Parameters:
        /// - `select: comptime fn (*const R) *const Self`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// - `select_frame: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`SendEffect`): typed send acknowledgement after framing the message.
        pub fn send(comptime select: fn (*const R) *const Self, comptime select_frame: fn (*const R) []const u8) SendEffect {
            return SendEffect.deferEffect(struct {
                fn run(env: *const R) SendEffect {
                    const session = select(env);
                    return session.vtable.send(session.userdata, env, select_frame(env));
                }
            }.run);
        }

        /// Receive  through the selected provider.
        /// Parameters:
        /// - `select: comptime fn (*const R) *const Self`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`ReceiveEffect`): next typed framed response, or a session/transport failure.
        pub fn receive(comptime select: fn (*const R) *const Self) ReceiveEffect {
            return ReceiveEffect.deferEffect(struct {
                fn run(env: *const R) ReceiveEffect {
                    const session = select(env);
                    return session.vtable.receive(session.userdata, env);
                }
            }.run);
        }

        /// Acknowledge processing through the selected capability.
        /// Parameters:
        /// - `select: comptime fn (*const R) *const Self`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// - `correlation: comptime fn (*const R) envelope.Correlation`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// - `sequence: comptime fn (*const R) envelope.Sequence`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`ControlEffect`): typed acknowledgement of consumed response progress.
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

        /// Request cancellation of the selected operation.
        /// Parameters:
        /// - `select: comptime fn (*const R) *const Self`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// - `correlation: comptime fn (*const R) envelope.Correlation`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`ControlEffect`): typed completion of a cancellation request for the selected call.
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
