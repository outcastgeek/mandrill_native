const std = @import("std");
const fx = @import("effect");
const Threads = @import("support/thread_executor.zig").ThreadExecutor;
const Bytes = fx.OwnedSlice(u8);
const Issue = fx.SchemaIssue.Issue;
const Mode = enum { normal, provider_failure, serialization_failure, domain_failure, send_domain_failure, encoded_request, encoded_invalid_request, encoded_invalid_version, receive_failure, multiple_calls, pending_receive, cleanup_failure, cleanup_chain, late_receive };
const Counts = struct {
    session_live: usize = 0,
    registry_live: usize = 0,
    readers: usize = 0,
    reader_closed: usize = 0,
    projected: usize = 0,
    environment_closed: usize = 0,
    released: usize = 0,
    service_closed: usize = 0,
    sent: usize = 0,
    chunk_seen: bool = false,
    terminal_seen: bool = false,
    failure_seen: bool = false,
    request_decoded: usize = 0,
    metadata_seen: bool = false,
    late_frames: usize = 0,
};
const Session = fx.RpcSession(Env, u8);
const SendGate = @import("support/dsl_gate.zig").Gate(void, Session.Failure);
const Root = struct {
    allocator: std.mem.Allocator,
    io: std.Io = std.testing.io,
    async_engine: fx.Executor,
    counts: *Counts,
    mode: Mode = .normal,
    gate: ?*SendGate = null,
};
const Env = struct {
    pub const effect_owned_payload = true;
    allocator: std.mem.Allocator,
    io: std.Io,
    async_engine: fx.Executor,
    state: *State,
    counts: *Counts,
    mode: Mode,
    gate: ?*SendGate,
    pub fn deinit(self: *@This()) void {
        std.debug.assert(self.counts.readers == self.counts.reader_closed);
        std.debug.assert(self.counts.service_closed == 0);
        if (self.gate) |gate| if (gate.started.isSet()) {
            std.debug.assert(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
        };
        self.counts.environment_closed += 1;
    }
};
fn bytes(allocator: std.mem.Allocator, value: []const u8) error{OutOfMemory}!Bytes {
    return .{ .items = try allocator.dupe(u8, value), .allocator = allocator };
}
const CodecEnv = struct { allocator: std.mem.Allocator };
const Schema = struct {
    fn transform(value: *const Bytes) fx.Effect(Bytes, Issue, CodecEnv) {
        return fx.Effect(Bytes, Issue, CodecEnv).fromBorrowed(Bytes, value, struct {
            fn run(input: *const Bytes, env: *const CodecEnv) fx.Exit(Bytes, Issue) {
                return .{ .success = bytes(env.allocator, input.items) catch return .{ .die = fx.Defect.out_of_memory } };
            }
        }.run);
    }
};
const owned_codec = fx.dsl.codec(.{ .kind = .contextual, .decode = Schema.transform, .encode = Schema.transform });
const operation = fx.dsl.rpcStreamDefinition(.{ .name = "owned.serving", .version = 7, .request = fx.JsonCodec.signed_integer, .chunk = owned_codec, .terminal = owned_codec, .failure = owned_codec });
const Event = @TypeOf(operation).Event;
const Stream = fx.Stream(Event, Bytes, Env);
const Pull = fx.Effect(fx.Chunk(Event), Bytes, Env);
const Iterator = struct {
    events: [2]Event,
    request: *const i64,
    context: *const fx.RpcContext,
    expected_request: i64,
    expected_correlation: fx.RpcEnvelope.Correlation,
    configuration: []const u8,
    emitted: bool = false,
};
const Native = struct {
    fn pull(iterator: *Iterator) Pull {
        return Pull.fromBorrowed(Iterator, iterator, struct {
            fn read(current: *const Iterator, env: *const Env) fx.Exit(fx.Chunk(Event), Bytes) {
                std.debug.assert(env.counts.registry_live == 1 and env.counts.session_live == 1);
                std.debug.assert(current.request.* == current.expected_request);
                std.debug.assert(current.context.correlation == current.expected_correlation);
                std.debug.assert(std.mem.eql(u8, current.configuration, "registry configuration"));
                if (env.mode == .domain_failure or env.mode == .send_domain_failure or env.mode == .cleanup_chain)
                    return .{ .fail = bytes(env.allocator, "owned domain failure") catch return .{ .die = fx.Defect.out_of_memory } };
                if (current.emitted) return .{ .success = .end };
                @constCast(current).emitted = true;
                return .{ .success = fx.Chunk(Event).borrowed(&current.events) };
            }
        }.read);
    }
    fn close(iterator: *Iterator) fx.Effect(void, Bytes, Env) {
        return fx.Effect(void, Bytes, Env).fromBorrowed(Iterator, iterator, struct {
            fn release(current: *const Iterator, env: *const Env) fx.Exit(void, Bytes) {
                std.debug.assert(env.counts.registry_live == 1 and env.counts.session_live == 1);
                std.debug.assert(env.counts.environment_closed == 0);
                std.debug.assert(current.request.* == current.expected_request);
                std.debug.assert(current.context.correlation == current.expected_correlation);
                if (current.context.metadata.len != 0) {
                    std.debug.assert(std.mem.eql(u8, current.context.metadata[0].key, "tenant"));
                    std.debug.assert(std.mem.eql(u8, current.context.metadata[0].value, "lifecycle"));
                }
                const mutable = @constCast(current);
                for (&mutable.events) |*event| event.deinit();
                env.allocator.destroy(mutable);
                env.counts.reader_closed += 1;
                if (env.mode == .cleanup_failure or env.mode == .cleanup_chain)
                    return .{ .fail = bytes(env.allocator, "owned reader cleanup") catch return .{ .die = fx.Defect.out_of_memory } };
                return .{ .success = {} };
            }
        }.release);
    }
    fn handle(configuration: anytype, context: *const fx.RpcContext, request: *const i64, current: *const Env) Stream {
        current.state.context = context;
        current.state.request = request;
        current.state.configuration = configuration.owner.value.items;
        return stream();
    }
    fn stream() Stream {
        return Stream.fromPull(fx.Effect(*Iterator, Bytes, Env).fromOutcome(struct {
            fn acquire(env: *const Env) fx.Exit(*Iterator, Bytes) {
                const iterator = env.allocator.create(Iterator) catch return .{ .die = fx.Defect.out_of_memory };
                const payload = bytes(env.allocator, "owned chunk:42") catch {
                    env.allocator.destroy(iterator);
                    return .{ .die = fx.Defect.out_of_memory };
                };
                const terminal = bytes(env.allocator, "owned terminal") catch {
                    var abandoned = payload;
                    abandoned.deinit();
                    env.allocator.destroy(iterator);
                    return .{ .die = fx.Defect.out_of_memory };
                };
                // These expectations come from the fixture's admission order,
                // independently of the borrowed request and context storage.
                const second_call = env.mode == .multiple_calls and env.counts.readers == 1;
                iterator.* = .{
                    .events = .{ .{ .chunk = payload }, .{ .terminal = terminal } },
                    .context = env.state.context.?,
                    .request = env.state.request.?,
                    .expected_request = if (second_call) 29 else 17,
                    .expected_correlation = if (second_call) 44 else 43,
                    .configuration = env.state.configuration.?,
                };
                env.counts.readers += 1;
                return .{ .success = iterator };
            }
        }.acquire), pull, close);
    }
    fn staticHandle(context: fx.RpcContext, request: *const i64, env: *const Env) Stream {
        std.debug.assert(context.metadata.len == 1);
        std.debug.assert(std.mem.eql(u8, context.metadata[0].key, "tenant") and std.mem.eql(u8, context.metadata[0].value, "lifecycle"));
        env.counts.metadata_seen = true;
        env.state.context_value = context;
        env.state.context = &env.state.context_value;
        env.state.request = request;
        env.state.configuration = env.state.native_configuration.?.value.items;
        return stream();
    }
};
fn Wire(comptime T: type) type {
    return struct {
        fn decode(input: *const Bytes) fx.Effect(T, Issue, Env) {
            if (T == Bytes) return fx.Effect(Bytes, Issue, Env).fromBorrowed(Bytes, input, struct {
                fn read(supplied: *const Bytes, env: *const Env) fx.Exit(Bytes, Issue) {
                    return .{ .success = bytes(env.allocator, supplied.items) catch return .{ .die = fx.Defect.out_of_memory } };
                }
            }.read);
            return fx.Effect(T, Issue, Env).fromBorrowed(Bytes, input, struct {
                fn read(supplied: *const Bytes, env: *const Env) fx.Exit(T, Issue) {
                    if (T == void) return .{ .success = {} };
                    const parsed = std.fmt.parseInt(i64, supplied.items, 10) catch return .{ .fail = Issue.custom(env, .decode, "request-integer") };
                    env.counts.request_decoded += 1;
                    return .{ .success = parsed };
                }
            }.read);
        }
        fn encode(input: *const T) fx.Effect(Bytes, Issue, Env) {
            return fx.Effect(Bytes, Issue, Env).fromBorrowed(T, input, struct {
                fn write(supplied: *const T, env: *const Env) fx.Exit(Bytes, Issue) {
                    if (env.mode == .serialization_failure) return .{ .fail = Issue.custom(env, .encode, "serving-fixture") };
                    const output = if (T == Bytes) supplied.items else if (T == void) "null" else "17";
                    return .{ .success = bytes(env.allocator, output) catch return .{ .die = fx.Defect.out_of_memory } };
                }
            }.write);
        }
        const value = fx.RpcWireCodec(T, Env).init(decode, encode);
    };
}
const policy: fx.RpcSessionPolicy = .{ .max_inflight = 1, .max_pending_frames = 2, .frame_limits = .{ .max_frame_bytes = 1024, .max_buffered_bytes = 2048 }, .envelope_limits = .{ .max_envelope_bytes = 1024, .max_operation_bytes = 64, .max_payload_bytes = 512, .max_metadata_entries = 4, .max_metadata_bytes = 128 } };
const Select = struct {
    fn session(env: *const Env) *const Session {
        return &env.state.session;
    }
    fn selectedPolicy(env: *const Env) fx.RpcSessionPolicy {
        var selected = policy;
        if (env.mode == .multiple_calls) selected.max_inflight = 2;
        return selected;
    }
    fn call(env: *const Env) *Server.Call {
        return &env.state.call;
    }
    fn scope(env: *const Env) *fx.Scope(Bytes) {
        return env.state.scope.?;
    }
    fn secondCall(env: *const Env) *Server.Call {
        return &env.state.second_call.?;
    }
};
const Server = fx.RpcStreamServer(operation.handledByContext(Env, Native.staticHandle), Wire(i64).value, Wire(Bytes).value, Wire(Bytes).value, Wire(Bytes).value, u8, Select.session, Select.selectedPolicy);
const State = struct { allocator: std.mem.Allocator, counts: *Counts, admission: Server.Admission, call: Server.Call, second_call: ?Server.Call = null, session: Session, frames: [2]?Bytes = .{ null, null }, context: ?*const fx.RpcContext = null, request: ?*const i64 = null, configuration: ?[]const u8 = null, context_value: fx.RpcContext = fx.RpcContext.empty, native_configuration: ?Owner = null, scope: ?*fx.Scope(Bytes) = null };
const Service = struct {
    pub const effect_owned_payload = true;
    state: *State,
    pub fn deinit(self: *@This()) void {
        std.debug.assert(self.state.counts.environment_closed == self.state.counts.projected);
        self.state.call.deinit();
        if (self.state.second_call) |*call| call.deinit();
        self.state.admission.deinit();
        if (self.state.native_configuration) |*owner| owner.deinit();
        for (&self.state.frames) |*frame| if (frame.*) |*value| value.deinit();
        self.state.counts.service_closed += 1;
        self.state.allocator.destroy(self.state);
    }
};
const Provider = struct {
    const Sending = struct { frame: []const u8 };
    fn send(_: ?*anyopaque, env: *const Env, frame: []const u8) Session.SendEffect {
        const Unit = fx.Effect(void, Session.Failure, Env);
        var wait = if (env.gate != null) Unit.fromAsync(struct {
            fn pending(current: *const Env) fx.Async(void, Session.Failure).Start {
                return current.gate.?.start();
            }
        }.pending) else Unit.succeed({});
        return wait.flatMapCapturedOwned(env, Session.SendEffect.Success, Sending{ .frame = frame }, copied);
    }
    fn copied(stored: *const Sending, _: void, env: *const Env) Session.SendEffect {
        return Session.SendEffect.fromOwned(env, stored.*, struct {
            fn copy(input: *const Sending, current: *const Env) fx.Exit(Session.SendEffect.Success, Session.Failure) {
                std.debug.assert(current.counts.environment_closed == 0 and current.counts.registry_live == 1);
                if (current.mode == .provider_failure) return .{ .fail = .{ .provider = 83 } };
                const saved = bytes(current.allocator, input.frame) catch return .{ .die = fx.Defect.out_of_memory };
                const index = current.counts.sent;
                std.debug.assert(index < current.state.frames.len);
                current.state.frames[index] = saved;
                current.counts.sent += 1;
                return .{ .success = .accepted };
            }
        }.copy);
    }
    fn receive(_: ?*anyopaque, env: *const Env) Session.ReceiveEffect {
        if (env.mode == .late_receive) return Session.ReceiveEffect.fromAsync(LateReceive.start);
        if (env.mode == .pending_receive) {
            const Unit = fx.Effect(void, Session.Failure, Env);
            var wait = Unit.fromAsync(struct {
                fn start(current: *const Env) fx.Async(void, Session.Failure).Start {
                    return current.gate.?.start();
                }
            }.start);
            return wait.flatMapOwned(env, struct {
                fn received(_: void, _: *const Env) Session.ReceiveEffect {
                    return incoming();
                }
            }.received);
        }
        return incoming();
    }
    fn incoming() Session.ReceiveEffect {
        return Session.ReceiveEffect.fromOutcome(readIncoming);
    }
    fn readIncoming(env: *const Env) fx.Exit(Session.ReceiveEffect.Success, Session.Failure) {
        if (env.mode == .receive_failure) return .{ .fail = .{ .provider = 84 } };
        const payload = if (env.mode == .encoded_invalid_request) "invalid" else "17";
        var encoded = fx.RpcEnvelope.encode(env.allocator, .{ .version = if (env.mode == .encoded_invalid_version) 8 else 7, .kind = .request, .correlation = 43, .operation = operation.name, .metadata = &.{.{ .key = "tenant", .value = "lifecycle" }}, .payload = payload }, policy.envelope_limits);
        defer encoded.deinit();
        if (encoded == .die) return .{ .die = encoded.die };
        std.debug.assert(encoded == .success);
        var framer = fx.RpcFramer.init(env, policy.frame_limits).success;
        defer framer.deinit();
        const framed = framer.encode(encoded.success.items);
        return switch (framed) {
            .success => |frame| .{ .success = .{ .frame = frame } },
            .die => |defect| .{ .die = defect },
            else => unreachable,
        };
    }
    fn acknowledge(_: ?*anyopaque, _: *const Env, _: fx.RpcEnvelope.Correlation, _: fx.RpcEnvelope.Sequence) Session.ControlEffect {
        unreachable;
    }
    fn cancel(_: ?*anyopaque, _: *const Env, _: fx.RpcEnvelope.Correlation) Session.ControlEffect {
        unreachable;
    }
    const vtable: Session.VTable = .{ .send = send, .receive = receive, .acknowledge = acknowledge, .cancel = cancel };
};
// A native provider may finish successfully after cancellation was requested.
// The existing async kernel owns rejection/disposal of that late owned frame.
const LateReceive = struct {
    const NativeReceive = fx.Async(Session.ReceiveEffect.Success, Session.Failure);
    const Frame = struct { env: *const Env, inner: fx.Async(void, Session.Failure).Operation };
    fn start(env: *const Env) NativeReceive.Start {
        const frame = env.allocator.create(Frame) catch return .{ .completed = .{ .die = fx.Defect.out_of_memory } };
        frame.* = .{ .env = env, .inner = env.gate.?.start().pending };
        return .{ .pending = .{ .userdata = frame, .vtable = &vtable } };
    }
    fn request(data: *anyopaque) ?fx.Defect {
        const frame: *Frame = @ptrCast(@alignCast(data));
        return frame.inner.request();
    }
    fn join(data: *anyopaque, protected: bool) NativeReceive.JoinResult {
        const frame: *Frame = @ptrCast(@alignCast(data));
        var settled = frame.inner.joinProtected(protected);
        settled.exit.deinit();
        std.debug.assert(frame.env.counts.environment_closed == 0);
        const received = Provider.readIncoming(frame.env);
        if (received == .success) frame.env.counts.late_frames += 1;
        return .{ .exit = received };
    }
    fn destroy(data: *anyopaque) void {
        const frame: *Frame = @ptrCast(@alignCast(data));
        frame.inner.deinit();
        frame.env.allocator.destroy(frame);
    }
    const vtable: NativeReceive.Operation.VTable = .{ .request = request, .join = join, .destroy = destroy };
};
const Owner = struct {
    pub const effect_owned_payload = true;
    value: Bytes,
    live: *usize,
    fn init(allocator: std.mem.Allocator, live: *usize, label: []const u8) !@This() {
        const value = try bytes(allocator, label);
        live.* += 1;
        return .{ .value = value, .live = live };
    }
    pub fn deinit(self: *@This()) void {
        self.value.deinit();
        self.live.* -= 1;
    }
};
const Callbacks = struct {
    fn acquire(_: anytype, _: *const Root) fx.Effect(Service, u8, Root) {
        return fx.Effect(Service, u8, Root).fromOutcome(struct {
            fn allocate(env: *const Root) fx.Exit(Service, u8) {
                const state = env.allocator.create(State) catch return .{ .die = fx.Defect.out_of_memory };
                state.* = .{ .allocator = env.allocator, .counts = env.counts, .admission = undefined, .call = undefined, .session = Session.fromProvider(null, &Provider.vtable) };
                // Both self-references are installed only after State has its final address.
                const current: Env = .{ .allocator = env.allocator, .io = env.io, .async_engine = env.async_engine, .state = state, .counts = env.counts, .mode = env.mode, .gate = env.gate };
                state.admission = Server.Admission.init(&current).ok;
                state.call = Server.Call.init(43, &state.admission).ok;
                if (env.mode == .multiple_calls) state.second_call = Server.Call.init(44, &state.admission).ok;
                return .{ .success = .{ .state = state } };
            }
        }.allocate);
    }
    fn project(_: anytype, service: *const Service, env: *const Root) Env {
        env.counts.projected += 1;
        return .{ .allocator = env.allocator, .io = env.io, .async_engine = env.async_engine, .state = service.state, .counts = env.counts, .mode = env.mode, .gate = env.gate };
    }
    fn domain(value: Bytes) Server.Failure {
        return .{ .domain = value };
    }
    fn use(_: anytype, scope: *fx.Scope(Bytes), env: *const Env) fx.Effect(void, Server.Failure, Env) {
        const owner = Owner.init(env.allocator, &env.counts.registry_live, "registry configuration") catch return Server.Unit.die(fx.Defect.out_of_memory);
        switch (env.mode) {
            .encoded_request, .encoded_invalid_request, .encoded_invalid_version, .receive_failure, .pending_receive, .late_receive => {
                env.state.native_configuration = owner;
                env.state.scope = scope;
                var received = Session.receive(Select.session);
                var mapped = received.mapErrorOwned(env, struct {
                    fn failure(problem: Session.Failure) Server.Failure {
                        return .{ .session = problem };
                    }
                }.failure);
                return mapped.flatMapOwned(env, Encoded.received);
            },
            else => {},
        }
        var context = fx.RpcContext.empty;
        context.correlation = 43;
        return fx.dsl.withRpcRegistry(env, .{ .definition = fx.dsl.declareMetadata(.{ .use = Registry.run }), .inputs = .{ .bindings = .{.{ .definition = fx.dsl.declareMetadata(.{ .operation = operation, .handler = Native.handle }), .inputs = .{ .owner = owner } }}, .scope = scope, .context = context, .request = @as(i64, 17) } });
    }
    fn release(_: anytype, _: *const Service, _: anytype, env: *const Root) fx.Effect(void, u8, Root) {
        std.debug.assert(env.counts.environment_closed == env.counts.projected and env.counts.readers == env.counts.reader_closed);
        env.counts.released += 1;
        if (env.mode == .cleanup_chain) return fx.Effect(void, u8, Root).fail(91);
        return fx.Effect(void, u8, Root).succeed({});
    }
};
const Encoded = struct {
    fn received(message: Session.ReceiveEffect.Success, env: *const Env) Server.Unit {
        var owned = message;
        defer owned.deinit();
        std.debug.assert(owned == .frame);
        var accepted = Server.acceptFrame(Select.call, Select.scope, owned.frame.items, env);
        return accepted.flatMapOwned(env, struct {
            fn start(_: void, current: *const Env) Server.Unit {
                var drive = Server.driveOne(Select.call);
                return drive.flatMapOwned(current, Registry.afterFirst);
            }
        }.start);
    }
};
const Registry = struct {
    fn run(registry: anytype, inputs: anytype, env: *const Env) Server.Unit {
        if (env.mode == .multiple_calls) return MultipleCalls.open(registry, inputs, env);
        var opening = registry.openWith(inputs.scope, operation, inputs.context, inputs.request);
        var mapped = opening.mapErrorOwned(env, Callbacks.domain);
        return mapped.flatMapOwned(env, opened);
    }
    fn opened(reader: Server.Reader, env: *const Env) Server.Unit {
        env.state.call.reader = reader;
        var first = if (env.mode == .send_domain_failure) Server.serveStep(Select.call, env) else Server.driveOne(Select.call);
        return first.flatMapOwned(env, afterFirst);
    }
    fn inspect(env: *const Env, index: usize, expected: fx.RpcEnvelope.Kind) fx.Exit(void, Server.Failure) {
        return inspectCorrelation(env, index, expected, 43);
    }
    fn inspectCorrelation(env: *const Env, index: usize, expected: fx.RpcEnvelope.Kind, correlation: fx.RpcEnvelope.Correlation) fx.Exit(void, Server.Failure) {
        const initialized = fx.RpcFramer.init(env, policy.frame_limits);
        var framer = initialized.success;
        defer framer.deinit();
        const fed = framer.feed(env.state.frames[index].?.items);
        switch (fed) {
            .success => {},
            .die => |defect| return .{ .die = defect },
            .fail => |problem| return .{ .fail = .{ .protocol = problem } },
            else => unreachable,
        }
        var read = framer.next();
        defer read.deinit();
        if (read == .die) return .{ .die = read.die };
        std.debug.assert(read == .success and read.success == .frame);
        var decoded = fx.RpcEnvelope.decode(env.allocator, read.success.frame.items, policy.envelope_limits);
        defer decoded.deinit();
        if (decoded == .die) return .{ .die = decoded.die };
        std.debug.assert(decoded == .success);
        const message = decoded.success;
        std.debug.assert(message.kind == expected and message.correlation == correlation and message.version == 7);
        std.debug.assert(std.mem.eql(u8, message.operation.items, operation.name));
        switch (expected) {
            .stream_chunk => {
                std.debug.assert(message.sequence == 0 and std.mem.eql(u8, message.payload.items, "owned chunk:42"));
                env.counts.chunk_seen = true;
            },
            .stream_end => {
                std.debug.assert(message.sequence == 1 and std.mem.eql(u8, message.payload.items, "owned terminal"));
                env.counts.terminal_seen = true;
            },
            .response_failure => {
                std.debug.assert(message.sequence == null and std.mem.eql(u8, message.payload.items, "owned domain failure"));
                env.counts.failure_seen = true;
            },
            else => unreachable,
        }
        return .{ .success = {} };
    }
    fn afterFirst(result: Server.DriveResult, env: *const Env) Server.Unit {
        if (env.mode == .send_domain_failure) {
            std.debug.assert(result == .ended);
            return Server.Unit.fromOwnedOutcome(env, inspect(env, 0, .response_failure));
        }
        std.debug.assert(result == .sent);
        const checked = inspect(env, 0, .stream_chunk);
        if (checked != .success) return Server.Unit.fromOwnedOutcome(env, checked);
        var waiting = Server.driveOne(Select.call);
        return waiting.flatMapOwned(env, struct {
            fn ack(status: Server.DriveResult, current: *const Env) Server.Unit {
                std.debug.assert(status == .awaiting_acknowledgement and current.counts.sent == 1);
                var encoded = fx.RpcEnvelope.encode(current.allocator, .{ .version = 7, .kind = .acknowledge, .correlation = 43, .sequence = 0, .operation = operation.name, .metadata = &.{}, .payload = "" }, policy.envelope_limits);
                defer encoded.deinit();
                if (encoded == .die) return Server.Unit.die(encoded.die);
                std.debug.assert(encoded == .success);
                var decoded = fx.RpcEnvelope.decode(current.allocator, encoded.success.items, policy.envelope_limits);
                defer decoded.deinit();
                if (decoded == .die) return Server.Unit.die(decoded.die);
                std.debug.assert(decoded == .success);
                std.debug.assert(current.state.call.control(&decoded.success, current).ok == .acknowledged);
                var next = Server.driveOne(Select.call);
                return next.flatMapOwned(current, afterTerminal);
            }
        }.ack);
    }
    fn afterTerminal(status: Server.DriveResult, env: *const Env) Server.Unit {
        std.debug.assert(status == .sent);
        return Server.Unit.fromOwnedOutcome(env, inspect(env, 1, .stream_end));
    }
};
const MultipleCalls = struct {
    fn open(registry: anytype, inputs: anytype, env: *const Env) Server.Unit {
        var opening = registry.openWith(inputs.scope, operation, inputs.context, inputs.request);
        var mapped = opening.mapErrorOwned(env, Callbacks.domain);
        const Saved = struct { registry: @TypeOf(registry), scope: *fx.Scope(Bytes) };
        return mapped.flatMapCapturedOwned(env, void, Saved{ .registry = registry, .scope = inputs.scope }, struct {
            fn first(stored: *const Saved, reader: Server.Reader, current: *const Env) Server.Unit {
                current.state.call.reader = reader;
                var context = fx.RpcContext.empty;
                context.correlation = 44;
                var second = stored.registry.openWith(stored.scope, operation, context, @as(i64, 29));
                var aligned = second.mapErrorOwned(current, Callbacks.domain);
                return aligned.flatMapOwned(current, installed);
            }
        }.first);
    }
    fn installed(reader: Server.Reader, env: *const Env) Server.Unit {
        env.state.second_call.?.reader = reader;
        std.debug.assert(env.state.admission.active == 2 and env.counts.readers == 2);
        const rejected = Server.Call.init(45, &env.state.admission);
        std.debug.assert(rejected == .err and rejected.err == .capacity_exhausted);
        var first = Server.driveOne(Select.call);
        return first.flatMapOwned(env, struct {
            fn next(status: Server.DriveResult, current: *const Env) Server.Unit {
                std.debug.assert(status == .sent);
                const checked = Registry.inspectCorrelation(current, 0, .stream_chunk, 43);
                if (checked != .success) return Server.Unit.fromOwnedOutcome(current, checked);
                var second = Server.driveOne(Select.secondCall);
                return second.flatMapOwned(current, struct {
                    fn finish(second_status: Server.DriveResult, supplied: *const Env) Server.Unit {
                        std.debug.assert(second_status == .sent);
                        std.debug.assert(supplied.counts.reader_closed == 0 and supplied.counts.registry_live == 1);
                        std.debug.assert(supplied.state.call.window.awaiting_acknowledgement == 0);
                        std.debug.assert(supplied.state.second_call.?.window.awaiting_acknowledgement == 0);
                        // End the region with both native readers and both
                        // copied owning terminal events still admitted.
                        return Server.Unit.fromOwnedOutcome(supplied, Registry.inspectCorrelation(supplied, 1, .stream_chunk, 44));
                    }
                }.finish);
            }
        }.next);
    }
};
fn description(env: *const Root) !fx.Effect(void, fx.RpcMiddlewareFailure(Server.Failure, u8), Root) {
    return fx.dsl.withRpcSession(env, .{ .definition = fx.dsl.declareMetadata(.{ .server = Server, .acquire = Callbacks.acquire, .project = Callbacks.project, .use = Callbacks.use, .release = Callbacks.release }), .inputs = .{ .owner = try Owner.init(env.allocator, &env.counts.session_live, "session configuration") } });
}

test "RPC-02 RPC-04 RPC-05 runtime registry serves owning chunks terminal and domain frames through the native provider with exact error categories" {
    inline for (.{ Mode.normal, Mode.provider_failure, Mode.serialization_failure, Mode.domain_failure, Mode.send_domain_failure }) |mode| {
        var counts: Counts = .{};
        var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
        defer threads.deinit();
        const env: Root = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &counts, .mode = mode };
        var program = try description(&env);
        var result = fx.run(&env, program);
        program.deinit();
        defer result.deinit();
        switch (mode) {
            .normal => {
                try std.testing.expect(result == .success);
                try std.testing.expect(counts.chunk_seen and counts.terminal_seen);
                try std.testing.expectEqual(@as(usize, 2), counts.sent);
            },
            .provider_failure => try std.testing.expectEqual(@as(u8, 83), result.fail.domain.session.provider),
            .serialization_failure => try std.testing.expect(result.fail.domain == .serialization),
            .domain_failure => try std.testing.expectEqualStrings("owned domain failure", result.fail.domain.domain.items),
            .send_domain_failure => {
                try std.testing.expect(result == .success);
                try std.testing.expect(counts.failure_seen);
            },
            else => unreachable,
        }
        try std.testing.expectEqual(@as(usize, 1), counts.reader_closed);
        try std.testing.expectEqual(@as(usize, 1), counts.environment_closed);
        try std.testing.expectEqual(@as(usize, 1), counts.released);
        try std.testing.expectEqual(@as(usize, 1), counts.service_closed);
        try std.testing.expectEqual(@as(usize, 0), counts.registry_live);
        try std.testing.expectEqual(@as(usize, 0), counts.session_live);
    }
}

