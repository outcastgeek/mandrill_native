//! Encoded unary dispatch is private to RPC. Public handlers remain typed;
//! request and response codecs are selected with the operation at comptime.
const std = @import("std");
const outcomes = @import("../../outcome.zig");
const effects = @import("../../effect.zig");
const protocol = @import("../protocol.zig");
const envelope = @import("../envelope.zig");
const framing = @import("../framing.zig");
const transport = @import("../session.zig");
const wire = @import("../wire_codec.zig");
const errors = @import("../rpc_error.zig");
const defect_wire = @import("defect_wire.zig");
const Issue = @import("../../schema/schema_issue.zig").Issue;

fn Box(comptime T: type) type {
    return struct {
        pub const effect_owned_payload = true;
        allocator: std.mem.Allocator,
        value: *T,

        fn init(allocator: std.mem.Allocator, value: T) error{OutOfMemory}!@This() {
            const pointer = allocator.create(T) catch {
                var abandoned = value;
                outcomes.discardPayload(&abandoned);
                return error.OutOfMemory;
            };
            pointer.* = value;
            return .{ .allocator = allocator, .value = pointer };
        }

        pub fn deinit(self: *@This()) void {
            outcomes.discardPayload(self.value);
            self.allocator.destroy(self.value);
            self.* = undefined;
        }

        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            return init(self.allocator, try outcomes.clonePayload(self.value.*));
        }
    };
}

/// `handler.invokeWith` borrows the decoded request and envelope metadata
/// until its Effect joins. The stable heap address survives suspended work.
fn Invocation(comptime Request: type) type {
    return struct {
        pub const effect_owned_payload = true;
        const Self = @This();
        const Inner = struct {
            request: Request,
            message: envelope.Envelope,
            entries: []envelope.View.Entry,
            context: protocol.Context,
        };

        allocator: std.mem.Allocator,
        inner: *Inner,

        fn init(allocator: std.mem.Allocator, request: Request, source: *const envelope.Envelope) error{OutOfMemory}!Self {
            var owned_request = request;
            errdefer outcomes.discardPayload(&owned_request);
            var message = try source.clone();
            errdefer message.deinit();
            const entries = try allocator.alloc(envelope.View.Entry, message.metadata.len);
            errdefer allocator.free(entries);
            for (message.metadata, entries) |metadata, *entry| {
                entry.* = .{ .key = metadata.key.items, .value = metadata.value.items };
            }
            const inner = try allocator.create(Inner);
            inner.* = .{ .request = owned_request, .message = message, .entries = entries, .context = .{ .correlation = message.correlation, .metadata = entries } };
            return .{ .allocator = allocator, .inner = inner };
        }

        pub fn deinit(self: *Self) void {
            outcomes.discardPayload(&self.inner.request);
            self.inner.message.deinit();
            self.allocator.free(self.inner.entries);
            self.allocator.destroy(self.inner);
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return init(self.allocator, try outcomes.clonePayload(self.inner.request), &self.inner.message);
        }
    };
}

fn ResponseValue(comptime T: type) type {
    return struct {
        pub const effect_owned_payload = true;
        const Self = @This();
        allocator: std.mem.Allocator,
        correlation: envelope.Correlation,
        value: *T,

        fn init(allocator: std.mem.Allocator, correlation: envelope.Correlation, value: T) error{OutOfMemory}!Self {
            const boxed = try Box(T).init(allocator, value);
            return .{ .allocator = boxed.allocator, .correlation = correlation, .value = boxed.value };
        }

        pub fn deinit(self: *Self) void {
            outcomes.discardPayload(self.value);
            self.allocator.destroy(self.value);
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return init(self.allocator, self.correlation, try outcomes.clonePayload(self.value.*));
        }
    };
}

