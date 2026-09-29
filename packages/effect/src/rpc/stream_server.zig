//! Typed streaming RPC handler execution over an application-owned session.
//! The surrounding server owns the request scope and routes control envelopes.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const scopes = @import("../scope.zig");
const envelope = @import("envelope.zig");
const framing = @import("framing.zig");
const transport = @import("session.zig");
const protocol = @import("protocol.zig");
const errors = @import("rpc_error.zig");
const defect_wire = @import("internal/defect_wire.zig");
const wire = @import("wire_codec.zig");
const Issue = @import("../schema/schema_issue.zig").Issue;

/// Specialize the framed streaming RPC server for a closed protocol.
/// Parameters:
/// - `handler: comptime anytype`: Compile-time typed callback; its signature is checked at comptime where static.
/// - `request_wire: comptime wire.WireCodec(@TypeOf(handler.operation).Request, @TypeOf(handler).Environment)`: Wire codec for the operation’s request type and environment.
/// - `chunk_wire: comptime wire.WireCodec(@TypeOf(handler.operation).Chunk, @TypeOf(handler).Environment)`: Wire codec for each streaming chunk.
/// - `terminal_wire: comptime wire.WireCodec(@TypeOf(handler.operation).Terminal, @TypeOf(handler).Environment)`: Wire codec for the stream’s terminal event.
/// - `failure_wire: comptime wire.WireCodec(@TypeOf(handler.operation).DomainError, @TypeOf(handler).Environment)`: Wire codec for the operation’s typed failure channel.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `select_session: comptime fn (*const @TypeOf(handler).Environment) *const transport.Session(@TypeOf(handler).Environment, ProviderError)`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// - `select_policy: comptime fn (*const @TypeOf(handler).Environment) transport.Policy`: Compile-time typed selector from the current environment; its result must
///   remain valid through use.
/// Returns (`type`): framed streaming server for a closed protocol and caller-selected admission limits.
pub fn StreamServer(
    comptime handler: anytype,
    comptime request_wire: wire.WireCodec(@TypeOf(handler.operation).Request, @TypeOf(handler).Environment),
    comptime chunk_wire: wire.WireCodec(@TypeOf(handler.operation).Chunk, @TypeOf(handler).Environment),
    comptime terminal_wire: wire.WireCodec(@TypeOf(handler.operation).Terminal, @TypeOf(handler).Environment),
    comptime failure_wire: wire.WireCodec(@TypeOf(handler.operation).DomainError, @TypeOf(handler).Environment),
    comptime ProviderError: type,
    comptime select_session: fn (*const @TypeOf(handler).Environment) *const transport.Session(@TypeOf(handler).Environment, ProviderError),
    comptime select_policy: fn (*const @TypeOf(handler).Environment) transport.Policy,
) type {
    const R = @TypeOf(handler).Environment;
    const Op = @TypeOf(handler.operation);
    comptime {
        if (!@hasDecl(Op, "Chunk") or !@hasDecl(Op, "Terminal"))
            @compileError("StreamServer requires a streaming RPC handler");
        if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator or
            !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
            @compileError("StreamServer requires allocator and io in R");
    }
    return struct {
        const Self = @This();
        /// RPC operation type served by this dispatcher.
        pub const Operation = Op;
        /// Domain-specific error type carried by the operation.
        pub const DomainError = Op.DomainError;
        /// Typed session error variant or error channel described by `transport.Error(ProviderError)`.
        pub const SessionError = transport.Error(ProviderError);
        /// Typed failure channel combining domain and provider errors.
        pub const Failure = errors.Error(DomainError, void, SessionError);
        /// Lazy Effect for unit; success, error, and environment types are fixed by
        /// `effects.Effect(void, Failure, R)`.
        pub const Unit = effects.Effect(void, Failure, R);
        /// Scoped reader for this RPC stream’s event type.
        pub const Reader = effects.StreamReader(Op.Event, DomainError, R);
        /// `DriveResult` cases: `sent`, `awaiting_acknowledgement`, `idle`, `ended`.
        pub const DriveResult = enum { sent, awaiting_acknowledgement, idle, ended };
        /// `Control` cases: `acknowledged`, `cancelled`.
        pub const Control = enum { acknowledged, cancelled };
        /// Lazy Effect for drive; success, error, and environment types are fixed by
        /// `effects.Effect(DriveResult, Failure, R)`.
        pub const Drive = effects.Effect(DriveResult, Failure, R);
        const Bytes = outcomes.OwnedSlice(u8);

        /// Bounded admission gate for concurrent streaming calls.
        pub const Admission = struct {
            io: std.Io,
            limit: usize,
            active: usize = 0,
            mutex: std.Io.Mutex = .init,
            closed: bool = false,

            /// Create a bounded admission gate for concurrent stream calls.
            /// Parameters:
            /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
            ///   capabilities.
            /// Returns (`outcomes.Result(Admission, SessionError)`): An ok/err Result; owned payloads must be released according to their type contract.
            pub fn init(env: *const R) outcomes.Result(Admission, SessionError) {
                const policy = select_policy(env);
                if (!policy.valid()) return .{ .err = .invalid_configuration };
                return .{ .ok = .{ .io = env.io, .limit = policy.max_inflight } };
            }

            fn enter(self: *Admission) outcomes.Result(void, SessionError) {
                self.mutex.lockUncancelable(self.io);
                defer self.mutex.unlock(self.io);
                if (self.closed) return .{ .err = .disconnected };
                if (self.active == self.limit) return .{ .err = .capacity_exhausted };
                self.active += 1;
                return .{ .ok = {} };
            }

            fn leave(self: *Admission) void {
                self.mutex.lockUncancelable(self.io);
                defer self.mutex.unlock(self.io);
                std.debug.assert(self.active != 0);
                self.active -= 1;
            }

            /// Stop the capability and release its retained resources.
            /// Parameters:
            /// - `self: *Admission`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
            /// Returns (`void`): no value.
            pub fn shutdown(self: *Admission) void {
                self.mutex.lockUncancelable(self.io);
                self.closed = true;
                self.mutex.unlock(self.io);
            }

            /// Release resources owned by this value.
            /// Parameters:
            /// - `self: *Admission`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
            /// Returns (`void`): no value.
            pub fn deinit(self: *Admission) void {
                self.mutex.lockUncancelable(self.io);
                const drained = self.active == 0;
                self.mutex.unlock(self.io);
                std.debug.assert(drained);
                self.* = undefined;
            }
        };

        /// One admitted stream call with its correlation identity and gate ownership.
        pub const Call = struct {
            correlation: envelope.Correlation,
            admission: *Admission,
            mutex: std.Io.Mutex = .init,
            window: transport.StreamWindow = .{},
            reader: ?Reader = null,
            request: ?Op.Request = null,
            request_envelope: ?envelope.Envelope = null,
            context_entries: ?[]envelope.View.Entry = null,
            pending: ?outcomes.OwnedSlice(Op.Event) = null,
            pending_index: usize = 0,
            driving: bool = false,

            /// Attach a correlation identity to an admitted stream call.
            /// Parameters:
            /// - `correlation: envelope.Correlation`: Identifier pairing a request with its response.
            /// - `admission: *Admission`: Borrowed admission state for this call.
            /// Returns (`outcomes.Result(Call, SessionError)`): An ok/err Result; owned payloads must be released according to their type contract.
            pub fn init(correlation: envelope.Correlation, admission: *Admission) outcomes.Result(Call, SessionError) {
                return switch (admission.enter()) {
                    .ok => .{ .ok = .{ .correlation = correlation, .admission = admission } },
                    .err => |problem| .{ .err = problem },
                };
            }

            /// Release resources owned by this value.
            /// Parameters:
            /// - `self: *Call`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
            /// Returns (`void`): no value.
            pub fn deinit(self: *Call) void {
                if (self.pending) |*items| items.deinit();
                if (self.reader) |*value| value.deinit();
                if (self.request) |*value| outcomes.discardPayload(value);
                if (self.context_entries) |entries| self.request_envelope.?.allocator.free(entries);
                if (self.request_envelope) |*value| value.deinit();
                self.admission.leave();
                self.* = undefined;
            }

            /// Applied by the caller's envelope router after identity checks.
            /// A `cancelled` result tells the owner to interrupt the drive
            /// fiber and close its request scope.
            ///
            /// Parameters:
            /// - `self: *Call`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
            /// - `message: *const envelope.Envelope`: Borrowed input; keep its owner alive until the operation has copied or
            ///   joined it.
            /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
            ///   capabilities.
            /// Returns (`outcomes.Result(Control, errors.Protocol)`): An ok/err Result; owned payloads must be released according to their type contract.
            pub fn control(self: *Call, message: *const envelope.Envelope, env: *const R) outcomes.Result(Control, errors.Protocol) {
                if (message.correlation != self.correlation or message.version != handler.operation.version or
                    !std.mem.eql(u8, message.operation.items, handler.operation.name) or
                    message.payload.items.len != 0)
                    return .{ .err = .malformed_envelope };
                self.mutex.lockUncancelable(env.io);
                defer self.mutex.unlock(env.io);
                return switch (message.kind) {
                    .acknowledge => if (message.sequence) |sequence| switch (self.window.acknowledge(sequence)) {
                        .ok => .{ .ok = .acknowledged },
                        .err => |problem| .{ .err = problem },
                    } else .{ .err = .stream_sequence },
                    .cancel => blk: {
                        self.window.cancel();
                        break :blk .{ .ok = .cancelled };
                    },
                    else => .{ .err = .malformed_envelope },
                };
            }
        };

        fn domainFailure(problem: DomainError) Failure {
            return .{ .domain = problem };
        }

        fn serialization(problem: Issue) Failure {
            return .{ .serialization = problem };
        }

        fn protocolFailure(problem: errors.Protocol) Failure {
            return .{ .protocol = problem };
        }

        fn sessionFailure(problem: SessionError) Failure {
            return .{ .session = problem };
        }

        const MessageOwner = struct {
            /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements
            /// `deinit` and must be released after use.
            pub const effect_owned_payload = true;
            value: *envelope.Envelope,
            allocator: std.mem.Allocator,

            /// Release resources owned by this value.
            /// Parameters:
            /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
            /// Returns (`void`): no value.
            pub fn deinit(self: *@This()) void {
                self.value.deinit();
                self.allocator.destroy(self.value);
                self.* = undefined;
            }

            /// Duplicate this value; marked owning payloads are cloned, while ordinary
            /// borrowed fields retain their original lifetime.
            /// Parameters:
            /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
            /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
            pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
                const copied = try self.allocator.create(envelope.Envelope);
                errdefer self.allocator.destroy(copied);
                copied.* = try self.value.clone();
                return .{ .value = copied, .allocator = self.allocator };
            }
        };

        /// The request and scope remain owned by the caller until the reader
        /// and its scoped finalizers have joined.
        ///
        /// Parameters:
        /// - `select_request: comptime fn (*const R) *const Op.Request`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_scope: comptime fn (*const R) *scopes.Scope(DomainError)`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_context: comptime fn (*const R) protocol.Context`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`effects.Effect(Reader, Failure, R)`): stream call state admitted under the server’s configured limit.
        pub fn open(
            comptime select_request: fn (*const R) *const Op.Request,
            comptime select_scope: fn (*const R) *scopes.Scope(DomainError),
            comptime select_context: fn (*const R) protocol.Context,
        ) effects.Effect(Reader, Failure, R) {
            const Out = effects.Effect(Reader, Failure, R);
            return Out.deferEffect(struct {
                fn start(env: *const R) Out {
                    const stream = handler.openWith(select_context(env), select_request(env), env);
                    var opening = stream.open(select_scope(env));
                    return opening.mapErrorOwned(env, domainFailure);
                }
            }.start);
        }

        /// Opens the typed handler stream in a caller-owned scope and moves
        /// its reader into the call. The request stays alive until call close.
        ///
        /// Parameters:
        /// - `select_call: comptime fn (*const R) *Call`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_request: comptime fn (*const R) *const Op.Request`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_scope: comptime fn (*const R) *scopes.Scope(DomainError)`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_context: comptime fn (*const R) protocol.Context`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`Unit`): A lazy unit-valued Effect; execution yields a typed Exit with no success payload.
        pub fn openInto(
            comptime select_call: fn (*const R) *Call,
            comptime select_request: fn (*const R) *const Op.Request,
            comptime select_scope: fn (*const R) *scopes.Scope(DomainError),
            comptime select_context: fn (*const R) protocol.Context,
        ) Unit {
            const opened = comptime open(select_request, select_scope, select_context);
            return opened.flatMap(struct {
                fn install(reader: Reader, env: *const R) Unit {
                    const call = select_call(env);
                    call.mutex.lockUncancelable(env.io);
                    defer call.mutex.unlock(env.io);
                    if (call.reader != null or call.window.closed) {
                        var unused = reader;
                        unused.deinit();
                        return Unit.fromOwnedOutcome(env, .{ .fail = protocolFailure(.stream_sequence) });
                    }
                    call.reader = reader;
                    return Unit.fromOwnedSuccess(env, {});
                }
            }.install);
        }

        /// Accepts one complete framed request and retains its decoded
        /// request/metadata for the entire handler stream lifetime.
        ///
        /// Parameters:
        /// - `select_call: comptime fn (*const R) *Call`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_scope: comptime fn (*const R) *scopes.Scope(DomainError)`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `frame: []const u8`: Encoded protocol frame to parse, dispatch, or transmit.
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// Returns (`Unit`): A lazy unit-valued Effect; execution yields a typed Exit with no success payload.
        pub fn acceptFrame(
            comptime select_call: fn (*const R) *Call,
            comptime select_scope: fn (*const R) *scopes.Scope(DomainError),
            frame: []const u8,
            env: *const R,
        ) Unit {
            const policy = select_policy(env);
            if (!policy.valid()) return Unit.fromOwnedOutcome(env, .{ .fail = .{ .session = .invalid_configuration } });
            const payload = switch (framing.inspectOne(frame, policy.frame_limits)) {
                .ok => |bytes| bytes,
                .err => |problem| return Unit.fromOwnedOutcome(env, .{ .fail = protocolFailure(problem) }),
            };
            const decoded = envelope.decode(env.allocator, payload, policy.envelope_limits);
            const message = switch (decoded) {
                .success => |value| value,
                else => return Unit.fromOwnedOutcome(env, outcomes.mapError(Failure, outcomes.castFailure(void, decoded), env.allocator, protocolFailure)),
            };
            if (message.kind != .request or message.version != handler.operation.version or
                !std.mem.eql(u8, message.operation.items, handler.operation.name))
            {
                var unused = message;
                unused.deinit();
                return Unit.fromOwnedOutcome(env, .{ .fail = protocolFailure(.malformed_envelope) });
            }
            const holder = env.allocator.create(envelope.Envelope) catch {
                var unused = message;
                unused.deinit();
                return Unit.die(outcomes.Defect.out_of_memory);
            };
            holder.* = message;
            const capture: MessageOwner = .{ .value = holder, .allocator = env.allocator };
            var decoding = request_wire.decode(&capture.value.payload);
            var mapped = decoding.mapErrorOwned(env, serialization);
            return mapped.flatMapCapturedOwned(env, void, capture, struct {
                fn decodedRequest(owner: *const MessageOwner, request: Op.Request, supplied: *const R) Unit {
                    var owned_request = request;
                    const expected_key = switch (handler.operation.key(&owned_request)) {
                        .ok => |value| value,
                        .err => |problem| {
                            outcomes.discardPayload(&owned_request);
                            return Unit.fromOwnedOutcome(supplied, .{ .fail = protocolFailure(problem) });
                        },
                    };
                    var supplied_key: ?[]const u8 = null;
                    for (owner.value.metadata) |entry| {
                        if (!std.mem.eql(u8, entry.key.items, protocol.idempotency_metadata_key)) continue;
                        if (supplied_key != null) {
                            outcomes.discardPayload(&owned_request);
                            return Unit.fromOwnedOutcome(supplied, .{ .fail = protocolFailure(.invalid_idempotency_key) });
                        }
                        supplied_key = entry.value.items;
                    }
                    if ((expected_key == null) != (supplied_key == null) or
                        (expected_key != null and !std.mem.eql(u8, expected_key.?, supplied_key.?)))
                    {
                        outcomes.discardPayload(&owned_request);
                        return Unit.fromOwnedOutcome(supplied, .{ .fail = protocolFailure(.invalid_idempotency_key) });
                    }
                    const call = select_call(supplied);
                    call.mutex.lockUncancelable(supplied.io);
                    if (call.reader != null or call.request != null or call.request_envelope != null or
                        call.correlation != owner.value.correlation or call.window.closed)
                    {
                        call.mutex.unlock(supplied.io);
                        outcomes.discardPayload(&owned_request);
                        return Unit.fromOwnedOutcome(supplied, .{ .fail = protocolFailure(.correlation_conflict) });
                    }
                    const copied = owner.value.clone() catch {
                        call.mutex.unlock(supplied.io);
                        outcomes.discardPayload(&owned_request);
                        return Unit.die(outcomes.Defect.out_of_memory);
                    };
                    const entries = supplied.allocator.alloc(envelope.View.Entry, copied.metadata.len) catch {
                        var discarded = copied;
                        discarded.deinit();
                        call.mutex.unlock(supplied.io);
                        outcomes.discardPayload(&owned_request);
                        return Unit.die(outcomes.Defect.out_of_memory);
                    };
                    for (copied.metadata, entries) |entry, *target|
                        target.* = .{ .key = entry.key.items, .value = entry.value.items };
                    call.request = owned_request;
                    call.request_envelope = copied;
                    call.context_entries = entries;
                    call.mutex.unlock(supplied.io);
                    var opening = openInto(select_call, struct {
                        fn requestValue(context: *const R) *const Op.Request {
                            return &select_call(context).request.?;
                        }
                    }.requestValue, select_scope, struct {
                        fn contextValue(context: *const R) protocol.Context {
                            const current = select_call(context);
                            return .{ .correlation = current.correlation, .metadata = current.context_entries.? };
                        }
                    }.contextValue);
                    var settled = opening.materializeOwned(supplied);
                    var aligned = settled.mapErrorOwned(supplied, struct {
                        fn impossible(_: void) Failure {
                            unreachable;
                        }
                    }.impossible);
                    return aligned.flatMapCapturedOwned(supplied, void, call, struct {
                        fn finish(current: *const *Call, result: outcomes.Exit(void, Failure), context: *const R) Unit {
                            if (result != .success) {
                                current.*.mutex.lockUncancelable(context.io);
                                if (current.*.request) |*value| outcomes.discardPayload(value);
                                current.*.request = null;
                                if (current.*.context_entries) |owned_entries| context.allocator.free(owned_entries);
                                current.*.context_entries = null;
                                if (current.*.request_envelope) |*value| value.deinit();
                                current.*.request_envelope = null;
                                current.*.mutex.unlock(context.io);
                            }
                            return Unit.fromOwnedOutcome(context, result);
                        }
                    }.finish);
                }
            }.decodedRequest);
        }

        /// Process an RPC acknowledgement or cancellation frame.
        /// Parameters:
        /// - `select_call: comptime fn (*const R) *Call`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `frame: []const u8`: Encoded protocol frame to parse, dispatch, or transmit.
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// Returns (`effects.Effect(Control, Failure, R)`): typed acknowledgement after processing a streaming control frame.
        pub fn acceptControlFrame(
            comptime select_call: fn (*const R) *Call,
            frame: []const u8,
            env: *const R,
        ) effects.Effect(Control, Failure, R) {
            const Out = effects.Effect(Control, Failure, R);
            const policy = select_policy(env);
            if (!policy.valid()) return Out.fromOwnedOutcome(env, .{ .fail = .{ .session = .invalid_configuration } });
            const payload = switch (framing.inspectOne(frame, policy.frame_limits)) {
                .ok => |bytes| bytes,
                .err => |problem| return Out.fromOwnedOutcome(env, .{ .fail = protocolFailure(problem) }),
            };
            const decoded = envelope.decode(env.allocator, payload, policy.envelope_limits);
            var message = switch (decoded) {
                .success => |value| value,
                else => return Out.fromOwnedOutcome(env, outcomes.mapError(Failure, outcomes.castFailure(Control, decoded), env.allocator, protocolFailure)),
            };
            defer message.deinit();
            return switch (select_call(env).control(&message, env)) {
                .ok => |value| Out.fromOwnedSuccess(env, value),
                .err => |problem| Out.fromOwnedOutcome(env, .{ .fail = protocolFailure(problem) }),
            };
        }

        fn released(call: *Call, primary: outcomes.PrimaryOutcome(Failure), env: *const R) Unit {
            call.mutex.lockUncancelable(env.io);
            call.driving = false;
            switch (primary) {
                .succeeded => {},
                .failed => |problem| {
                    if (problem != .domain) call.window.cancel();
                },
                else => call.window.cancel(),
            }
            call.mutex.unlock(env.io);
            return Unit.succeed({});
        }

        fn afterSent(call: *const *Call, _: void, env: *const R) Drive {
            const target = call.*;
            target.mutex.lockUncancelable(env.io);
            target.pending_index += 1;
            if (target.pending) |*items| {
                if (target.pending_index == items.items.len) {
                    items.deinit();
                    target.pending = null;
                    target.pending_index = 0;
                }
            }
            target.mutex.unlock(env.io);
            return Drive.fromOwnedSuccess(env, .sent);
        }

        const Outbound = struct {
            call: *Call,
            kind: envelope.Kind,
            sequence: envelope.Sequence,
        };

        fn afterEncoded(outbound: *const Outbound, payload: Bytes, env: *const R) Drive {
            var sent = pack(outbound.kind, outbound.call, outbound.sequence, payload, env);
            return sent.flatMapCapturedOwned(env, DriveResult, outbound.call, afterSent);
        }

        fn emit(call: *Call, env: *const R) Drive {
            const pending = call.pending orelse return Drive.fromOwnedSuccess(env, .idle);
            const event = &pending.items[call.pending_index];
            call.mutex.lockUncancelable(env.io);
            const sequence = switch (event.*) {
                .chunk => switch (call.window.beginChunk()) {
                    .ok => |value| value,
                    .err => |problem| {
                        call.mutex.unlock(env.io);
                        return Drive.fromOwnedOutcome(env, .{ .fail = protocolFailure(problem) });
                    },
                },
                .terminal => switch (call.window.finish()) {
                    .ok => |value| value,
                    .err => |problem| {
                        call.mutex.unlock(env.io);
                        return Drive.fromOwnedOutcome(env, .{ .fail = protocolFailure(problem) });
                    },
                },
            };
            call.mutex.unlock(env.io);
            const outbound: Outbound = .{ .call = call, .kind = if (event.* == .chunk) .stream_chunk else .stream_end, .sequence = sequence };
            return switch (event.*) {
                .chunk => blk: {
                    var encoded = chunk_wire.encode(&event.chunk);
                    var mapped = encoded.mapErrorOwned(env, serialization);
                    break :blk mapped.flatMapCapturedOwned(env, DriveResult, outbound, afterEncoded);
                },
                .terminal => blk: {
                    var encoded = terminal_wire.encode(&event.terminal);
                    var mapped = encoded.mapErrorOwned(env, serialization);
                    break :blk mapped.flatMapCapturedOwned(env, DriveResult, outbound, afterEncoded);
                },
            };
        }

        fn receivedChunk(call: *const *Call, chunk: effects.Chunk(Op.Event), env: *const R) Drive {
            var owned = chunk;
            defer owned.deinit();
            if (owned == .end) {
                call.*.mutex.lockUncancelable(env.io);
                const finished = call.*.window.closed;
                call.*.mutex.unlock(env.io);
                return if (finished) Drive.fromOwnedSuccess(env, .ended) else Drive.fromOwnedOutcome(env, .{ .fail = protocolFailure(.stream_sequence) });
            }
            const values = owned.items.values;
            if (values.len == 0) return Drive.fromOwnedSuccess(env, .idle);
            const policy = select_policy(env);
            if (!policy.valid()) return Drive.fromOwnedOutcome(env, .{ .fail = .{ .session = .invalid_configuration } });
            if (values.len > policy.max_pending_frames)
                return Drive.fromOwnedOutcome(env, .{ .fail = protocolFailure(.frame_too_large) });
            for (values, 0..) |value, index| {
                if (value == .terminal and index + 1 != values.len)
                    return Drive.fromOwnedOutcome(env, .{ .fail = protocolFailure(.stream_sequence) });
            }
            const copied = env.allocator.alloc(Op.Event, values.len) catch return Drive.die(outcomes.Defect.out_of_memory);
            var count: usize = 0;
            for (values) |value| {
                copied[count] = value.clone() catch {
                    for (copied[0..count]) |*saved| saved.deinit();
                    env.allocator.free(copied);
                    return Drive.die(outcomes.Defect.out_of_memory);
                };
                count += 1;
            }
            call.*.pending = .{ .items = copied, .allocator = env.allocator };
            call.*.pending_index = 0;
            return emit(call.*, env);
        }

        fn drive(call: *Call, env: *const R) Drive {
            call.mutex.lockUncancelable(env.io);
            const closed = call.window.closed;
            const waiting = call.window.awaiting_acknowledgement != null;
            const has_pending = call.pending != null;
            const reader = call.reader;
            call.mutex.unlock(env.io);
            if (closed) return Drive.fromOwnedSuccess(env, .ended);
            if (waiting) return Drive.fromOwnedSuccess(env, .awaiting_acknowledgement);
            if (has_pending) return emit(call, env);
            if (reader == null) return Drive.fromOwnedOutcome(env, .{ .fail = protocolFailure(.stream_sequence) });
            var pulled = reader.?.pull.borrow();
            var mapped = pulled.mapErrorOwned(env, domainFailure);
            return mapped.flatMapCapturedOwned(env, DriveResult, call, receivedChunk);
        }

        /// Advances one handler event. The caller routes acknowledgement and
        /// cancellation envelopes to `Call.control` between advances.
        ///
        /// Parameters:
        /// - `select_call: comptime fn (*const R) *Call`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`Drive`): The typed operation result or description selected by this API.
        pub fn driveOne(comptime select_call: fn (*const R) *Call) Drive {
            const Acquire = effects.Effect(*Call, Failure, R);
            const entered = Acquire.fromOutcome(struct {
                fn enter(env: *const R) outcomes.Exit(*Call, Failure) {
                    const call = select_call(env);
                    call.mutex.lockUncancelable(env.io);
                    defer call.mutex.unlock(env.io);
                    if (call.driving) return .{ .fail = protocolFailure(.stream_sequence) };
                    call.driving = true;
                    return .{ .success = call };
                }
            }.enter);
            return entered.acquireRelease(drive, released);
        }

        const DomainOwner = struct {
            /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements
            /// `deinit` and must be released after use.
            pub const effect_owned_payload = true;
            value: *DomainError,
            allocator: std.mem.Allocator,
            call: *Call,

            /// Release resources owned by this value.
            /// Parameters:
            /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
            /// Returns (`void`): no value.
            pub fn deinit(self: *@This()) void {
                outcomes.discardPayload(self.value);
                self.allocator.destroy(self.value);
                self.* = undefined;
            }

            /// Duplicate this value; marked owning payloads are cloned, while ordinary
            /// borrowed fields retain their original lifetime.
            /// Parameters:
            /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
            /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
            pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
                const copied = try self.allocator.create(DomainError);
                errdefer self.allocator.destroy(copied);
                copied.* = try outcomes.clonePayload(self.value.*);
                return .{ .value = copied, .allocator = self.allocator, .call = self.call };
            }
        };

        fn ended(_: void, env: *const R) Drive {
            return Drive.fromOwnedSuccess(env, .ended);
        }

        fn sendDomainValue(call: *Call, value: DomainError, env: *const R) Drive {
            const holder = env.allocator.create(DomainError) catch {
                var discarded = value;
                outcomes.discardPayload(&discarded);
                return Drive.die(outcomes.Defect.out_of_memory);
            };
            holder.* = value;
            call.mutex.lockUncancelable(env.io);
            call.window.cancel();
            call.mutex.unlock(env.io);
            const capture: DomainOwner = .{ .value = holder, .allocator = env.allocator, .call = call };
            var encoded = failure_wire.encode(holder);
            var mapped = encoded.mapErrorOwned(env, serialization);
            var response = mapped.flatMapCapturedOwned(env, void, capture, struct {
                fn packFailure(owner: *const DomainOwner, bytes: Bytes, supplied: *const R) Unit {
                    return pack(.response_failure, owner.call, null, bytes, supplied);
                }
            }.packFailure);
            return response.flatMapOwned(env, ended);
        }

        fn sendDefectValue(call: *Call, defect: outcomes.Defect, env: *const R) Drive {
            call.mutex.lockUncancelable(env.io);
            call.window.cancel();
            call.mutex.unlock(env.io);
            const policy = select_policy(env);
            if (!policy.valid()) return Drive.fromOwnedOutcome(env, .{ .fail = .{ .session = .invalid_configuration } });
            const encoded = defect_wire.encode(env, defect, policy.envelope_limits.max_payload_bytes);
            return switch (encoded) {
                .success => |bytes| blk: {
                    var response = pack(.response_defect, call, null, bytes, env);
                    break :blk response.flatMapOwned(env, ended);
                },
                else => Drive.fromOwnedOutcome(env, outcomes.mapError(Failure, outcomes.castFailure(DriveResult, encoded), env.allocator, protocolFailure)),
            };
        }

        fn sendInterruptedValue(call: *Call, env: *const R) Drive {
            call.mutex.lockUncancelable(env.io);
            call.window.cancel();
            call.mutex.unlock(env.io);
            const payload: Bytes = .{ .items = env.allocator.alloc(u8, 0) catch
                return Drive.die(outcomes.Defect.out_of_memory), .allocator = env.allocator };
            var response = pack(.response_interrupted, call, null, payload, env);
            return response.flatMapOwned(env, ended);
        }

        fn respond(call: *const *Call, outcome: outcomes.Exit(DriveResult, Failure), env: *const R) Drive {
            return switch (outcome) {
                .success => |value| Drive.fromOwnedSuccess(env, value),
                .fail => |problem| switch (problem) {
                    .domain => |value| sendDomainValue(call.*, value, env),
                    else => Drive.fromOwnedOutcome(env, .{ .fail = problem }),
                },
                .die => |defect| sendDefectValue(call.*, defect, env),
                .interrupted => sendInterruptedValue(call.*, env),
                .finalization_failed => |details| blk: {
                    var discarded: outcomes.Exit(DriveResult, Failure) = .{ .finalization_failed = details };
                    discarded.deinit();
                    break :blk sendDefectValue(call.*, .{ .kind = .explicit, .message = @import("../internal/diagnostics.zig").rpc_handler_finalization_failed }, env);
                },
            };
        }

        /// Executes one stream step and maps the handler's complete typed Exit
        /// onto the streaming wire protocol before returning to the caller.
        ///
        /// Parameters:
        /// - `select_call: comptime fn (*const R) *Call`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// Returns (`Drive`): The typed operation result or description selected by this API.
        pub fn serveStep(comptime select_call: fn (*const R) *Call, env: *const R) Drive {
            var step = driveOne(select_call);
            var materialized = step.materializeOwned(env);
            var aligned = materialized.mapErrorOwned(env, struct {
                fn impossible(_: void) Failure {
                    unreachable;
                }
            }.impossible);
            return aligned.flatMapCapturedOwned(env, DriveResult, select_call(env), respond);
        }

        const SentFrame = struct {
            /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements
            /// `deinit` and must be released after use.
            pub const effect_owned_payload = true;
            frame: Bytes,

            /// Release resources owned by this value.
            /// Parameters:
            /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
            /// Returns (`void`): no value.
            pub fn deinit(self: *@This()) void {
                self.frame.deinit();
                self.* = undefined;
            }

            /// Duplicate this value; marked owning payloads are cloned, while ordinary
            /// borrowed fields retain their original lifetime.
            /// Parameters:
            /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
            /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
            pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
                return .{ .frame = try self.frame.clone() };
            }
        };

        fn afterSend(_: *const SentFrame, _: transport.SendAcknowledgement, env: *const R) Unit {
            return Unit.fromOwnedSuccess(env, {});
        }

        fn sendFrame(frame: Bytes, env: *const R) Unit {
            const session = select_session(env);
            var sending = session.vtable.send(session.userdata, env, frame.items);
            var mapped = sending.mapErrorOwned(env, sessionFailure);
            return mapped.flatMapCapturedOwned(env, void, SentFrame{ .frame = frame }, afterSend);
        }

        fn pack(comptime kind: envelope.Kind, call: *Call, sequence: ?envelope.Sequence, payload: Bytes, env: *const R) Unit {
            var owned = payload;
            defer owned.deinit();
            const policy = select_policy(env);
            if (!policy.valid()) return Unit.fromOwnedOutcome(env, .{ .fail = .{ .session = .invalid_configuration } });
            const body = envelope.encode(env.allocator, .{ .version = handler.operation.version, .kind = kind, .correlation = call.correlation, .operation = handler.operation.name, .sequence = sequence, .metadata = &.{}, .payload = owned.items }, policy.envelope_limits);
            var encoded = switch (body) {
                .success => |value| value,
                else => return Unit.fromOwnedOutcome(env, outcomes.mapError(Failure, outcomes.castFailure(void, body), env.allocator, protocolFailure)),
            };
            defer encoded.deinit();
            const framed = framing.encodeOne(env, encoded.items, policy.frame_limits);
            return switch (framed) {
                .success => |frame| sendFrame(frame, env),
                else => Unit.fromOwnedOutcome(env, outcomes.mapError(Failure, outcomes.castFailure(void, framed), env.allocator, protocolFailure)),
            };
        }

        fn sendPayload(
            comptime T: type,
            comptime codec: wire.WireCodec(T, R),
            comptime kind: envelope.Kind,
            comptime select_call: fn (*const R) *Call,
            comptime select_value: fn (*const R) *const T,
        ) Unit {
            return Unit.deferEffect(struct {
                fn start(env: *const R) Unit {
                    const call = select_call(env);
                    call.mutex.lockUncancelable(env.io);
                    const sequence: ?envelope.Sequence = switch (kind) {
                        .stream_chunk => switch (call.window.beginChunk()) {
                            .ok => |value| value,
                            .err => |problem| {
                                call.mutex.unlock(env.io);
                                return Unit.fromOwnedOutcome(env, .{ .fail = protocolFailure(problem) });
                            },
                        },
                        .stream_end => switch (call.window.finish()) {
                            .ok => |value| value,
                            .err => |problem| {
                                call.mutex.unlock(env.io);
                                return Unit.fromOwnedOutcome(env, .{ .fail = protocolFailure(problem) });
                            },
                        },
                        .response_failure => blk: {
                            if (call.window.closed) {
                                call.mutex.unlock(env.io);
                                return Unit.fromOwnedOutcome(env, .{ .fail = protocolFailure(.stream_sequence) });
                            }
                            call.window.cancel();
                            break :blk null;
                        },
                        else => null,
                    };
                    call.mutex.unlock(env.io);
                    var encoding = codec.encode(select_value(env));
                    var mapped = encoding.mapErrorOwned(env, serialization);
                    const Capture = struct { call: *Call, sequence: ?envelope.Sequence };
                    return mapped.flatMapCapturedOwned(env, void, Capture{ .call = call, .sequence = sequence }, struct {
                        fn complete(capture: *const Capture, bytes: Bytes, supplied: *const R) Unit {
                            return pack(kind, capture.call, capture.sequence, bytes, supplied);
                        }
                    }.complete);
                }
            }.start);
        }

        /// Send one typed streaming RPC chunk.
        /// Parameters:
        /// - `select_call: comptime fn (*const R) *Call`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_chunk: comptime fn (*const R) *const Op.Chunk`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`Unit`): A lazy unit-valued Effect; execution yields a typed Exit with no success payload.
        pub fn sendChunk(comptime select_call: fn (*const R) *Call, comptime select_chunk: fn (*const R) *const Op.Chunk) Unit {
            return sendPayload(Op.Chunk, chunk_wire, .stream_chunk, select_call, select_chunk);
        }

        /// Send the terminal streaming RPC value.
        /// Parameters:
        /// - `select_call: comptime fn (*const R) *Call`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_terminal: comptime fn (*const R) *const Op.Terminal`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`Unit`): A lazy unit-valued Effect; execution yields a typed Exit with no success payload.
        pub fn sendTerminal(comptime select_call: fn (*const R) *Call, comptime select_terminal: fn (*const R) *const Op.Terminal) Unit {
            return sendPayload(Op.Terminal, terminal_wire, .stream_end, select_call, select_terminal);
        }

        /// Send a typed streaming RPC domain failure.
        /// Parameters:
        /// - `select_call: comptime fn (*const R) *Call`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_failure: comptime fn (*const R) *const DomainError`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`Unit`): A lazy unit-valued Effect; execution yields a typed Exit with no success payload.
        pub fn sendFailure(comptime select_call: fn (*const R) *Call, comptime select_failure: fn (*const R) *const DomainError) Unit {
            return sendPayload(DomainError, failure_wire, .response_failure, select_call, select_failure);
        }

        /// Send an unexpected streaming RPC defect.
        /// Parameters:
        /// - `select_call: comptime fn (*const R) *Call`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_defect: comptime fn (*const R) outcomes.Defect`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`Unit`): A lazy unit-valued Effect; execution yields a typed Exit with no success payload.
        pub fn sendDefect(comptime select_call: fn (*const R) *Call, comptime select_defect: fn (*const R) outcomes.Defect) Unit {
            return Unit.deferEffect(struct {
                fn start(env: *const R) Unit {
                    const call = select_call(env);
                    call.mutex.lockUncancelable(env.io);
                    if (call.window.closed) {
                        call.mutex.unlock(env.io);
                        return Unit.fromOwnedOutcome(env, .{ .fail = protocolFailure(.stream_sequence) });
                    }
                    call.window.cancel();
                    call.mutex.unlock(env.io);
                    const policy = select_policy(env);
                    if (!policy.valid())
                        return Unit.fromOwnedOutcome(env, .{ .fail = .{ .session = .invalid_configuration } });
                    const encoded = defect_wire.encode(env, select_defect(env), policy.envelope_limits.max_payload_bytes);
                    return switch (encoded) {
                        .success => |payload| pack(.response_defect, call, null, payload, env),
                        else => Unit.fromOwnedOutcome(env, outcomes.mapError(Failure, outcomes.castFailure(void, encoded), env.allocator, protocolFailure)),
                    };
                }
            }.start);
        }

        /// Send an interrupted streaming RPC outcome.
        /// Parameters:
        /// - `select_call: comptime fn (*const R) *Call`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`Unit`): A lazy unit-valued Effect; execution yields a typed Exit with no success payload.
        pub fn sendInterrupted(comptime select_call: fn (*const R) *Call) Unit {
            return Unit.deferEffect(struct {
                fn start(env: *const R) Unit {
                    const call = select_call(env);
                    call.mutex.lockUncancelable(env.io);
                    if (call.window.closed) {
                        call.mutex.unlock(env.io);
                        return Unit.fromOwnedOutcome(env, .{ .fail = protocolFailure(.stream_sequence) });
                    }
                    call.window.cancel();
                    call.mutex.unlock(env.io);
                    const payload: Bytes = .{ .items = env.allocator.alloc(u8, 0) catch
                        return Unit.die(outcomes.Defect.out_of_memory), .allocator = env.allocator };
                    return pack(.response_interrupted, call, null, payload, env);
                }
            }.start);
        }
    };
}