test "RPC-04 LIFE-12 destroying a pending encoded send joins provider work before registry reader environment and middleware cleanup" {
    var counts: Counts = .{};
    var gate: SendGate = .{ .result = .{ .success = {} } };
    var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
    defer threads.deinit();
    const env: Root = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &counts, .gate = &gate };
    var program = try description(&env);
    var pending = fx.runAsync(&env, program);
    gate.started.waitUncancelable(env.io);
    program.deinit();
    try std.testing.expectEqual(@as(usize, 0), counts.reader_closed);
    try std.testing.expectEqual(@as(usize, 1), counts.registry_live);
    pending.deinit();
    try std.testing.expect(gate.cancelled.load(.acquire) and gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
    try std.testing.expectEqual(@as(usize, 1), counts.reader_closed);
    try std.testing.expectEqual(@as(usize, 1), counts.service_closed);
    try std.testing.expectEqual(@as(usize, 0), counts.session_live);
    try std.testing.expectEqual(@as(usize, 0), counts.registry_live);
}

fn containsAllocationFailure(result: anytype) bool {
    if (result == .die) return result.die.kind == .out_of_memory;
    if (result != .finalization_failed) return false;
    const detail = result.finalization_failed;
    if (detail.primary == .defected and detail.primary.defected.kind == .out_of_memory) return true;
    if (detail.first == .release_defect and detail.first.release_defect.kind == .out_of_memory) return true;
    var cursor = detail.rest;
    while (cursor) |node| {
        if (node.failure == .release_defect and node.failure.release_defect.kind == .out_of_memory) return true;
        cursor = node.next;
    }
    return false;
}

