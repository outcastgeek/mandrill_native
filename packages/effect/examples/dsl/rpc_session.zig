//! EX-04: typed unary and streaming operations under one managed RPC session.
const std = @import("std");
const fx = @import("effect");
pub const Counts = struct { acquired: usize = 0, projected: usize = 0, constructed: usize = 0, readers: usize = 0, reader_closed: usize = 0, environment_closed: usize = 0, release_calls: usize = 0, service_closed: usize = 0 };
pub const Root = struct { allocator: std.mem.Allocator, io: std.Io, async_engine: fx.Executor, counts: *Counts, session: ?*const fx.RpcSession(HandlerEnv, u8) = null };
const Service = struct {
    pub const effect_owned_payload = true;
    counts: *Counts,
    value: i64,
    pub fn deinit(self: *@This()) void {
        std.debug.assert(self.counts.environment_closed == self.counts.projected);
        self.counts.service_closed += 1;
    }
};
const DomainScope = fx.Scope(i64);
const HandlerEnv = struct {
    pub const effect_owned_payload = true;
    allocator: std.mem.Allocator,
    io: std.Io,
    async_engine: fx.Executor,
    counts: *Counts,
    service: *const Service,
    request: *const i64,
    session: ?*const fx.RpcSession(HandlerEnv, u8),
    reader_failure: bool,
    pub fn deinit(self: *@This()) void {
        std.debug.assert(self.counts.readers == self.counts.reader_closed);
        std.debug.assert(self.counts.service_closed == 0);
        self.counts.environment_closed += 1;
    }
};
const operation = fx.dsl.rpcStreamDefinition(.{ .name = "managed.stream", .version = 1, .request = fx.JsonCodec.signed_integer, .chunk = fx.JsonCodec.signed_integer, .terminal = fx.JsonCodec.null_value, .failure = fx.JsonCodec.signed_integer });
const Event = @TypeOf(operation).Event;
const Stream = fx.Stream(Event, i64, HandlerEnv);
const Pull = fx.Effect(fx.Chunk(Event), i64, HandlerEnv);
const Iterator = struct {
    service: *const Service,
    events: [2]Event,
    index: usize = 0,
};
const Streaming = struct {
    fn allocate(env: *const HandlerEnv) fx.Exit(*Iterator, i64) {
        std.debug.assert(env.counts.service_closed == 0);
        const iterator = env.allocator.create(Iterator) catch return .{ .die = fx.Defect.out_of_memory };
        iterator.* = .{ .service = env.service, .events = .{ .{ .chunk = env.service.value + env.request.* }, .{ .terminal = {} } } };
        env.counts.readers += 1;
        return .{ .success = iterator };
    }
    fn emit(iterator: *const Iterator, _: void, _: *const HandlerEnv) Pull {
        return Pull.fromBorrowed(Iterator, iterator, struct {
            fn read(current: *const Iterator, supplied: *const HandlerEnv) fx.Exit(fx.Chunk(Event), i64) {
                std.debug.assert(current.service.counts.service_closed == 0);
                if (current.index == current.events.len) return .{ .success = .end };
                const mutable = @constCast(current);
                const index = mutable.index;
                mutable.index += 1;
                _ = supplied;
                return .{ .success = fx.Chunk(Event).borrowed(current.events[index .. index + 1]) };
            }
        }.read);
    }
    fn next(iterator: *Iterator) Pull {
        return Pull.deferBorrowed(Iterator, iterator, struct {
            fn start(current: *const Iterator, env: *const HandlerEnv) Pull {
                const Unit = fx.Effect(void, i64, HandlerEnv);
                var seed = Unit.succeed({});
                return seed.flatMapCapturedOwned(env, fx.Chunk(Event), current, struct {
                    fn resumed(stored: *const *const Iterator, _: void, supplied: *const HandlerEnv) Pull {
                        return emit(stored.*, {}, supplied);
                    }
                }.resumed);
            }
        }.start);
    }
    fn release(iterator: *Iterator) fx.Effect(void, i64, HandlerEnv) {
        return fx.Effect(void, i64, HandlerEnv).fromBorrowed(Iterator, iterator, struct {
            fn close(current: *const Iterator, env: *const HandlerEnv) fx.Exit(void, i64) {
                std.debug.assert(current.service.counts.service_closed == 0);
                env.counts.reader_closed += 1;
                env.allocator.destroy(@constCast(current));
                return if (env.reader_failure) .{ .fail = 66 } else .{ .success = {} };
            }
        }.close);
    }
    fn construct(_: *const i64, env: *const HandlerEnv) Stream {
        env.counts.constructed += 1;
        return Stream.fromPull(fx.Effect(*Iterator, i64, HandlerEnv).fromOutcome(allocate), next, release);
    }
};
const handler = operation.handledBy(HandlerEnv, Streaming.construct);
fn Wire(comptime T: type) type {
    return struct {
        fn decode(input: *const fx.OwnedSlice(u8)) fx.Effect(T, fx.SchemaIssue.Issue, HandlerEnv) {
            return fx.Effect(T, fx.SchemaIssue.Issue, HandlerEnv).fromBorrowed(fx.OwnedSlice(u8), input, struct {
                fn read(bytes: *const fx.OwnedSlice(u8), _: *const HandlerEnv) fx.Exit(T, fx.SchemaIssue.Issue) {
                    if (comptime T == void) return if (bytes.items.len == 0) .{ .success = {} } else .{ .fail = fx.SchemaIssue.Issue.pure(.decode, .invalid_value) };
                    if (bytes.items.len != 8) return .{ .fail = fx.SchemaIssue.Issue.pure(.decode, .invalid_value) };
                    return .{ .success = std.mem.readInt(i64, @ptrCast(bytes.items.ptr), .little) };
                }
            }.read);
        }
        fn encode(input: *const T) fx.Effect(fx.OwnedSlice(u8), fx.SchemaIssue.Issue, HandlerEnv) {
            return fx.Effect(fx.OwnedSlice(u8), fx.SchemaIssue.Issue, HandlerEnv).fromBorrowed(T, input, struct {
                fn write(payload: *const T, env: *const HandlerEnv) fx.Exit(fx.OwnedSlice(u8), fx.SchemaIssue.Issue) {
                    const bytes = env.allocator.alloc(u8, if (T == void) 0 else 8) catch return .{ .die = fx.Defect.out_of_memory };
                    if (comptime T != void) std.mem.writeInt(i64, @ptrCast(bytes.ptr), payload.*, .little);
                    return .{ .success = .{ .items = bytes, .allocator = env.allocator } };
                }
            }.write);
        }
        const value = fx.RpcWireCodec(T, HandlerEnv).init(decode, encode);
    };
}
const Server = fx.RpcStreamServer(handler, Wire(i64).value, Wire(i64).value, Wire(void).value, Wire(i64).value, u8, struct {
    fn select(env: *const HandlerEnv) *const fx.RpcSession(HandlerEnv, u8) {
        return env.session.?;
    }
}.select, struct {
    fn select(_: *const HandlerEnv) fx.RpcSessionPolicy {
        return .{ .max_inflight = 4, .max_pending_frames = 8, .frame_limits = .{ .max_frame_bytes = 2048, .max_buffered_bytes = 4096 }, .envelope_limits = .{ .max_envelope_bytes = 2048, .max_operation_bytes = 128, .max_payload_bytes = 1024, .max_metadata_entries = 8, .max_metadata_bytes = 256 } };
    }
}.select);
const unary = fx.dsl.rpcUnary(.{ .name = "managed.quote", .version = 1, .request = fx.JsonCodec.signed_integer, .success = fx.JsonCodec.signed_integer, .failure = fx.JsonCodec.signed_integer, .handler = struct {
    fn quote(input: *const i64, env: *const HandlerEnv) fx.Effect(i64, i64, HandlerEnv) {
        return fx.Effect(i64, i64, HandlerEnv).succeed(env.service.value + input.*);
    }
}.quote });
const Registry = fx.dsl.rpcRegistry(.{ unary, handler });
const Body = fx.Effect(u32, Server.Failure, HandlerEnv);
const ConfigurationOwner = struct {
    pub const effect_owned_payload = true;
    allocator: std.mem.Allocator,
    bytes: []u8,
    live: *usize,
    fn create(allocator: std.mem.Allocator, live: *usize) !@This() {
        const bytes = try allocator.dupe(u8, "managed session");
        live.* += 1;
        return .{ .allocator = allocator, .bytes = bytes, .live = live };
    }
    pub fn deinit(self: *@This()) void {
        self.allocator.free(self.bytes);
        self.live.* -= 1;
    }
};
const Callbacks = struct {
    fn acquire(_: anytype, _: *const Root) fx.Effect(Service, u8, Root) {
        return fx.Effect(Service, u8, Root).fromOutcome(struct {
            fn create(env: *const Root) fx.Exit(Service, u8) {
                env.counts.acquired += 1;
                return .{ .success = .{ .counts = env.counts, .value = 41 } };
            }
        }.create);
    }
    fn project(configuration: anytype, service: *const Service, env: *const Root) HandlerEnv {
        env.counts.projected += 1;
        return .{ .allocator = env.allocator, .io = env.io, .async_engine = env.async_engine, .counts = env.counts, .service = service, .request = &configuration.request, .session = env.session, .reader_failure = configuration.reader_failure };
    }
    fn domain(problem: i64) Server.Failure {
        return .{ .domain = problem };
    }
    const Loop = struct { pull: Pull, count: u32, early: bool };
    fn consume(pull: Pull, count: u32, early: bool, env: *const HandlerEnv) Body {
        var pending = pull.borrow();
        var mapped = pending.mapErrorOwned(env, domain);
        return mapped.flatMapCapturedOwned(env, u32, Loop{ .pull = pull.borrow(), .count = count, .early = early }, struct {
            fn read(stored: *const Loop, chunk: fx.Chunk(Event), current: *const HandlerEnv) Body {
                var owned = chunk;
                defer owned.deinit();
                if (owned == .end) return Body.succeed(stored.count);
                const next_count = stored.count + @as(u32, @intCast(owned.items.values.len));
                std.debug.assert(current.counts.constructed == 1);
                std.debug.assert(current.counts.service_closed == 0);
                if (stored.early) return Body.succeed(next_count);
                return consume(stored.pull, next_count, false, current);
            }
        }.read);
    }
    fn streamUse(configuration: anytype, scope: *DomainScope, env: *const HandlerEnv) Body {
        const source = handler.openWith(fx.RpcContext.empty, &configuration.request, env);
        var opening = source.open(scope);
        var opened = opening.mapErrorOwned(env, domain);
        const C = @typeInfo(@TypeOf(configuration)).pointer.child;
        return opened.acquireReleaseCapturedOwned(env, configuration, struct {
            fn run(stored: *const *const C, reader: Server.Reader, current: *const HandlerEnv) Body {
                return consume(reader.pull.borrow(), 0, stored.*.early, current);
            }
        }.run, struct {
            fn close(_: *const *const C, reader: Server.Reader, _: fx.PrimaryOutcome(Server.Failure), _: *const HandlerEnv) fx.Effect(void, Server.Failure, HandlerEnv) {
                var owned = reader;
                owned.deinit();
                return fx.Effect(void, Server.Failure, HandlerEnv).succeed({});
            }
        }.close);
    }
    fn use(configuration: anytype, scope: *DomainScope, env: *const HandlerEnv) Body {
        var invoked = Registry.invoke(unary.operation, &configuration.request);
        var mapped = invoked.mapErrorOwned(env, domain);
        return mapped.flatMapCapturedOwned(env, u32, .{ .configuration = configuration, .scope = scope }, struct {
            fn quoted(input: anytype, value: i64, current: *const HandlerEnv) Body {
                if (value != current.service.value + input.configuration.request) return Body.fromOwnedOutcome(current, .{ .fail = .{ .domain = -1 } });
                return streamUse(input.configuration, input.scope, current);
            }
        }.quoted);
    }
    fn release(_: anytype, service: *const Service, _: anytype, env: *const Root) fx.Effect(void, u8, Root) {
        std.debug.assert(service.counts.readers == service.counts.reader_closed);
        std.debug.assert(service.counts.environment_closed == service.counts.projected);
        env.counts.release_calls += 1;
        return fx.Effect(void, u8, Root).succeed({});
    }
};

