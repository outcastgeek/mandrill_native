//! Transport-independent typed unary RPC client. The application supplies
//! session, correlation registry, policy, metadata, and execution through R.
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

pub fn UnaryClient(
    comptime operation: anytype,
    comptime R: type,
    comptime ProviderError: type,
    comptime MiddlewareError: type,
    comptime request_wire: wire.WireCodec(@TypeOf(operation).Request, R),
    comptime success_wire: wire.WireCodec(@TypeOf(operation).Success, R),
    comptime failure_wire: wire.WireCodec(@TypeOf(operation).DomainError, R),
    comptime select_session: fn (*const R) *const transport.Session(R, ProviderError),
    comptime select_registry: fn (*const R) *registry_module.Registry(R),
    comptime select_policy: fn (*const R) transport.Policy,
    comptime select_correlation: fn (*const R, *const @TypeOf(operation).Request) envelope.Correlation,
    comptime select_metadata: fn (*const R, *const @TypeOf(operation).Request) []const envelope.View.Entry,
) type {
    const Op = @TypeOf(operation);
    comptime {
        if (!@hasDecl(Op, "Success") or !@hasDecl(Op, "DomainError"))
            @compileError("UnaryClient requires a unary RPC operation");
        if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator or
            !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
            @compileError("UnaryClient requires allocator and io in R");
    }
    return struct {
        const Self = @This();
        pub const Request = Op.Request;
        pub const Success = Op.Success;
        pub const DomainError = Op.DomainError;
        pub const SessionError = transport.Error(ProviderError);
        pub const Failure = errors.Error(DomainError, MiddlewareError, SessionError);
        pub const Result = effects.Effect(Success, Failure, R);
        const Registry = registry_module.Registry(R);
        const Pending = Registry.Pending;
        const Bytes = outcomes.OwnedSlice(u8);
        const Lease = struct { pending: *Pending, registry: *Registry, request: *const Request, policy: transport.Policy };
        const SendCapture = struct {
            pub const effect_owned_payload = true;
            lease: Lease,
            frame: Bytes,

            pub fn deinit(self: *@This()) void {
                self.frame.deinit();
                self.* = undefined;
            }

            pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
                return .{ .lease = self.lease, .frame = try self.frame.clone() };
            }
        };
        const ReplyOwner = struct {
            pub const effect_owned_payload = true;
            value: *envelope.Envelope,
            allocator: std.mem.Allocator,

            pub fn deinit(self: *@This()) void {
                self.value.deinit();
                self.allocator.destroy(self.value);
                self.* = undefined;
            }

            pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
                const target = try self.allocator.create(envelope.Envelope);
                errdefer self.allocator.destroy(target);
                target.* = try self.value.clone();
                return .{ .value = target, .allocator = self.allocator };
            }
        };

        fn serialization(problem: Issue) Failure {
            return .{ .serialization = problem };
        }

        fn sessionFailure(problem: SessionError) Failure {
            return .{ .session = problem };
        }

        fn protocolFailure(problem: errors.Protocol) Failure {
            return .{ .protocol = problem };
        }

        fn registryFailure(problem: registry_module.Error) Failure {
            return switch (problem) {
                .closed => .{ .session = .disconnected },
                .capacity_exhausted => .{ .session = .capacity_exhausted },
                .correlation_conflict => .{ .protocol = .correlation_conflict },
                .unexpected_response => .{ .protocol = .malformed_envelope },
            };
        }

        fn frameRequest(env: *const R, lease: Lease, payload: []const u8) outcomes.Exit(Bytes, Failure) {
            const application = select_metadata(env, lease.request);
            const idempotency = switch (operation.key(lease.request)) {
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
            const view: envelope.View = .{
                .version = operation.version,
                .kind = .request,
                .correlation = lease.pending.correlation,
                .operation = operation.name,
                .metadata = entries,
                .payload = payload,
            };
            const encoded = envelope.encode(env.allocator, view, lease.policy.envelope_limits);
            var bytes = switch (encoded) {
                .success => |value| value,
                .fail => |problem| return .{ .fail = .{ .protocol = problem } },
                else => return outcomes.mapError(Failure, outcomes.castFailure(Bytes, encoded), env.allocator, protocolFailure),
            };
            defer bytes.deinit();
            const framed = framing.encodeOne(env, bytes.items, lease.policy.frame_limits);
            return switch (framed) {
                .success => |value| .{ .success = value },
                .fail => |problem| .{ .fail = .{ .protocol = problem } },
                else => outcomes.mapError(Failure, outcomes.castFailure(Bytes, framed), env.allocator, protocolFailure),
            };
        }

        fn decodeReply(reply: envelope.Envelope, env: *const R) Result {
            var owned = reply;
            switch (owned.kind) {
                .response_defect => {
                    const decoded = defect_wire.decode(env, owned.payload.items);
                    owned.deinit();
                    return switch (decoded) {
                        .success => |value| Result.fromOwnedOutcome(env, .{ .fail = .{ .remote_defect = value } }),
                        .fail => |problem| Result.fromOwnedOutcome(env, .{ .fail = .{ .protocol = problem } }),
                        else => Result.fromOwnedOutcome(env, outcomes.mapError(Failure, outcomes.castFailure(Success, decoded), env.allocator, protocolFailure)),
                    };
                },
                .response_interrupted => {
                    owned.deinit();
                    return Result.fromOwnedOutcome(env, .{ .fail = .remote_interrupted });
                },
                .response, .response_failure => {},
                else => {
                    owned.deinit();
                    return Result.fromOwnedOutcome(env, .{ .fail = .{ .protocol = .malformed_envelope } });
                },
            }
            const holder = env.allocator.create(envelope.Envelope) catch {
                owned.deinit();
                return Result.die(outcomes.Defect.out_of_memory);
            };
            holder.* = owned;
            const capture: ReplyOwner = .{ .value = holder, .allocator = env.allocator };
            if (holder.kind == .response) {
                var decoded = success_wire.decode(&holder.payload);
                var mapped = decoded.mapErrorOwned(env, serialization);
                return mapped.flatMapCapturedOwned(env, Success, capture, struct {
                    fn done(_: *const ReplyOwner, value: Success, supplied: *const R) Result {
                        return Result.fromOwnedSuccess(supplied, value);
                    }
                }.done);
            }
            var decoded = failure_wire.decode(&holder.payload);
            var mapped = decoded.mapErrorOwned(env, serialization);
            return mapped.flatMapCapturedOwned(env, Success, capture, struct {
                fn done(_: *const ReplyOwner, value: DomainError, supplied: *const R) Result {
                    return Result.fromOwnedOutcome(supplied, .{ .fail = .{ .domain = value } });
                }
            }.done);
        }

        fn awaitReply(lease: Lease, env: *const R) Result {
            var awaited = lease.pending.awaitResponse();
            var mapped = awaited.mapErrorOwned(env, registryFailure);
            return mapped.flatMapOwned(env, decodeReply);
        }

        fn afterSend(capture: *const SendCapture, _: transport.SendAcknowledgement, env: *const R) Result {
            return awaitReply(capture.lease, env);
        }

        fn afterEncode(lease: *const Lease, payload: Bytes, env: *const R) Result {
            var owned = payload;
            defer owned.deinit();
            const framed = frameRequest(env, lease.*, owned.items);
            const frame = switch (framed) {
                .success => |value| value,
                else => return Result.fromOwnedOutcome(env, outcomes.castFailure(Success, framed)),
            };
            const session = select_session(env);
            var sent = session.vtable.send(session.userdata, env, frame.items);
            var mapped = sent.mapErrorOwned(env, sessionFailure);
            return mapped.flatMapCapturedOwned(env, Success, SendCapture{ .lease = lease.*, .frame = frame }, afterSend);
        }

        fn use(lease: Lease, env: *const R) Result {
            var encoded = request_wire.encode(lease.request);
            var mapped = encoded.mapErrorOwned(env, serialization);
            return mapped.flatMapCapturedOwned(env, Success, lease, afterEncode);
        }

        fn release(lease: Lease, _: outcomes.PrimaryOutcome(Failure), _: *const R) effects.Effect(void, Failure, R) {
            lease.registry.unregister(lease.pending);
            return effects.Effect(void, Failure, R).succeed({});
        }

        /// The selected request remains borrowed until this Effect joins.
        /// Registration precedes encoding so an early response is not orphaned.
        pub fn call(comptime select_request: fn (*const R) *const Request) Result {
            const Acquire = effects.Effect(Lease, Failure, R);
            const acquisition = Acquire.fromOutcome(struct {
                fn register(env: *const R) outcomes.Exit(Lease, Failure) {
                    const policy = select_policy(env);
                    if (!policy.valid()) return .{ .fail = .{ .session = .invalid_configuration } };
                    const request = select_request(env);
                    const registry = select_registry(env);
                    const correlation = select_correlation(env, request);
                    const registered = registry.register(correlation, operation.name, operation.version);
                    return switch (registered) {
                        .success => |pending| .{ .success = .{ .pending = pending, .registry = registry, .request = request, .policy = policy } },
                        .fail => |problem| .{ .fail = registryFailure(problem) },
                        else => outcomes.mapError(Failure, outcomes.castFailure(Lease, registered), env.allocator, registryFailure),
                    };
                }
            }.register);
            return acquisition.acquireRelease(use, release);
        }
    };
}