test "RPC-02 RPC-04 LIFE-07 every allocation failure in owned stream registry frame encoding provider copy acknowledgement and teardown releases initialized owners" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator, mode: Mode) !void {
            var counts: Counts = .{};
            defer {
                std.debug.assert(counts.session_live == 0 and counts.registry_live == 0);
                std.debug.assert(counts.readers == counts.reader_closed);
                std.debug.assert(counts.projected == counts.environment_closed);
            }
            var threads: Threads = .{ .allocator = allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
            defer threads.deinit();
            const env: Root = .{ .allocator = allocator, .async_engine = threads.executor(), .counts = &counts, .mode = mode };
            var program = try description(&env);
            var result = fx.run(&env, program);
            program.deinit();
            defer result.deinit();
            if (containsAllocationFailure(result)) return error.OutOfMemory;
            switch (mode) {
                .normal => try std.testing.expect(result == .success and counts.chunk_seen and counts.terminal_seen),
                .send_domain_failure => try std.testing.expect(result == .success and counts.failure_seen),
                .domain_failure => try std.testing.expectEqualStrings("owned domain failure", result.fail.domain.domain.items),
                .provider_failure => try std.testing.expectEqual(@as(u8, 83), result.fail.domain.session.provider),
                .serialization_failure => try std.testing.expect(result.fail.domain == .serialization),
                .encoded_request => try std.testing.expect(result == .success and counts.chunk_seen and counts.terminal_seen and counts.metadata_seen),
                .encoded_invalid_request => try std.testing.expect(result.fail.domain == .serialization),
                .encoded_invalid_version => try std.testing.expect(result.fail.domain.protocol == .malformed_envelope),
                .receive_failure => try std.testing.expectEqual(@as(u8, 84), result.fail.domain.session.provider),
                .multiple_calls => try std.testing.expect(result == .success and counts.reader_closed == 2 and counts.sent == 2),
                .pending_receive, .late_receive => unreachable, // exercised by explicit native cancellation tests
                .cleanup_failure, .cleanup_chain => try verifyCleanup(result, mode),
            }
        }
    };
    inline for (.{ Mode.normal, Mode.domain_failure, Mode.send_domain_failure, Mode.encoded_request, Mode.encoded_invalid_request, Mode.receive_failure, Mode.multiple_calls, Mode.cleanup_failure, Mode.cleanup_chain }) |mode|
        try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{mode});
}

