//! Bounded correlation registry for a transport-neutral RPC session. A single
//! session receiver can deliver owned response envelopes to concurrent calls.
const std = @import("std");
const outcomes = @import("../../outcome.zig");
const effects = @import("../../effect.zig");
const primitives = @import("../../primitives.zig");
const envelope = @import("../envelope.zig");
const framing = @import("../framing.zig");
const transport = @import("../session.zig");
const Protocol = @import("../rpc_error.zig").Protocol;
const Cancellation = @import("../../internal/runtime.zig").Cancellation;

/// Session failure for closure, exhausted capacity, correlation conflict, or unexpected reply.
pub const Error = enum { closed, capacity_exhausted, correlation_conflict, unexpected_response };
/// `Delivery` cases: `accepted`, `orphaned`, `duplicate`, `conflict`.
pub const Delivery = enum { accepted, orphaned, duplicate, conflict };

/// Specialize the rpc failure variants from their typed channels.
/// Parameters:
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// Returns (`type`): typed receive failure union for a session.
pub fn ReceiveError(comptime ProviderError: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        session: transport.Error(ProviderError),
        protocol: Protocol,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            if (self.* == .session) outcomes.discardPayload(&self.session);
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .session => |value| .{ .session = try outcomes.clonePayload(value) },
                .protocol => self.*,
            };
        }
    };
}