pub fn describe(env: *const Root, early: bool) fx.Effect(u32, fx.RpcMiddlewareFailure(Server.Failure, u8), Root) {
    return fx.dsl.withRpcSession(env, .{ .definition = fx.dsl.declareMetadata(.{ .server = Server, .acquire = Callbacks.acquire, .project = Callbacks.project, .use = Callbacks.use, .release = Callbacks.release }), .inputs = .{ .request = @as(i64, @intCast(env.counts.acquired + 1)), .early = early, .reader_failure = false } });
}

fn check(allocator: std.mem.Allocator, io: std.Io, early: bool) !void {
    var engine: fx.NativeIoExecutor = .{ .allocator = allocator, .io = io, .capabilities = .{ .independent = true, .cpu = true, .blocking = true }, .maximum_owned = 4 };
    defer engine.deinit();
    var live: usize = 0;
    var counts: Counts = .{};
    const env: Root = .{ .allocator = allocator, .io = io, .async_engine = engine.executor(), .counts = &counts };
    var description = fx.dsl.withRpcSession(&env, .{ .definition = fx.dsl.declareMetadata(.{ .server = Server, .acquire = Callbacks.acquire, .project = Callbacks.project, .use = Callbacks.use, .release = Callbacks.release }), .inputs = .{ .owner = try ConfigurationOwner.create(allocator, &live), .request = @as(i64, @intCast(counts.acquired + 1)), .early = early, .reader_failure = false } });
    var result = fx.run(&env, description);
    description.deinit();
    defer result.deinit();
    switch (result) {
        .success => |count| try std.testing.expectEqual(@as(u32, if (early) 1 else 2), count),
        .fail => return error.RpcFailure,
        .die => return error.Defect,
        .interrupted => return error.Interrupted,
        .finalization_failed => return error.CleanupFailure,
    }
    try std.testing.expectEqual(@as(usize, 0), live);
    try std.testing.expectEqual(@as(usize, 1), counts.reader_closed);
    try std.testing.expectEqual(counts.projected, counts.environment_closed);
    try std.testing.expectEqual(counts.acquired, counts.service_closed);
}
pub fn main(init: std.process.Init) !void {
    try check(init.gpa, init.io, false);
}
test "EX-04 managed typed unary and streaming session retains service through EOF and early close" {
    try check(std.testing.allocator, std.testing.io, false);
    try check(std.testing.allocator, std.testing.io, true);
}