test "RPC-04 RPC-05 provider received request decoding metadata and checked identity remain owned through the managed native serving session" {
    inline for (.{ Mode.encoded_request, Mode.encoded_invalid_request, Mode.encoded_invalid_version, Mode.receive_failure }) |mode| {
        var counts: Counts = .{};
        var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
        defer threads.deinit();
        const env: Root = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &counts, .mode = mode };
        var program = try description(&env);
        var result = fx.run(&env, program);
        program.deinit();
        defer result.deinit();
        switch (mode) {
            .encoded_request => {
                try std.testing.expect(result == .success);
                try std.testing.expect(counts.chunk_seen and counts.terminal_seen and counts.metadata_seen);
                try std.testing.expectEqual(@as(usize, 1), counts.request_decoded);
                try std.testing.expectEqual(@as(usize, 1), counts.reader_closed);
            },
            .encoded_invalid_request => {
                try std.testing.expect(result.fail.domain == .serialization);
                try std.testing.expectEqual(@as(usize, 0), counts.readers);
            },
            .encoded_invalid_version => {
                try std.testing.expect(result.fail.domain.protocol == .malformed_envelope);
                try std.testing.expectEqual(@as(usize, 0), counts.request_decoded);
            },
            .receive_failure => {
                try std.testing.expectEqual(@as(u8, 84), result.fail.domain.session.provider);
                try std.testing.expectEqual(@as(usize, 0), counts.readers);
                try std.testing.expectEqual(@as(usize, 0), counts.request_decoded);
            },
            else => unreachable,
        }
        try std.testing.expectEqual(@as(usize, 1), counts.environment_closed);
        try std.testing.expectEqual(@as(usize, 1), counts.released);
        try std.testing.expectEqual(@as(usize, 1), counts.service_closed);
        try std.testing.expectEqual(@as(usize, 0), counts.registry_live);
        try std.testing.expectEqual(@as(usize, 0), counts.session_live);
    }
}