/// Build a closed registry of completed rpc definitions.
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): bounded ownership registry for pending unary responses.
pub fn Registry(comptime R: type) type {
    return struct {
        const Self = @This();
        /// Owned record of a unary response awaiting its correlation ID.
        pub const Pending = struct {
            correlation: envelope.Correlation,
            operation: []u8,
            version: envelope.Version,
            allocator: std.mem.Allocator,
            response: primitives.Deferred(envelope.Envelope, Error),

            /// Wait for the response paired with this RPC correlation.
            /// Parameters:
            /// - `self: *Pending`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
            /// Returns (`effects.Effect(envelope.Envelope, Error, R)`): typed response correlated to one pending unary call.
            pub fn awaitResponse(self: *Pending) effects.Effect(envelope.Envelope, Error, R) {
                return effects.Effect(envelope.Envelope, Error, R).awaitDeferred(&self.response);
            }

            fn deinit(self: *Pending) void {
                self.response.deinit();
                self.allocator.free(self.operation);
                self.allocator.destroy(self);
            }
        };

        allocator: std.mem.Allocator,
        io: std.Io,
        max_inflight: usize,
        mutex: std.Io.Mutex = .init,
        pending: std.AutoHashMap(envelope.Correlation, *Pending),
        closed: bool = false,

        /// Create a unary RPC session with an explicit in-flight call bound.
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `max_inflight: usize`: maximum simultaneously pending calls.
        /// Returns (`outcomes.Exit(Self, Error)`): Carries `Self` on `.success` or `Error` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn init(env: *const R, max_inflight: usize) outcomes.Exit(Self, Error) {
            if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator or
                !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
                @compileError("RPC session registry requires allocator and io in R");
            if (max_inflight == 0) return .{ .fail = .capacity_exhausted };
            return .{ .success = .{
                .allocator = env.allocator,
                .io = env.io,
                .max_inflight = max_inflight,
                .pending = std.AutoHashMap(envelope.Correlation, *Pending).init(env.allocator),
            } };
        }

        /// Register a definition or subscription with its owner.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `correlation: envelope.Correlation`: Identifier pairing a request with its response.
        /// - `operation: []const u8`: Typed operation definition selected for this call.
        /// - `version: envelope.Version`: Stable version used to reject incompatible encoded or replayed data.
        /// Returns (`outcomes.Exit(*Pending, Error)`): Carries `*Pending` on `.success` or `Error` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn register(self: *Self, correlation: envelope.Correlation, operation: []const u8, version: envelope.Version) outcomes.Exit(*Pending, Error) {
            if (operation.len == 0 or !std.unicode.utf8ValidateSlice(operation)) return .{ .fail = .unexpected_response };
            const name = self.allocator.dupe(u8, operation) catch return .{ .die = outcomes.Defect.out_of_memory };
            var retain_name = false;
            defer if (!retain_name) self.allocator.free(name);
            const slot = self.allocator.create(Pending) catch return .{ .die = outcomes.Defect.out_of_memory };
            var retain_slot = false;
            defer if (!retain_slot) self.allocator.destroy(slot);
            slot.* = .{ .correlation = correlation, .operation = name, .version = version, .allocator = self.allocator, .response = .{ .io = self.io } };

            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .closed };
            if (self.pending.contains(correlation)) return .{ .fail = .correlation_conflict };
            if (self.pending.count() >= self.max_inflight) return .{ .fail = .capacity_exhausted };
            self.pending.put(correlation, slot) catch return .{ .die = outcomes.Defect.out_of_memory };
            retain_name = true;
            retain_slot = true;
            return .{ .success = slot };
        }

        /// Takes ownership of `response` in every case. An orphan is reported
        /// so the receiver can record a late response without retaining it.
        ///
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `response: envelope.Envelope`: Typed or encoded response being delivered.
        /// Returns (`Delivery`): The publication or delivery status for this message.
        pub fn deliver(self: *Self, response: envelope.Envelope) Delivery {
            var owned = response;
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) {
                owned.deinit();
                return .orphaned;
            }
            const slot = self.pending.get(owned.correlation) orelse {
                owned.deinit();
                return .orphaned;
            };
            const compatible_kind = switch (owned.kind) {
                .response, .response_failure, .response_defect, .response_interrupted => true,
                else => false,
            };
            if (!compatible_kind or owned.version != slot.version or
                !std.mem.eql(u8, owned.operation.items, slot.operation))
            {
                owned.deinit();
                if (!slot.response.complete(.{ .fail = .unexpected_response })) return .duplicate;
                return .conflict;
            }
            if (!slot.response.complete(.{ .success = owned })) {
                owned.deinit();
                return .duplicate;
            }
            return .accepted;
        }

        /// Called only after the owner has stopped awaiting this slot. A late
        /// response is then orphaned rather than delivered into freed storage.
        ///
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `slot: *Pending`: Borrowed pending-operation slot used to track completion.
        /// Returns (`void`): no value.
        pub fn unregister(self: *Self, slot: *Pending) void {
            self.mutex.lockUncancelable(self.io);
            const removed = self.pending.fetchRemove(slot.correlation);
            self.mutex.unlock(self.io);
            std.debug.assert(removed != null and removed.?.value == slot);
            slot.deinit();
        }

        /// Check whether this registry owns the selected correlation.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `correlation: envelope.Correlation`: Identifier pairing a request with its response.
        /// Returns (`bool`): `true` when this registry currently contains the correlation ID.
        pub fn owns(self: *Self, correlation: envelope.Correlation) bool {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            return self.pending.contains(correlation);
        }

        /// Close this scoped resource.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn close(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return;
            self.closed = true;
            var iterator = self.pending.valueIterator();
            while (iterator.next()) |slot| _ = slot.*.response.complete(.{ .fail = .closed });
        }

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            const empty = self.pending.count() == 0;
            self.mutex.unlock(self.io);
            std.debug.assert(empty);
            self.pending.deinit();
            self.* = undefined;
        }
    };
}