/// A single typed unary registration. A closed server protocol selects one
/// such route by its checked operation identity; no erased business value is
/// exposed to the consumer.
pub fn UnaryRoute(
    comptime handler: anytype,
    comptime request_wire: wire.WireCodec(@TypeOf(handler.operation).Request, @TypeOf(handler).Environment),
    comptime success_wire: wire.WireCodec(@TypeOf(handler.operation).Success, @TypeOf(handler).Environment),
    comptime failure_wire: wire.WireCodec(@TypeOf(handler.operation).DomainError, @TypeOf(handler).Environment),
    comptime ProviderError: type,
    comptime select_policy: fn (*const @TypeOf(handler).Environment) transport.Policy,
) type {
    const H = @TypeOf(handler);
    const R = H.Environment;
    const Op = @TypeOf(handler.operation);
    const DomainError = Op.DomainError;
    const Success = Op.Success;
    const Request = Op.Request;
    comptime {
        if (!@hasDecl(Op, "Success")) @compileError("encoded unary route requires a unary operation");
        if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
            @compileError("encoded RPC route requires allocator in R");
    }
    return struct {
        const Self = @This();
        pub const operation = handler.operation;
        pub const Environment = R;
        pub const Failure = errors.Error(void, void, transport.Error(ProviderError));
        pub const Result = effects.Effect(outcomes.OwnedSlice(u8), Failure, R);
        const Bytes = outcomes.OwnedSlice(u8);
        const MessageOwner = Box(envelope.Envelope);
        const RequestOwner = Invocation(Request);

        fn serialization(problem: Issue) Failure {
            return .{ .serialization = problem };
        }

        fn impossibleHandlerFailure(_: void) Failure {
            unreachable;
        }

        fn protocolFailure(problem: errors.Protocol) Failure {
            return .{ .protocol = problem };
        }

        fn pack(comptime kind: envelope.Kind, correlation: envelope.Correlation, payload: Bytes, env: *const R) Result {
            var owned = payload;
            defer owned.deinit();
            const policy = select_policy(env);
            const encoded = envelope.encode(env.allocator, .{ .version = operation.version, .kind = kind, .correlation = correlation, .operation = operation.name, .metadata = &.{}, .payload = owned.items }, policy.envelope_limits);
            var body = switch (encoded) {
                .success => |value| value,
                .fail => |problem| return Result.fromOwnedOutcome(env, .{ .fail = .{ .protocol = problem } }),
                else => return Result.fromOwnedOutcome(env, outcomes.mapError(Failure, outcomes.castFailure(Bytes, encoded), env.allocator, protocolFailure)),
            };
            defer body.deinit();
            const framed = framing.encodeOne(env, body.items, policy.frame_limits);
            return Result.fromOwnedOutcome(env, outcomes.mapError(Failure, framed, env.allocator, protocolFailure));
        }

        fn encodeValue(
            comptime T: type,
            comptime codec: wire.WireCodec(T, R),
            comptime kind: envelope.Kind,
            correlation: envelope.Correlation,
            value: T,
            env: *const R,
        ) Result {
            const capture = ResponseValue(T).init(env.allocator, correlation, value) catch
                return Result.die(outcomes.Defect.out_of_memory);
            var encoded = codec.encode(capture.value);
            var mapped = encoded.mapErrorOwned(env, serialization);
            return mapped.flatMapCapturedOwned(env, Bytes, capture, struct {
                fn finish(owner: *const ResponseValue(T), payload: Bytes, supplied: *const R) Result {
                    return pack(kind, owner.correlation, payload, supplied);
                }
            }.finish);
        }

        fn encodeDefect(correlation: envelope.Correlation, defect: outcomes.Defect, env: *const R) Result {
            const policy = select_policy(env);
            const payload = defect_wire.encode(env, defect, policy.envelope_limits.max_payload_bytes);
            return switch (payload) {
                .success => |bytes| pack(.response_defect, correlation, bytes, env),
                .fail => |problem| Result.fromOwnedOutcome(env, .{ .fail = .{ .protocol = problem } }),
                else => Result.fromOwnedOutcome(env, outcomes.mapError(Failure, outcomes.castFailure(Bytes, payload), env.allocator, protocolFailure)),
            };
        }

        fn afterHandler(owner: *const RequestOwner, outcome: outcomes.Exit(Success, DomainError), env: *const R) Result {
            const correlation = owner.inner.context.correlation.?;
            return switch (outcome) {
                .success => |value| encodeValue(Success, success_wire, .response, correlation, value, env),
                .fail => |problem| encodeValue(DomainError, failure_wire, .response_failure, correlation, problem, env),
                .die => |defect| encodeDefect(correlation, defect, env),
                .interrupted => pack(.response_interrupted, correlation, .{ .items = env.allocator.alloc(u8, 0) catch return Result.die(outcomes.Defect.out_of_memory), .allocator = env.allocator }, env),
                .finalization_failed => |details| blk: {
                    var abandoned: outcomes.Exit(Success, DomainError) = .{ .finalization_failed = details };
                    abandoned.deinit();
                    break :blk encodeDefect(correlation, .{ .kind = .explicit, .message = @import("../../internal/diagnostics.zig").rpc_handler_finalization_failed }, env);
                },
            };
        }

        fn afterDecode(message: *const MessageOwner, request: Request, env: *const R) Result {
            var owned_request = request;
            const expected_key = switch (operation.key(&owned_request)) {
                .ok => |key| key,
                .err => |problem| {
                    outcomes.discardPayload(&owned_request);
                    return Result.fromOwnedOutcome(env, .{ .fail = .{ .protocol = problem } });
                },
            };
            var supplied_key: ?[]const u8 = null;
            for (message.value.metadata) |entry| {
                if (!std.mem.eql(u8, entry.key.items, protocol.idempotency_metadata_key)) continue;
                if (supplied_key != null) {
                    outcomes.discardPayload(&owned_request);
                    return Result.fromOwnedOutcome(env, .{ .fail = .{ .protocol = .invalid_idempotency_key } });
                }
                supplied_key = entry.value.items;
            }
            if ((expected_key == null) != (supplied_key == null) or
                (expected_key != null and !std.mem.eql(u8, expected_key.?, supplied_key.?)))
            {
                outcomes.discardPayload(&owned_request);
                return Result.fromOwnedOutcome(env, .{ .fail = .{ .protocol = .invalid_idempotency_key } });
            }
            const capture = RequestOwner.init(env.allocator, owned_request, message.value) catch
                return Result.die(outcomes.Defect.out_of_memory);
            var invoked = handler.invokeWith(&capture.inner.context, &capture.inner.request, env);
            var exited = invoked.materializeOwned(env);
            var mapped = exited.mapErrorOwned(env, impossibleHandlerFailure);
            return mapped.flatMapCapturedOwned(env, Bytes, capture, afterHandler);
        }

        /// Takes ownership of a decoded request envelope. The caller's R
        /// supplies allocation, I/O and the selected runtime policy.
        pub fn serve(message: envelope.Envelope, env: *const R) Result {
            var owned = message;
            const policy = select_policy(env);
            if (!policy.valid() or owned.kind != .request or owned.version != operation.version or
                !std.mem.eql(u8, owned.operation.items, operation.name))
            {
                owned.deinit();
                return Result.fromOwnedOutcome(env, .{ .fail = .{ .protocol = .malformed_envelope } });
            }
            const capture = MessageOwner.init(env.allocator, owned) catch return Result.die(outcomes.Defect.out_of_memory);
            var decoded = request_wire.decode(&capture.value.payload);
            var mapped = decoded.mapErrorOwned(env, serialization);
            return mapped.flatMapCapturedOwned(env, Bytes, capture, afterDecode);
        }

        pub fn serveFrame(frame: []const u8, env: *const R) Result {
            const policy = select_policy(env);
            if (!policy.valid()) return Result.fromOwnedOutcome(env, .{ .fail = .{ .protocol = .malformed_envelope } });
            const encoded = switch (framing.inspectOne(frame, policy.frame_limits)) {
                .ok => |body| body,
                .err => |problem| return Result.fromOwnedOutcome(env, .{ .fail = .{ .protocol = problem } }),
            };
            const decoded = envelope.decode(env.allocator, encoded, policy.envelope_limits);
            return switch (decoded) {
                .success => |message| serve(message, env),
                .fail => |problem| Result.fromOwnedOutcome(env, .{ .fail = .{ .protocol = problem } }),
                else => Result.fromOwnedOutcome(env, outcomes.mapError(Failure, outcomes.castFailure(Bytes, decoded), env.allocator, protocolFailure)),
            };
        }
    };
}