test "RPC-02 RPC-04 two admitted runtime registry calls retain separate contexts readers owning pending terminals and one shared configuration through early session close" {
    var counts: Counts = .{};
    var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
    defer threads.deinit();
    const env: Root = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &counts, .mode = .multiple_calls };
    var program = try description(&env);
    var result = fx.run(&env, program);
    program.deinit();
    defer result.deinit();
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(usize, 2), counts.reader_closed);
    try std.testing.expectEqual(@as(usize, 2), counts.sent);
    try std.testing.expectEqual(@as(usize, 1), counts.environment_closed);
    try std.testing.expectEqual(@as(usize, 1), counts.released);
    try std.testing.expectEqual(@as(usize, 1), counts.service_closed);
    try std.testing.expectEqual(@as(usize, 0), counts.registry_live);
    try std.testing.expectEqual(@as(usize, 0), counts.session_live);
}

test "RPC-06 encoded envelope rejects an unknown wire kind at the checked boundary" {
    var encoded = fx.RpcEnvelope.encode(std.testing.allocator, .{ .version = 7, .kind = .request, .correlation = 43, .operation = operation.name, .metadata = &.{}, .payload = "17" }, policy.envelope_limits);
    defer encoded.deinit();
    try std.testing.expect(encoded == .success);
    encoded.success.items[@sizeOf(fx.RpcEnvelope.Version)] = std.math.maxInt(u8);
    var decoded = fx.RpcEnvelope.decode(std.testing.allocator, encoded.success.items, policy.envelope_limits);
    defer decoded.deinit();
    try std.testing.expect(decoded == .fail and decoded.fail == .malformed_envelope);
}