/// One suspended provider receive, then bounded frame/envelope decoding and
/// delivery. A caller-owned registry remains alive until this Effect joins.
///
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `select_session: comptime fn (*const R) *const transport.Session(R, ProviderError)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_registry: comptime fn (*const R) *Registry(R)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_policy: comptime fn (*const R) transport.Policy`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// Returns (`effects.Effect(Delivery, ReceiveError(ProviderError), R)`): one framed response dispatched to its pending unary owner.
pub fn receiveOne(
    comptime R: type,
    comptime ProviderError: type,
    comptime select_session: fn (*const R) *const transport.Session(R, ProviderError),
    comptime select_registry: fn (*const R) *Registry(R),
    comptime select_policy: fn (*const R) transport.Policy,
) effects.Effect(Delivery, ReceiveError(ProviderError), R) {
    const Session = transport.Session(R, ProviderError);
    const Failure = ReceiveError(ProviderError);
    const Out = effects.Effect(Delivery, Failure, R);
    const Capture = struct { registry: *Registry(R), policy: transport.Policy };
    return Out.deferEffect(struct {
        fn start(env: *const R) Out {
            const policy = select_policy(env);
            if (!policy.valid()) return Out.fromOwnedOutcome(env, .{ .fail = .{ .session = .invalid_configuration } });
            const session = select_session(env);
            var raw = session.vtable.receive(session.userdata, env);
            var received = raw.mapErrorOwned(env, struct {
                fn map(value: Session.Failure) Failure {
                    return .{ .session = value };
                }
            }.map);
            return received.flatMapCapturedOwned(env, Delivery, Capture{ .registry = select_registry(env), .policy = policy }, struct {
                fn dispatch(capture: *const Capture, value: transport.Received, supplied: *const R) Out {
                    return switch (value) {
                        .closed => Out.fromOwnedOutcome(supplied, .{ .fail = .{ .session = .disconnected } }),
                        .frame => |frame| blk: {
                            var owned = frame;
                            defer owned.deinit();
                            const payload = switch (framing.inspectOne(owned.items, capture.policy.frame_limits)) {
                                .ok => |bytes| bytes,
                                .err => |problem| break :blk Out.fromOwnedOutcome(supplied, .{ .fail = .{ .protocol = problem } }),
                            };
                            const decoded = envelope.decode(supplied.allocator, payload, capture.policy.envelope_limits);
                            const message = switch (decoded) {
                                .success => |item| item,
                                else => {
                                    const mapped = outcomes.mapError(Failure, outcomes.castFailure(Delivery, decoded), supplied.allocator, struct {
                                        fn map(problem: Protocol) Failure {
                                            return .{ .protocol = problem };
                                        }
                                    }.map);
                                    break :blk Out.fromOwnedOutcome(supplied, mapped);
                                },
                            };
                            break :blk Out.fromOwnedSuccess(supplied, capture.registry.deliver(message));
                        },
                    };
                }
            }.dispatch);
        }
    }.start);
}

