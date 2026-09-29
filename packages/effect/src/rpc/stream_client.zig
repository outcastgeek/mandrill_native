//! Typed streaming RPC client. A handle owns its registered correlation and
//! retains each decoded frame through the corresponding payload Effect.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const envelope = @import("envelope.zig");
const framing = @import("framing.zig");
const transport = @import("session.zig");
const registry_module = @import("internal/session.zig");
const defect_wire = @import("internal/defect_wire.zig");
const protocol = @import("protocol.zig");
const errors = @import("rpc_error.zig");
const wire = @import("wire_codec.zig");
const Issue = @import("../schema/schema_issue.zig").Issue;

pub fn StreamClient(
    comptime operation: anytype,
    comptime R: type,
    comptime ProviderError: type,
    comptime MiddlewareError: type,
    comptime request_wire: wire.WireCodec(@TypeOf(operation).Request, R),
    comptime chunk_wire: wire.WireCodec(@TypeOf(operation).Chunk, R),
    comptime terminal_wire: wire.WireCodec(@TypeOf(operation).Terminal, R),
    comptime failure_wire: wire.WireCodec(@TypeOf(operation).DomainError, R),
    comptime select_session: fn (*const R) *const transport.Session(R, ProviderError),
    comptime select_registry: fn (*const R) *registry_module.StreamRegistry(R),
    comptime select_policy: fn (*const R) transport.Policy,
    comptime select_correlation: fn (*const R, *const @TypeOf(operation).Request) envelope.Correlation,
    comptime select_metadata: fn (*const R, *const @TypeOf(operation).Request) []const envelope.View.Entry,
) type {
    const Op = @TypeOf(operation);
    comptime {
        if (!@hasDecl(Op, "Chunk") or !@hasDecl(Op, "Terminal") or !@hasDecl(Op, "Event"))
            @compileError("StreamClient requires a streaming RPC operation");
        if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator or
            !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
            @compileError("StreamClient requires allocator and io in R");
    }
    return struct {
        const Self = @This();
        pub const Request = Op.Request;
        pub const Event = Op.Event;
        pub const DomainError = Op.DomainError;
        pub const SessionError = transport.Error(ProviderError);
        pub const Failure = errors.Error(DomainError, MiddlewareError, SessionError);
        pub const Open = effects.Effect(Handle, Failure, R);
        pub const Next = effects.Effect(Event, Failure, R);
        pub const Stream = effects.Stream(Event, Failure, R);
        const StreamPull = effects.Effect(effects.Chunk(Event), Failure, R);
        const Unit = effects.Effect(void, Failure, R);
        const Registry = registry_module.StreamRegistry(R);
        const Pending = Registry.Pending;
        const Bytes = outcomes.OwnedSlice(u8);
        const Registered = struct {
            pending: *Pending,
            registry: *Registry,
            request: *const Request,
            policy: transport.Policy,
        };
        const SendCapture = struct {
            pub const effect_owned_payload = true;
            registered: Registered,
            frame: Bytes,

            pub fn deinit(self: *@This()) void {
                self.frame.deinit();
                self.* = undefined;
            }

            pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
                return .{ .registered = self.registered, .frame = try self.frame.clone() };
            }
        };
        const Shared = struct {
            allocator: std.mem.Allocator,
            registry: *Registry,
            pending: *Pending,
            references: std.atomic.Value(usize) = .init(1),
            mutex: std.Io.Mutex = .init,
            pending_ack: ?envelope.Sequence = null,
            terminal_seen: bool = false,
            cancelled: bool = false,
            pulling: bool = false,
        };

        const ReplyOwner = struct {
            pub const effect_owned_payload = true;
            value: *envelope.Envelope,
            allocator: std.mem.Allocator,
            shared: *Shared,

            pub fn deinit(self: *@This()) void {
                self.value.deinit();
                self.allocator.destroy(self.value);
                self.* = undefined;
            }

            pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
                const copied = try self.allocator.create(envelope.Envelope);
                errdefer self.allocator.destroy(copied);
                copied.* = try self.value.clone();
                return .{ .value = copied, .allocator = self.allocator, .shared = self.shared };
            }
        };

        pub const Handle = struct {
            pub const effect_owned_payload = true;
            shared: *Shared,

            pub fn deinit(self: *Handle) void {
                if (self.shared.references.fetchSub(1, .acq_rel) == 1) {
                    self.shared.registry.unregister(self.shared.pending);
                    self.shared.allocator.destroy(self.shared);
                }
                self.* = undefined;
            }

            pub fn clone(self: *const Handle) error{OutOfMemory}!Handle {
                _ = self.shared.references.fetchAdd(1, .monotonic);
                return self.*;
            }
        };

        fn finishPull(handle: Handle, primary: outcomes.PrimaryOutcome(Failure), env: *const R) effects.Effect(void, Failure, R) {
            handle.shared.mutex.lockUncancelable(env.io);
            handle.shared.pulling = false;
            if (primary != .succeeded) handle.shared.cancelled = true;
            handle.shared.mutex.unlock(env.io);
            const correlation = handle.shared.pending.correlation;
            var owned = handle;
            owned.deinit();
            if (primary != .succeeded) {
                const session = select_session(env);
                var cancelling = session.vtable.cancel(session.userdata, env, correlation);
                return cancelling.mapErrorOwned(env, sessionFailure);
            }
            return effects.Effect(void, Failure, R).succeed({});
        }

        fn pull(handle: Handle, env: *const R) Next {
            handle.shared.mutex.lockUncancelable(env.io);
            const sequence = handle.shared.pending_ack;
            handle.shared.mutex.unlock(env.io);
            if (sequence) |ack| {
                const session = select_session(env);
                var acknowledging = session.vtable.acknowledge(session.userdata, env, handle.shared.pending.correlation, ack);
                var mapped = acknowledging.mapErrorOwned(env, sessionFailure);
                return mapped.flatMapCapturedOwned(env, Event, handle.shared, struct {
                    fn done(captured: *const *Shared, _: void, supplied: *const R) Next {
                        captured.*.mutex.lockUncancelable(supplied.io);
                        captured.*.pending_ack = null;
                        captured.*.mutex.unlock(supplied.io);
                        return takeNext(captured.*, supplied);
                    }
                }.done);
            }
            return takeNext(handle.shared, env);
        }

        fn takeNext(shared: *Shared, env: *const R) Next {
            var awaited = shared.pending.take();
            var mapped = awaited.mapErrorOwned(env, registryFailure);
            return mapped.flatMapCapturedOwned(env, Event, shared, decodeReply);
        }

        fn decodeReply(shared: *const *Shared, reply: envelope.Envelope, env: *const R) Next {
            var owned = reply;
            if (owned.kind != .stream_chunk) {
                shared.*.mutex.lockUncancelable(env.io);
                shared.*.terminal_seen = true;
                shared.*.mutex.unlock(env.io);
            }
            switch (owned.kind) {
                .response_defect => {
                    const decoded = defect_wire.decode(env, owned.payload.items);
                    owned.deinit();
                    return switch (decoded) {
                        .success => |value| Next.fromOwnedOutcome(env, .{ .fail = .{ .remote_defect = value } }),
                        .fail => |problem| Next.fromOwnedOutcome(env, .{ .fail = protocolFailure(problem) }),
                        else => Next.fromOwnedOutcome(env, outcomes.mapError(Failure, outcomes.castFailure(Event, decoded), env.allocator, protocolFailure)),
                    };
                },
                .response_interrupted => {
                    owned.deinit();
                    return Next.fromOwnedOutcome(env, .{ .fail = .remote_interrupted });
                },
                .stream_chunk, .stream_end, .response_failure => {},
                else => {
                    owned.deinit();
                    return Next.fromOwnedOutcome(env, .{ .fail = protocolFailure(.malformed_envelope) });
                },
            }
            const holder = env.allocator.create(envelope.Envelope) catch {
                owned.deinit();
                return Next.die(outcomes.Defect.out_of_memory);
            };
            holder.* = owned;
            const capture: ReplyOwner = .{ .value = holder, .allocator = env.allocator, .shared = shared.* };
            if (holder.kind == .stream_chunk) {
                var decoded = chunk_wire.decode(&holder.payload);
                var mapped = decoded.mapErrorOwned(env, serialization);
                return mapped.flatMapCapturedOwned(env, Event, capture, struct {
                    fn done(owner: *const ReplyOwner, value: Op.Chunk, supplied: *const R) Next {
                        owner.shared.mutex.lockUncancelable(supplied.io);
                        owner.shared.pending_ack = owner.value.sequence;
                        owner.shared.mutex.unlock(supplied.io);
                        return Next.fromOwnedSuccess(supplied, .{ .chunk = value });
                    }
                }.done);
            }
            if (holder.kind == .stream_end) {
                var decoded = terminal_wire.decode(&holder.payload);
                var mapped = decoded.mapErrorOwned(env, serialization);
                return mapped.flatMapCapturedOwned(env, Event, capture, struct {
                    fn done(owner: *const ReplyOwner, value: Op.Terminal, supplied: *const R) Next {
                        owner.shared.mutex.lockUncancelable(supplied.io);
                        owner.shared.terminal_seen = true;
                        owner.shared.mutex.unlock(supplied.io);
                        return Next.fromOwnedSuccess(supplied, .{ .terminal = value });
                    }
                }.done);
            }
            var decoded = failure_wire.decode(&holder.payload);
            var mapped = decoded.mapErrorOwned(env, serialization);
            return mapped.flatMapCapturedOwned(env, Event, capture, struct {
                fn done(_: *const ReplyOwner, value: DomainError, supplied: *const R) Next {
                    return Next.fromOwnedOutcome(supplied, .{ .fail = .{ .domain = value } });
                }
            }.done);
        }

        fn serialization(problem: Issue) Failure {
            return .{ .serialization = problem };
        }

        fn sessionFailure(problem: SessionError) Failure {
            return .{ .session = problem };
        }

        fn protocolFailure(problem: errors.Protocol) Failure {
            return .{ .protocol = problem };
        }

        fn registryFailure(problem: Registry.StreamError) Failure {
            return switch (problem) {
                .closed => .{ .session = .disconnected },
                .capacity_exhausted => .{ .session = .capacity_exhausted },
                .correlation_conflict => .{ .protocol = .correlation_conflict },
                .unexpected_response => .{ .protocol = .malformed_envelope },
                .stream_sequence, .pending_full, .concurrent_pull => .{ .protocol = .stream_sequence },
            };
        }

        fn frameRequest(env: *const R, registered: Registered, payload: []const u8) outcomes.Exit(Bytes, Failure) {
            const application = select_metadata(env, registered.request);
            const idempotency = switch (operation.key(registered.request)) {
                .ok => |key| key,
                .err => |problem| return .{ .fail = .{ .protocol = problem } },
            };
            for (application) |entry| {
                if (std.mem.eql(u8, entry.key, protocol.idempotency_metadata_key))
                    return .{ .fail = .{ .protocol = .malformed_envelope } };
            }
            var metadata: ?[]envelope.View.Entry = null;
            defer if (metadata) |items| env.allocator.free(items);
            const entries: []const envelope.View.Entry = if (idempotency) |key| blk: {
                const count = std.math.add(usize, application.len, 1) catch
                    return .{ .fail = .{ .protocol = .frame_too_large } };
                const items = env.allocator.alloc(envelope.View.Entry, count) catch
                    return .{ .die = outcomes.Defect.out_of_memory };
                std.mem.copyForwards(envelope.View.Entry, items[0..application.len], application);
                items[application.len] = .{ .key = protocol.idempotency_metadata_key, .value = key };
                metadata = items;
                break :blk items;
            } else application;
            const encoded = envelope.encode(env.allocator, .{ .version = operation.version, .kind = .request, .correlation = registered.pending.correlation, .operation = operation.name, .metadata = entries, .payload = payload }, registered.policy.envelope_limits);
            var body = switch (encoded) {
                .success => |value| value,
                .fail => |problem| return .{ .fail = .{ .protocol = problem } },
                else => return outcomes.mapError(Failure, outcomes.castFailure(Bytes, encoded), env.allocator, protocolFailure),
            };
            defer body.deinit();
            return outcomes.mapError(Failure, framing.encodeOne(env, body.items, registered.policy.frame_limits), env.allocator, protocolFailure);
        }

        fn afterSend(capture: *const SendCapture, _: transport.SendAcknowledgement, env: *const R) Open {
            const shared = env.allocator.create(Shared) catch return Open.die(outcomes.Defect.out_of_memory);
            shared.* = .{ .allocator = env.allocator, .registry = capture.registered.registry, .pending = capture.registered.pending };
            return Open.fromOwnedSuccess(env, .{ .shared = shared });
        }

        fn afterEncode(registered: *const Registered, payload: Bytes, env: *const R) Open {
            var owned = payload;
            defer owned.deinit();
            const framed = frameRequest(env, registered.*, owned.items);
            const frame = switch (framed) {
                .success => |value| value,
                else => return Open.fromOwnedOutcome(env, outcomes.castFailure(Handle, framed)),
            };
            const session = select_session(env);
            var sending = session.vtable.send(session.userdata, env, frame.items);
            var mapped = sending.mapErrorOwned(env, sessionFailure);
            return mapped.flatMapCapturedOwned(env, Handle, SendCapture{ .registered = registered.*, .frame = frame }, afterSend);
        }

        fn use(registered: Registered, env: *const R) Open {
            var encoded = request_wire.encode(registered.request);
            var mapped = encoded.mapErrorOwned(env, serialization);
            return mapped.flatMapCapturedOwned(env, Handle, registered, afterEncode);
        }

        fn release(registered: Registered, primary: outcomes.PrimaryOutcome(Failure), _: *const R) effects.Effect(void, Failure, R) {
            if (primary != .succeeded) registered.registry.unregister(registered.pending);
            return effects.Effect(void, Failure, R).succeed({});
        }

        /// Select a live handle from R. The Effect scope holds the pull gate
        /// through acknowledgement, waiting and decoding.
        pub fn next(comptime select_handle: fn (*const R) *Handle) Next {
            const Acquire = effects.Effect(Handle, Failure, R);
            const acquisition = Acquire.fromOutcome(struct {
                fn enter(env: *const R) outcomes.Exit(Handle, Failure) {
                    const handle = select_handle(env);
                    handle.shared.mutex.lockUncancelable(env.io);
                    defer handle.shared.mutex.unlock(env.io);
                    if (handle.shared.cancelled or handle.shared.terminal_seen or handle.shared.pulling)
                        return .{ .fail = protocolFailure(.stream_sequence) };
                    const retained = handle.clone() catch return .{ .die = outcomes.Defect.out_of_memory };
                    handle.shared.pulling = true;
                    return .{ .success = retained };
                }
            }.enter);
            return acquisition.acquireRelease(pull, finishPull);
        }

        pub fn close(comptime select_handle: fn (*const R) *Handle) effects.Effect(void, Failure, R) {
            return Unit.deferEffect(struct {
                fn run(env: *const R) Unit {
                    const handle = select_handle(env);
                    handle.shared.mutex.lockUncancelable(env.io);
                    handle.shared.cancelled = true;
                    handle.shared.mutex.unlock(env.io);
                    const session = select_session(env);
                    var cancelling = session.vtable.cancel(session.userdata, env, handle.shared.pending.correlation);
                    return cancelling.mapErrorOwned(env, sessionFailure);
                }
            }.run);
        }

        /// The request is borrowed until the open Effect completes. The
        /// returned owning handle unregisters the call on its final release.
        pub fn open(comptime select_request: fn (*const R) *const Request) Open {
            const Acquire = effects.Effect(Registered, Failure, R);
            const acquisition = Acquire.fromOutcome(struct {
                fn register(env: *const R) outcomes.Exit(Registered, Failure) {
                    const policy = select_policy(env);
                    if (!policy.valid()) return .{ .fail = .{ .session = .invalid_configuration } };
                    const request = select_request(env);
                    const registry = select_registry(env);
                    const correlation = select_correlation(env, request);
                    const registered = registry.register(correlation, operation.name, operation.version);
                    return switch (registered) {
                        .success => |pending| .{ .success = .{ .pending = pending, .registry = registry, .request = request, .policy = policy } },
                        .fail => |problem| .{ .fail = registryFailure(problem) },
                        else => outcomes.mapError(Failure, outcomes.castFailure(Registered, registered), env.allocator, registryFailure),
                    };
                }
            }.register);
            return acquisition.acquireRelease(use, release);
        }

        fn streamPulled(shared: *const *Shared, outcome: outcomes.Exit(Event, Failure), env: *const R) StreamPull {
            shared.*.mutex.lockUncancelable(env.io);
            shared.*.pulling = false;
            if (outcome != .success) shared.*.cancelled = true;
            shared.*.mutex.unlock(env.io);
            return switch (outcome) {
                .success => |event| blk: {
                    const single = [_]Event{event};
                    const values = env.allocator.dupe(Event, &single) catch {
                        var discarded = event;
                        discarded.deinit();
                        break :blk StreamPull.die(outcomes.Defect.out_of_memory);
                    };
                    break :blk StreamPull.fromOwnedSuccess(env, effects.Chunk(Event).owned(.{ .items = values, .allocator = env.allocator }));
                },
                else => StreamPull.fromOwnedOutcome(env, outcomes.castFailure(effects.Chunk(Event), outcome)),
            };
        }

        fn pullStream(shared: *Shared) StreamPull {
            return StreamPull.deferBorrowed(Shared, shared, struct {
                fn run(borrowed: *const Shared, env: *const R) StreamPull {
                    const current = @constCast(borrowed);
                    current.mutex.lockUncancelable(env.io);
                    if (current.terminal_seen) {
                        current.mutex.unlock(env.io);
                        return StreamPull.fromOwnedSuccess(env, .end);
                    }
                    if (current.cancelled or current.pulling) {
                        current.mutex.unlock(env.io);
                        return StreamPull.fromOwnedOutcome(env, .{ .fail = protocolFailure(.stream_sequence) });
                    }
                    current.pulling = true;
                    current.mutex.unlock(env.io);
                    var next_event = pull(.{ .shared = current }, env);
                    var settled = next_event.materializeOwned(env);
                    var aligned = settled.mapErrorOwned(env, struct {
                        fn impossible(_: void) Failure {
                            unreachable;
                        }
                    }.impossible);
                    return aligned.flatMapCapturedOwned(env, effects.Chunk(Event), current, streamPulled);
                }
            }.run);
        }

        fn releasedStream(shared: *const *Shared, result: outcomes.Exit(void, Failure), env: *const R) Unit {
            var handle: Handle = .{ .shared = shared.* };
            handle.deinit();
            return Unit.fromOwnedOutcome(env, result);
        }

        fn releaseStream(shared: *Shared) Unit {
            return Unit.deferBorrowed(Shared, shared, struct {
                fn run(borrowed: *const Shared, env: *const R) Unit {
                    const current = @constCast(borrowed);
                    current.mutex.lockUncancelable(env.io);
                    const finished = current.terminal_seen;
                    current.cancelled = true;
                    current.mutex.unlock(env.io);
                    var cancellation = if (finished) Unit.fromOwnedSuccess(env, {}) else blk: {
                        const session = select_session(env);
                        var cancelling = session.vtable.cancel(session.userdata, env, current.pending.correlation);
                        break :blk cancelling.mapErrorOwned(env, sessionFailure);
                    };
                    var settled = cancellation.materializeOwned(env);
                    var aligned = settled.mapErrorOwned(env, struct {
                        fn impossible(_: void) Failure {
                            unreachable;
                        }
                    }.impossible);
                    return aligned.flatMapCapturedOwned(env, void, current, releasedStream);
                }
            }.run);
        }

        /// Opens, pulls and cancels through one structured Stream scope. The
        /// request selector and every runtime capability come from R.
        pub fn stream(comptime select_request: fn (*const R) *const Request) Stream {
            const Acquire = effects.Effect(*Shared, Failure, R);
            const opening = comptime open(select_request);
            const acquired = opening.flatMap(struct {
                fn transfer(handle: Handle, _: *const R) Acquire {
                    return Acquire.succeed(handle.shared);
                }
            }.transfer);
            return Stream.fromPull(acquired, pullStream, releaseStream);
        }
    };
}