test "RPC-04 LIFE-12 cancelled provider receive joins before middleware and retained decoding configuration are destroyed" {
    var counts: Counts = .{};
    var gate: SendGate = .{ .result = .{ .success = {} } };
    var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
    defer threads.deinit();
    const env: Root = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &counts, .mode = .pending_receive, .gate = &gate };
    var program = try description(&env);
    var pending = fx.runAsync(&env, program);
    gate.started.waitUncancelable(env.io);
    program.deinit();
    try std.testing.expectEqual(@as(usize, 1), counts.registry_live);
    try std.testing.expectEqual(@as(usize, 1), counts.session_live);
    try std.testing.expectEqual(@as(usize, 0), counts.environment_closed);
    pending.deinit();
    try std.testing.expect(gate.cancelled.load(.acquire) and gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
    try std.testing.expectEqual(@as(usize, 0), counts.readers);
    try std.testing.expectEqual(@as(usize, 0), counts.request_decoded);
    try std.testing.expectEqual(@as(usize, 1), counts.environment_closed);
    try std.testing.expectEqual(@as(usize, 1), counts.released);
    try std.testing.expectEqual(@as(usize, 1), counts.service_closed);
    try std.testing.expectEqual(@as(usize, 0), counts.registry_live);
    try std.testing.expectEqual(@as(usize, 0), counts.session_live);
}