/// A separate bounded registry keeps streaming responses distinct from unary
/// Deferred values. One pending call owns a bounded frame queue and sequence.
///
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): bounded registry for pending streaming responses and their frames.
pub fn StreamRegistry(comptime R: type) type {
    return struct {
        const Self = @This();
        /// `StreamError` cases: `closed`, `capacity_exhausted`, `correlation_conflict`,
        /// `unexpected_response`, `stream_sequence`, `pending_full`, `concurrent_pull`.
        pub const StreamError = enum {
            closed,
            capacity_exhausted,
            correlation_conflict,
            unexpected_response,
            stream_sequence,
            pending_full,
            concurrent_pull,
        };
        /// Owned record of a streaming response awaiting frames.
        pub const Pending = struct {
            correlation: envelope.Correlation,
            operation: []u8,
            version: envelope.Version,
            allocator: std.mem.Allocator,
            queue: primitives.Queue(envelope.Envelope),
            expected: envelope.Sequence = 0,
            terminal: bool = false,
            pulling: std.atomic.Value(bool) = .init(false),

            fn queueFailure(_: primitives.QueueError) StreamError {
                return .closed;
            }

            /// Take one item or permit from the selected capability.
            /// Parameters:
            /// - `self: *Pending`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
            /// Returns (`effects.Effect(envelope.Envelope, StreamError, R)`): next pending streaming response frame.
            pub fn take(self: *Pending) effects.Effect(envelope.Envelope, StreamError, R) {
                return effects.Effect(envelope.Envelope, StreamError, R).fromNativeCancellableBorrowed(Pending, self, struct {
                    fn run(borrowed: *const Pending, _: *const R, token: ?*const Cancellation) outcomes.Exit(envelope.Envelope, StreamError) {
                        const pending = @constCast(borrowed);
                        if (pending.pulling.cmpxchgStrong(false, true, .acq_rel, .acquire) != null)
                            return .{ .fail = .concurrent_pull };
                        defer pending.pulling.store(false, .release);
                        const taken = pending.queue.takeToken(token);
                        return outcomes.mapError(StreamError, taken, pending.allocator, queueFailure);
                    }
                }.run);
            }

            fn deinit(self: *Pending) void {
                self.queue.shutdown(false);
                self.queue.deinit();
                self.allocator.free(self.operation);
                self.allocator.destroy(self);
            }
        };

        allocator: std.mem.Allocator,
        io: std.Io,
        max_inflight: usize,
        pending_capacity: usize,
        mutex: std.Io.Mutex = .init,
        pending: std.AutoHashMap(envelope.Correlation, *Pending),
        closed: bool = false,

        /// Create a stream RPC session with in-flight and pending bounds.
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `max_inflight: usize`: maximum simultaneously pending calls.
        /// - `pending_capacity: usize`: maximum buffered frames for pending stream calls.
        /// Returns (`outcomes.Exit(Self, StreamError)`): Carries `Self` on `.success` or `StreamError` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn init(env: *const R, max_inflight: usize, pending_capacity: usize) outcomes.Exit(Self, StreamError) {
            if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator or
                !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
                @compileError("RPC stream registry requires allocator and io in R");
            if (max_inflight == 0 or pending_capacity == 0) return .{ .fail = .capacity_exhausted };
            return .{ .success = .{ .allocator = env.allocator, .io = env.io, .max_inflight = max_inflight, .pending_capacity = pending_capacity, .pending = std.AutoHashMap(envelope.Correlation, *Pending).init(env.allocator) } };
        }

        /// Register a definition or subscription with its owner.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `correlation: envelope.Correlation`: Identifier pairing a request with its response.
        /// - `operation: []const u8`: Typed operation definition selected for this call.
        /// - `version: envelope.Version`: Stable version used to reject incompatible encoded or replayed data.
        /// Returns (`outcomes.Exit(*Pending, StreamError)`): Carries `*Pending` on `.success` or `StreamError` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn register(self: *Self, correlation: envelope.Correlation, operation: []const u8, version: envelope.Version) outcomes.Exit(*Pending, StreamError) {
            if (operation.len == 0 or !std.unicode.utf8ValidateSlice(operation))
                return .{ .fail = .unexpected_response };
            const name = self.allocator.dupe(u8, operation) catch return .{ .die = outcomes.Defect.out_of_memory };
            var retain_name = false;
            defer if (!retain_name) self.allocator.free(name);
            const slot = self.allocator.create(Pending) catch return .{ .die = outcomes.Defect.out_of_memory };
            var retain_slot = false;
            defer if (!retain_slot) self.allocator.destroy(slot);
            const provider = struct { allocator: std.mem.Allocator, io: std.Io }{ .allocator = self.allocator, .io = self.io };
            const created = primitives.Queue(envelope.Envelope).init(&provider, self.pending_capacity);
            const queue = switch (created) {
                .success => |value| value,
                .fail => return .{ .fail = .capacity_exhausted },
                else => return outcomes.mapError(StreamError, outcomes.castFailure(*Pending, created), self.allocator, struct {
                    fn map(_: primitives.QueueError) StreamError {
                        return .capacity_exhausted;
                    }
                }.map),
            };
            slot.* = .{ .correlation = correlation, .operation = name, .version = version, .allocator = self.allocator, .queue = queue };
            var retain_queue = false;
            defer if (!retain_queue) slot.queue.deinit();
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .fail = .closed };
            if (self.pending.contains(correlation)) return .{ .fail = .correlation_conflict };
            if (self.pending.count() >= self.max_inflight) return .{ .fail = .capacity_exhausted };
            self.pending.put(correlation, slot) catch return .{ .die = outcomes.Defect.out_of_memory };
            retain_name = true;
            retain_slot = true;
            retain_queue = true;
            return .{ .success = slot };
        }

        /// Takes every envelope. A full pending queue is a protocol failure;
        /// it cannot silently drop a chunk or advance the sequence.
        ///
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `response: envelope.Envelope`: Typed or encoded response being delivered.
        /// Returns (`outcomes.Exit(Delivery, StreamError)`): Carries `Delivery` on `.success` or `StreamError` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn deliver(self: *Self, response: envelope.Envelope) outcomes.Exit(Delivery, StreamError) {
            var owned = response;
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) {
                owned.deinit();
                return .{ .success = .orphaned };
            }
            const slot = self.pending.get(owned.correlation) orelse {
                owned.deinit();
                return .{ .success = .orphaned };
            };
            if (owned.version != slot.version or !std.mem.eql(u8, owned.operation.items, slot.operation) or slot.terminal) {
                owned.deinit();
                return .{ .fail = .unexpected_response };
            }
            const terminal = switch (owned.kind) {
                .stream_chunk => false,
                .stream_end, .response_failure, .response_defect, .response_interrupted => true,
                else => {
                    owned.deinit();
                    return .{ .fail = .unexpected_response };
                },
            };
            switch (owned.kind) {
                .stream_chunk, .stream_end => {
                    if (owned.sequence == null or owned.sequence.? != slot.expected or
                        (owned.kind == .stream_chunk and slot.expected == std.math.maxInt(envelope.Sequence)))
                    {
                        owned.deinit();
                        return .{ .fail = .stream_sequence };
                    }
                },
                else => if (owned.sequence != null) {
                    owned.deinit();
                    return .{ .fail = .stream_sequence };
                },
            }
            const offered = slot.queue.tryOfferOwned(&owned);
            switch (offered) {
                .success => |state| switch (state) {
                    .accepted => {},
                    .full => {
                        owned.deinit();
                        return .{ .fail = .pending_full };
                    },
                    .closed => {
                        owned.deinit();
                        return .{ .fail = .closed };
                    },
                },
                else => {
                    owned.deinit();
                    return outcomes.mapError(StreamError, outcomes.castFailure(Delivery, offered), self.allocator, struct {
                        fn map(_: primitives.QueueError) StreamError {
                            return .closed;
                        }
                    }.map);
                },
            }
            if (terminal) slot.terminal = true else slot.expected += 1;
            return .{ .success = .accepted };
        }

        /// Detach this registration from its owner.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `slot: *Pending`: Borrowed pending-operation slot used to track completion.
        /// Returns (`void`): no value.
        pub fn unregister(self: *Self, slot: *Pending) void {
            self.mutex.lockUncancelable(self.io);
            const removed = self.pending.fetchRemove(slot.correlation);
            self.mutex.unlock(self.io);
            std.debug.assert(removed != null and removed.?.value == slot);
            slot.deinit();
        }

        /// Check whether this registry owns the selected correlation.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `correlation: envelope.Correlation`: Identifier pairing a request with its response.
        /// Returns (`bool`): `true` when this stream registry currently contains the correlation ID.
        pub fn owns(self: *Self, correlation: envelope.Correlation) bool {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            return self.pending.contains(correlation);
        }

        /// Close this scoped resource.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn close(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return;
            self.closed = true;
            var iterator = self.pending.valueIterator();
            while (iterator.next()) |slot| slot.*.queue.shutdown(false);
        }

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            const empty = self.pending.count() == 0;
            self.mutex.unlock(self.io);
            std.debug.assert(empty);
            self.pending.deinit();
            self.* = undefined;
        }
    };
}