/// A closed tuple of typed encoded registrations. Selection is by checked
/// operation identity; every branch returns the same framed-byte Effect.
pub fn EncodedProtocol(
    comptime routes: anytype,
    comptime ProviderError: type,
    comptime select_policy: anytype,
) type {
    const Info = @typeInfo(@TypeOf(routes));
    if (Info != .@"struct" or !Info.@"struct".is_tuple or routes.len == 0)
        @compileError("encoded RPC protocol needs a nonempty tuple of routes");
    const R = routes[0].Environment;
    if (@TypeOf(select_policy) != fn (*const R) transport.Policy)
        @compileError("encoded RPC protocol needs a policy selector from its environment");
    const RouteFailure = errors.Error(void, void, transport.Error(ProviderError));
    inline for (routes, 0..) |Route, index| {
        if (@TypeOf(Route) != type or !@hasDecl(Route, "operation") or
            !@hasDecl(Route, "Environment") or !@hasDecl(Route, "serve"))
            @compileError("encoded RPC protocol members must be typed routes");
        if (Route.Environment != R or Route.Failure != RouteFailure)
            @compileError("encoded RPC routes need one environment and session error type");
        inline for (routes, 0..) |Other, earlier| {
            if (earlier < index and std.mem.eql(u8, Route.operation.name, Other.operation.name) and
                Route.operation.version == Other.operation.version)
                @compileError("duplicate encoded RPC operation identity");
        }
    }
    return struct {
        pub const Environment = R;
        pub const Failure = errors.Error(void, void, transport.Error(ProviderError));
        pub const Result = effects.Effect(outcomes.OwnedSlice(u8), Failure, R);
        const Bytes = outcomes.OwnedSlice(u8);

        fn protocolFailure(problem: errors.Protocol) Failure {
            return .{ .protocol = problem };
        }

        pub fn serve(message: envelope.Envelope, env: *const R) Result {
            var owned = message;
            if (owned.kind != .request) {
                owned.deinit();
                return Result.fromOwnedOutcome(env, .{ .fail = .{ .protocol = .malformed_envelope } });
            }
            var supported: ?envelope.Version = null;
            inline for (routes) |Route| {
                if (std.mem.eql(u8, owned.operation.items, Route.operation.name)) {
                    if (owned.version == Route.operation.version) return Route.serve(owned, env);
                    if (supported == null) supported = Route.operation.version;
                }
            }
            const requested = owned.version;
            owned.deinit();
            if (supported) |version| return Result.fromOwnedOutcome(env, .{ .fail = .{ .protocol = .{ .incompatible_version = .{ .requested = requested, .supported = version } } } });
            return Result.fromOwnedOutcome(env, .{ .fail = .{ .protocol = .unknown_operation } });
        }

        pub fn serveFrame(frame: []const u8, env: *const R) Result {
            const policy = select_policy(env);
            if (!policy.valid()) return Result.fromOwnedOutcome(env, .{ .fail = .{ .session = .invalid_configuration } });
            const encoded = switch (framing.inspectOne(frame, policy.frame_limits)) {
                .ok => |body| body,
                .err => |problem| return Result.fromOwnedOutcome(env, .{ .fail = .{ .protocol = problem } }),
            };
            const decoded = envelope.decode(env.allocator, encoded, policy.envelope_limits);
            return switch (decoded) {
                .success => |message| serve(message, env),
                .fail => |problem| Result.fromOwnedOutcome(env, .{ .fail = .{ .protocol = problem } }),
                else => Result.fromOwnedOutcome(env, outcomes.mapError(Failure, outcomes.castFailure(Bytes, decoded), env.allocator, protocolFailure)),
            };
        }
    };
}