fn verifyCleanup(result: anytype, mode: Mode) !void {
    try std.testing.expect(result == .finalization_failed);
    const detail = result.finalization_failed;
    if (mode == .cleanup_chain)
        try std.testing.expectEqualStrings("owned domain failure", detail.primary.failed.domain.domain.items)
    else
        try std.testing.expect(detail.primary == .succeeded);
    var reader_failure = false;
    var middleware_failure = false;
    const Check = struct {
        fn observe(problem: anytype, reader: *bool, middleware: *bool) !void {
            try std.testing.expect(problem == .release_failure);
            switch (problem.release_failure) {
                .domain => |rpc| {
                    try std.testing.expectEqualStrings("owned reader cleanup", rpc.domain.items);
                    reader.* = true;
                },
                .middleware => |value| {
                    try std.testing.expectEqual(@as(u8, 91), value);
                    middleware.* = true;
                },
            }
        }
    };
    try Check.observe(detail.first, &reader_failure, &middleware_failure);
    var cursor = detail.rest;
    while (cursor) |node| {
        try Check.observe(node.failure, &reader_failure, &middleware_failure);
        cursor = node.next;
    }
    try std.testing.expect(reader_failure);
    try std.testing.expectEqual(mode == .cleanup_chain, middleware_failure);
}