/// Receive stream one through the selected provider.
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `select_session: comptime fn (*const R) *const transport.Session(R, ProviderError)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_registry: comptime fn (*const R) *StreamRegistry(R)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_policy: comptime fn (*const R) transport.Policy`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// Returns (`effects.Effect(Delivery, ReceiveError(ProviderError), R)`): one framed response dispatched to its pending stream owner.
pub fn receiveStreamOne(
    comptime R: type,
    comptime ProviderError: type,
    comptime select_session: fn (*const R) *const transport.Session(R, ProviderError),
    comptime select_registry: fn (*const R) *StreamRegistry(R),
    comptime select_policy: fn (*const R) transport.Policy,
) effects.Effect(Delivery, ReceiveError(ProviderError), R) {
    const Session = transport.Session(R, ProviderError);
    const Failure = ReceiveError(ProviderError);
    const Out = effects.Effect(Delivery, Failure, R);
    const StreamRegistryType = StreamRegistry(R);
    const Capture = struct { registry: *StreamRegistryType, policy: transport.Policy };
    return Out.deferEffect(struct {
        fn start(env: *const R) Out {
            const policy = select_policy(env);
            if (!policy.valid()) return Out.fromOwnedOutcome(env, .{ .fail = .{ .session = .invalid_configuration } });
            const session = select_session(env);
            var raw = session.vtable.receive(session.userdata, env);
            var received = raw.mapErrorOwned(env, struct {
                fn map(value: Session.Failure) Failure {
                    return .{ .session = value };
                }
            }.map);
            return received.flatMapCapturedOwned(env, Delivery, Capture{ .registry = select_registry(env), .policy = policy }, struct {
                fn dispatch(capture: *const Capture, value: transport.Received, supplied: *const R) Out {
                    return switch (value) {
                        .closed => Out.fromOwnedOutcome(supplied, .{ .fail = .{ .session = .disconnected } }),
                        .frame => |frame| blk: {
                            var owned = frame;
                            defer owned.deinit();
                            const payload = switch (framing.inspectOne(owned.items, capture.policy.frame_limits)) {
                                .ok => |bytes| bytes,
                                .err => |problem| break :blk Out.fromOwnedOutcome(supplied, .{ .fail = .{ .protocol = problem } }),
                            };
                            const decoded = envelope.decode(supplied.allocator, payload, capture.policy.envelope_limits);
                            const message = switch (decoded) {
                                .success => |item| item,
                                else => {
                                    const mapped = outcomes.mapError(Failure, outcomes.castFailure(Delivery, decoded), supplied.allocator, struct {
                                        fn map(problem: Protocol) Failure {
                                            return .{ .protocol = problem };
                                        }
                                    }.map);
                                    break :blk Out.fromOwnedOutcome(supplied, mapped);
                                },
                            };
                            const delivered = capture.registry.deliver(message);
                            const mapped = outcomes.mapError(Failure, delivered, supplied.allocator, struct {
                                fn map(problem: Registry.StreamError) Failure {
                                    return switch (problem) {
                                        .closed => .{ .session = .disconnected },
                                        .capacity_exhausted => .{ .session = .capacity_exhausted },
                                        .correlation_conflict => .{ .protocol = .correlation_conflict },
                                        .unexpected_response => .{ .protocol = .malformed_envelope },
                                        .stream_sequence, .pending_full, .concurrent_pull => .{ .protocol = .stream_sequence },
                                    };
                                }
                            }.map);
                            break :blk Out.fromOwnedOutcome(supplied, mapped);
                        },
                    };
                }
            }.dispatch);
        }
    }.start);
}