test "RPC-05 LIFE-04 owning handler and reader cleanup errors survive the middleware release failure chain and complete service teardown" {
    inline for (.{ Mode.cleanup_failure, Mode.cleanup_chain }) |mode| {
        var counts: Counts = .{};
        var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
        defer threads.deinit();
        const env: Root = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &counts, .mode = mode };
        var program = try description(&env);
        var result = fx.run(&env, program);
        program.deinit();
        defer result.deinit();
        try std.testing.expectEqual(@as(usize, 1), counts.reader_closed);
        try std.testing.expectEqual(@as(usize, 1), counts.environment_closed);
        try std.testing.expectEqual(@as(usize, 1), counts.released);
        try std.testing.expectEqual(@as(usize, 1), counts.service_closed);
        try std.testing.expectEqual(@as(usize, 0), counts.registry_live);
        try std.testing.expectEqual(@as(usize, 0), counts.session_live);
        try verifyCleanup(result, mode);
    }
}

test "RPC-04 LIFE-12 a provider frame completed after cancellation is discarded before request decoding and managed session teardown" {
    var counts: Counts = .{};
    var gate: SendGate = .{ .result = .{ .success = {} } };
    var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
    defer threads.deinit();
    const env: Root = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &counts, .mode = .late_receive, .gate = &gate };
    var program = try description(&env);
    var pending = fx.runAsync(&env, program);
    gate.started.waitUncancelable(env.io);
    program.deinit();
    pending.deinit();
    try std.testing.expect(gate.cancelled.load(.acquire) and gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
    try std.testing.expectEqual(@as(usize, 1), counts.late_frames);
    try std.testing.expectEqual(@as(usize, 0), counts.request_decoded);
    try std.testing.expectEqual(@as(usize, 0), counts.readers);
    try std.testing.expectEqual(@as(usize, 1), counts.service_closed);
    try std.testing.expectEqual(@as(usize, 0), counts.registry_live);
    try std.testing.expectEqual(@as(usize, 0), counts.session_live);
}