/// A single receiver dispatches both unary and streaming replies. Separate
/// receive loops on the same provider could steal one another's frames.
///
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `select_session: comptime fn (*const R) *const transport.Session(R, ProviderError)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_unary: comptime fn (*const R) *Registry(R)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_stream: comptime fn (*const R) *StreamRegistry(R)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_policy: comptime fn (*const R) transport.Policy`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// Returns (`effects.Effect(Delivery, ReceiveError(ProviderError), R)`): one incoming frame dispatched to its matching unary or streaming owner.
pub fn receiveAnyOne(
    comptime R: type,
    comptime ProviderError: type,
    comptime select_session: fn (*const R) *const transport.Session(R, ProviderError),
    comptime select_unary: fn (*const R) *Registry(R),
    comptime select_stream: fn (*const R) *StreamRegistry(R),
    comptime select_policy: fn (*const R) transport.Policy,
) effects.Effect(Delivery, ReceiveError(ProviderError), R) {
    const Session = transport.Session(R, ProviderError);
    const Failure = ReceiveError(ProviderError);
    const Out = effects.Effect(Delivery, Failure, R);
    const Capture = struct {
        unary: *Registry(R),
        stream: *StreamRegistry(R),
        policy: transport.Policy,
    };
    return Out.deferEffect(struct {
        fn start(env: *const R) Out {
            const policy = select_policy(env);
            if (!policy.valid()) return Out.fromOwnedOutcome(env, .{ .fail = .{ .session = .invalid_configuration } });
            const session = select_session(env);
            var incoming = session.vtable.receive(session.userdata, env);
            var mapped = incoming.mapErrorOwned(env, struct {
                fn toFailure(problem: Session.Failure) Failure {
                    return .{ .session = problem };
                }
            }.toFailure);
            const capture: Capture = .{ .unary = select_unary(env), .stream = select_stream(env), .policy = policy };
            return mapped.flatMapCapturedOwned(env, Delivery, capture, struct {
                fn dispatch(targets: *const Capture, value: transport.Received, supplied: *const R) Out {
                    return switch (value) {
                        .closed => Out.fromOwnedOutcome(supplied, .{ .fail = .{ .session = .disconnected } }),
                        .frame => |frame| blk: {
                            var owned = frame;
                            defer owned.deinit();
                            const payload = switch (framing.inspectOne(owned.items, targets.policy.frame_limits)) {
                                .ok => |bytes| bytes,
                                .err => |problem| break :blk Out.fromOwnedOutcome(supplied, .{ .fail = .{ .protocol = problem } }),
                            };
                            const decoded = envelope.decode(supplied.allocator, payload, targets.policy.envelope_limits);
                            const message = switch (decoded) {
                                .success => |item| item,
                                else => break :blk Out.fromOwnedOutcome(supplied, outcomes.mapError(Failure, outcomes.castFailure(Delivery, decoded), supplied.allocator, struct {
                                    fn toFailure(problem: Protocol) Failure {
                                        return .{ .protocol = problem };
                                    }
                                }.toFailure)),
                            };
                            const unary = targets.unary.owns(message.correlation);
                            const stream = targets.stream.owns(message.correlation);
                            if (unary and stream) {
                                var rejected = message;
                                rejected.deinit();
                                break :blk Out.fromOwnedOutcome(supplied, .{ .fail = .{ .protocol = .correlation_conflict } });
                            }
                            if (unary) break :blk Out.fromOwnedSuccess(supplied, targets.unary.deliver(message));
                            if (stream) {
                                const delivered = targets.stream.deliver(message);
                                break :blk Out.fromOwnedOutcome(supplied, outcomes.mapError(Failure, delivered, supplied.allocator, struct {
                                    fn toFailure(problem: StreamRegistry(R).StreamError) Failure {
                                        return switch (problem) {
                                            .closed => .{ .session = .disconnected },
                                            .capacity_exhausted => .{ .session = .capacity_exhausted },
                                            .correlation_conflict => .{ .protocol = .correlation_conflict },
                                            .unexpected_response => .{ .protocol = .malformed_envelope },
                                            .stream_sequence, .pending_full, .concurrent_pull => .{ .protocol = .stream_sequence },
                                        };
                                    }
                                }.toFailure));
                            }
                            var orphan = message;
                            orphan.deinit();
                            break :blk Out.fromOwnedSuccess(supplied, .orphaned);
                        },
                    };
                }
            }.dispatch);
        }
    }.start);
}
