const std = @import("std");
const fx = @import("effect");
const Threads = @import("support/thread_executor.zig").ThreadExecutor;

const Env = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
};

const ByteCodec = struct {
    pub fn encode(value: *const u8, env: *const Env) fx.Exit(fx.OwnedSlice(u8), fx.SchemaIssue.Issue) {
        const bytes = env.allocator.alloc(u8, @sizeOf(u8)) catch
            return .{ .die = fx.Defect.out_of_memory };
        bytes[0] = value.*;
        return .{ .success = .{ .items = bytes, .allocator = env.allocator } };
    }

    pub fn decode(bytes: []const u8, _: *const Env) fx.Exit(u8, fx.SchemaIssue.Issue) {
        if (bytes.len != @sizeOf(u8))
            return .{ .fail = fx.SchemaIssue.Issue.pure(.decode, .invalid_value) };
        return .{ .success = bytes[0] };
    }
};

test "memory cluster records fenced chunks and releases owned pages" {
    const R = struct {
        allocator: std.mem.Allocator,
        io: std.Io,
        instant: fx.WallInstant,
    };
    const Clock = struct {
        fn now(env: *const R) fx.WallInstant {
            return env.instant;
        }
    };
    var env: R = .{ .allocator = std.testing.allocator, .io = std.testing.io, .instant = .{ .unix_nanoseconds = 0 } };
    var ownership = fx.MemoryClusterOwnership(R, Clock.now).init(&env);
    defer ownership.deinit();
    var messages = fx.MemoryClusterMessageStore(R, Clock.now).init(&env, ownership.fence());
    defer messages.deinit();
    var entity_key = fx.PrimaryKey.encodeDirect(&env, "entity", &.{.{ .text = "entity" }});
    defer entity_key.deinit();
    try std.testing.expect(entity_key == .success);
    var request_key = fx.PrimaryKey.encodeDirect(&env, "request", &.{.{ .text = "request" }});
    defer request_key.deinit();
    try std.testing.expect(request_key == .success);
    const entity: fx.ClusterTypes.EntityAddress = .{ .group = "group", .entity_key = entity_key.success.items };
    const address: fx.ClusterTypes.Address = .{ .group = entity.group, .entity_key = entity.entity_key, .request_key = request_key.success.items };
    const until: fx.Deadline = .{ .at = .{ .unix_nanoseconds = 100 } };

    const owner = ownership.capability();
    var acquire_effect = owner.vtable.acquire(owner.userdata, &env, entity, "worker", .{ .until = until });
    defer acquire_effect.deinit();
    var acquired = fx.run(&env, acquire_effect);
    defer acquired.deinit();
    try std.testing.expect(acquired == .success and acquired.success == .acquired);

    const store = messages.capability();
    var accept_effect = store.vtable.accept(store.userdata, &env, .{
        .address = address,
        .definition_version = 1,
        .encoded_request = "request-body",
        .retention_until = until,
    });
    defer accept_effect.deinit();
    var accepted = fx.run(&env, accept_effect);
    defer accepted.deinit();
    try std.testing.expect(accepted == .success and accepted.success == .accepted);

    const lease = acquired.success.acquired.token();
    var claim_effect = store.vtable.claim(store.userdata, &env, entity, .{ .owner_id = lease.owner_id, .generation = lease.generation }, .{
        .max_messages = 1,
        .lease_until = .{ .at = .{ .unix_nanoseconds = 50 } },
    });
    defer claim_effect.deinit();
    var claimed = fx.run(&env, claim_effect);
    defer claimed.deinit();
    try std.testing.expect(claimed == .success and claimed.success == .claimed);
    try std.testing.expectEqual(@as(usize, 1), claimed.success.claimed.items.len);
    const claim = claimed.success.claimed.items[0].token();

    var append_effect = store.vtable.append_chunk(store.userdata, &env, claim, 1, "first");
    defer append_effect.deinit();
    var appended = fx.run(&env, append_effect);
    defer appended.deinit();
    try std.testing.expect(appended == .success and appended.success == .recorded);
    var duplicate_effect = store.vtable.append_chunk(store.userdata, &env, claim, 1, "first");
    defer duplicate_effect.deinit();
    var duplicate = fx.run(&env, duplicate_effect);
    defer duplicate.deinit();
    try std.testing.expect(duplicate == .success and duplicate.success == .already_recorded);
    var read_effect = store.vtable.read_chunks(store.userdata, &env, address, 0, .{ .max_chunks = 1, .max_encoded_bytes = 16 });
    defer read_effect.deinit();
    var page = fx.run(&env, read_effect);
    defer page.deinit();
    try std.testing.expect(page == .success and page.success == .available);
    try std.testing.expectEqualStrings("first", page.success.available.chunks.items[0].encoded.items);
    var ack_effect = store.vtable.acknowledge_chunk(store.userdata, &env, address, 1);
    defer ack_effect.deinit();
    var acknowledged = fx.run(&env, ack_effect);
    defer acknowledged.deinit();
    try std.testing.expect(acknowledged == .success and acknowledged.success == .acknowledged);
}

test "schema binary and incremental NDJSON round trip with owned buffers" {
    const env: Env = .{ .allocator = std.testing.allocator, .io = std.testing.io };
    const Case = union(enum) { ready: u8, pending: void };
    const Payload = struct { accepted: bool, count: u32, note: ?[]const u8, case: Case };
    const Codec = fx.TypedBinary.Codec(Payload, "proof.payload.v1");
    const limits: fx.TypedBinary.Limits = .{
        .frame = .{ .max_schema_name_bytes = 32, .max_payload_bytes = 256, .max_frame_bytes = 300 },
        .max_collection_items = 64,
        .max_depth = 8,
    };
    const input: Payload = .{ .accepted = true, .count = 7, .note = "hello", .case = .{ .ready = 9 } };
    var encoded = Codec.encode(&env, &input, limits);
    defer encoded.deinit();
    try std.testing.expect(encoded == .success);
    var decoded = Codec.decode(&env, encoded.success.items, limits);
    defer decoded.deinit();
    try std.testing.expect(decoded == .success);
    try std.testing.expect(decoded.success.value.accepted);
    try std.testing.expectEqual(input.count, decoded.success.value.count);
    try std.testing.expectEqualStrings(input.note.?, decoded.success.value.note.?);
    try std.testing.expect(decoded.success.value.case == .ready);
    try std.testing.expectEqual(input.case.ready, decoded.success.value.case.ready);

    const line_limits: fx.Ndjson.Limits = .{ .max_record_bytes = 64, .max_buffered_bytes = 128 };
    var line = fx.Ndjson.encodeOne(&env, "{\"value\":7}", line_limits);
    defer line.deinit();
    try std.testing.expect(line == .success);
    const opened = fx.Ndjson.Decoder.init(&env, line_limits);
    var decoder = switch (opened) {
        .success => |value| value,
        else => return error.DecoderCreationFailed,
    };
    defer decoder.deinit();
    const halfway = line.success.items.len / 2;
    try std.testing.expect(decoder.feed(line.success.items[0..halfway]) == .success);
    var pending = decoder.next();
    defer pending.deinit();
    try std.testing.expect(pending == .success and pending.success == .need_more);
    try std.testing.expect(decoder.feed(line.success.items[halfway..]) == .success);
    var record = decoder.next();
    defer record.deinit();
    try std.testing.expect(record == .success and record.success == .record);
    try std.testing.expectEqualStrings("{\"value\":7}", record.success.record.items);
}

test "hydration preserves encoded identity, version and revision without leaks" {
    const env: Env = .{ .allocator = std.testing.allocator, .io = std.testing.io };
    const created = fx.AtomRegistry.create(&env, fx.ReactiveGraphLimits{
        .max_nodes = 8,
        .max_dependencies_per_node = 4,
    }, 8);
    const registry = switch (created) {
        .success => |value| value,
        else => return error.RegistryCreationFailed,
    };
    defer registry.deinit();

    const encoded = fx.PrimaryKey.encodeDirect(&env, "proof-atom", &.{.{ .text = "value" }});
    var key = switch (encoded) {
        .success => |value| value,
        else => return error.KeyEncodingFailed,
    };
    defer key.deinit();

    const State = fx.ReactiveHydration.State(u8, Env, 1, ByteCodec);
    const initialized = State.init(&env, registry, key.items, 1, 8, 1);
    var state = switch (initialized) {
        .success => |value| value,
        else => return error.HydrationCreationFailed,
    };
    defer state.deinit();

    const captured = state.snapshot(&env);
    var previous = switch (captured) {
        .success => |value| value,
        else => return error.SnapshotFailed,
    };
    defer previous.deinit();

    var updated = state.set(2);
    defer updated.deinit();
    try std.testing.expect(updated == .success);
    var stale = state.restore(&env, &previous);
    defer stale.deinit();
    try std.testing.expect(stale == .fail and stale.fail == .stale);
}

const CacheEnv = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    async_engine: fx.Executor,
    stored: ?u8 = null,
    lookups: usize = 0,
};
const CacheRead = fx.PersistedCacheRead(u8);
const CacheFailure = fx.PersistedCacheFailure(void, void);
const CacheProof = struct {
    fn read(key: u8, env: *const CacheEnv) fx.Effect(CacheRead, void, CacheEnv) {
        return fx.Effect(CacheRead, void, CacheEnv).fromOwned(env, key, struct {
            fn load(_: *const u8, current: *const CacheEnv) fx.Exit(CacheRead, void) {
                return .{ .success = if (current.stored) |value|
                    .{ .found = value }
                else
                    .missing };
            }
        }.load);
    }

    fn lookup(key: u8, env: *const CacheEnv) fx.Effect(u8, void, CacheEnv) {
        return fx.Effect(u8, void, CacheEnv).fromOwned(env, key, struct {
            fn load(selected: *const u8, current: *const CacheEnv) fx.Exit(u8, void) {
                @constCast(current).lookups += 1;
                return .{ .success = selected.* };
            }
        }.load);
    }

    fn write(_: u8, value: *const u8, env: *const CacheEnv) fx.Effect(fx.WriteAcknowledgement, void, CacheEnv) {
        return fx.Effect(fx.WriteAcknowledgement, void, CacheEnv).fromOwned(env, value.*, struct {
            fn save(selected: *const u8, current: *const CacheEnv) fx.Exit(fx.WriteAcknowledgement, void) {
                @constCast(current).stored = selected.*;
                return .{ .success = .committed };
            }
        }.save);
    }

    fn remove(key: u8, env: *const CacheEnv) fx.Effect(fx.WriteAcknowledgement, void, CacheEnv) {
        return fx.Effect(fx.WriteAcknowledgement, void, CacheEnv).fromOwned(env, key, struct {
            fn erase(_: *const u8, current: *const CacheEnv) fx.Exit(fx.WriteAcknowledgement, void) {
                @constCast(current).stored = null;
                return .{ .success = .committed };
            }
        }.erase);
    }
};

test "persisted cache keeps write acknowledgement and invalidation order" {
    var engine: Threads = .{ .allocator = std.testing.allocator, .capacity = 8, .capabilities = .{ .independent = true, .cpu = true, .blocking = true } };
    defer engine.deinit();
    var env: CacheEnv = .{ .allocator = std.testing.allocator, .io = std.testing.io, .async_engine = engine.executor() };
    const Cached = fx.PersistedCache(u8, u8, void, void, CacheEnv, CacheProof.read, CacheProof.lookup, CacheProof.write, CacheProof.remove);
    const config: fx.CacheConfig(u8, CacheFailure) = .{
        .keys = .{ .userdata = null, .equal = struct {
            fn equal(_: ?*anyopaque, a: u8, b: u8) bool {
                return a == b;
            }
        }.equal, .retain = struct {
            fn retain(_: ?*anyopaque, value: u8) error{OutOfMemory}!u8 {
                return value;
            }
        }.retain, .release = struct {
            fn release(_: ?*anyopaque, _: *u8) void {}
        }.release },
        .time_to_live = null,
        .capacity = 2,
        .maximum_inflight = 2,
        .cache_failure = struct {
            fn cache(_: CacheFailure) bool {
                return false;
            }
        }.cache,
    };
    const created = Cached.create(&env, config);
    var cached = switch (created) {
        .success => |value| value,
        else => return error.CacheCreationFailed,
    };
    defer {
        var closed = cached.shutdown();
        closed.deinit();
        cached.deinit();
    }
    var first = cached.get(3, null);
    defer first.deinit();
    try std.testing.expect(first == .success);
    try std.testing.expectEqual(@as(u8, 3), first.success.data);
    try std.testing.expect(first.success.write.? == .committed);
    var second = cached.get(3, null);
    defer second.deinit();
    try std.testing.expect(second == .success);
    try std.testing.expectEqual(@as(usize, 1), env.lookups);
    var invalidated = cached.invalidate(3);
    defer invalidated.deinit();
    try std.testing.expect(invalidated == .success);
    try std.testing.expect(invalidated.success.acknowledgement == .committed);
    try std.testing.expect(invalidated.success.local == .success);
}

const ClusterEnv = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    store: *const fx.ClusterMessageStore(@This(), void, .@"volatile"),
    engine: *const fx.WorkflowEngine(@This(), void, .@"volatile"),
    signal: *const fx.WorkflowDurableDeferred(@This(), void, .@"volatile"),
    clock: *const fx.WorkflowDurableClock(@This(), void, .@"volatile"),
    claim: *const fx.ClusterClaimedMessage,
};
const ClusterSelectors = struct {
    fn store(env: *const ClusterEnv) *const fx.ClusterMessageStore(ClusterEnv, void, .@"volatile") {
        return env.store;
    }
    fn engine(env: *const ClusterEnv) *const fx.WorkflowEngine(ClusterEnv, void, .@"volatile") {
        return env.engine;
    }
    fn signal(env: *const ClusterEnv) *const fx.WorkflowDurableDeferred(ClusterEnv, void, .@"volatile") {
        return env.signal;
    }
    fn clock(env: *const ClusterEnv) *const fx.WorkflowDurableClock(ClusterEnv, void, .@"volatile") {
        return env.clock;
    }
    fn retention(_: *const ClusterEnv) fx.Deadline {
        return .never;
    }
    fn request(command: *const fx.ClusterWorkflowCommand) []const u8 {
        return command.execution().encoded;
    }
    fn claim(env: *const ClusterEnv) *const fx.ClusterClaimedMessage {
        return env.claim;
    }
};
const ClusterBridge = fx.ClusterWorkflowEngine(
    ClusterEnv,
    void,
    void,
    void,
    void,
    void,
    .@"volatile",
    .@"volatile",
    .@"volatile",
    .@"volatile",
    ClusterSelectors.store,
    ClusterSelectors.engine,
    ClusterSelectors.signal,
    ClusterSelectors.clock,
    ClusterSelectors.retention,
    "workflow-proof",
    1,
    ClusterSelectors.request,
    ClusterCodec,
);
const ClusterCodec = struct {
    pub fn encodeCommand(_: *const fx.ClusterWorkflowCommand, _: *const ClusterEnv) fx.Exit(fx.OwnedSlice(u8), void) {
        return .{ .die = fx.Defect.out_of_memory };
    }
    pub fn decodeCommand(_: []const u8, _: *const ClusterEnv) fx.Exit(fx.ClusterWorkflowCommand, void) {
        return .{ .die = fx.Defect.out_of_memory };
    }
    pub fn encodeOutcome(_: *const ClusterBridge.Recorded, _: *const ClusterEnv) fx.Exit(fx.OwnedSlice(u8), void) {
        return .{ .die = fx.Defect.out_of_memory };
    }
    pub fn decodeOutcome(_: []const u8, _: *const ClusterEnv) fx.Exit(ClusterBridge.Recorded, void) {
        return .{ .die = fx.Defect.out_of_memory };
    }
};

test "cluster workflow rejects a foreign claim before provider dispatch" {
    const allocator = std.testing.allocator;
    const empty = struct {
        fn bytes(a: std.mem.Allocator, data: []const u8) !fx.OwnedSlice(u8) {
            return .{ .items = try a.dupe(u8, data), .allocator = a };
        }
    }.bytes;
    var claim: fx.ClusterClaimedMessage = .{
        .group = try empty(allocator, "foreign"),
        .entity_key = try empty(allocator, ""),
        .request_key = try empty(allocator, ""),
        .encoded_request = try empty(allocator, ""),
        .definition_version = 1,
        .owner_id = try empty(allocator, ""),
        .generation = 1,
        .delivery_attempt = 1,
    };
    defer claim.deinit();
    const env: ClusterEnv = .{ .allocator = allocator, .io = std.testing.io, .store = undefined, .engine = undefined, .signal = undefined, .clock = undefined, .claim = &claim };
    const running = ClusterBridge.serveClaim(ClusterSelectors.claim);
    var result = fx.run(&env, running);
    defer result.deinit();
    try std.testing.expect(result == .fail and result.fail == .invalid_identity);
}

const StreamEnv = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    state: *fx.ReactiveQueryState(u8, void),
    emitted: *bool,
    request: u8,
};
const StreamClient = struct {
    pub const Request = u8;
    pub const Event = u8;
    pub const Failure = void;
    pub const Stream = fx.Stream(Event, Failure, StreamEnv);

    pub fn stream(comptime select_request: fn (*const StreamEnv) *const Request) Stream {
        _ = select_request;
        const Acquire = fx.Effect(void, Failure, StreamEnv);
        return Stream.fromPull(Acquire.succeed({}), struct {
            fn pull(_: void) fx.Effect(fx.Chunk(Event), Failure, StreamEnv) {
                return fx.Effect(fx.Chunk(Event), Failure, StreamEnv).fromOutcome(struct {
                    fn next(env: *const StreamEnv) fx.Exit(fx.Chunk(Event), Failure) {
                        if (env.emitted.*) return .{ .success = .end };
                        env.emitted.* = true;
                        const values = env.allocator.alloc(Event, 1) catch
                            return .{ .die = fx.Defect.out_of_memory };
                        values[0] = env.request;
                        return .{ .success = fx.Chunk(Event).owned(.{ .items = values, .allocator = env.allocator }) };
                    }
                }.next);
            }
        }.pull, struct {
            fn release(_: void) fx.Effect(void, Failure, StreamEnv) {
                return fx.Effect(void, Failure, StreamEnv).succeed({});
            }
        }.release);
    }
};
const StreamSelectors = struct {
    fn request(env: *const StreamEnv) *const u8 {
        return &env.request;
    }
    fn state(env: *const StreamEnv) *fx.ReactiveQueryState(u8, void) {
        return env.state;
    }
};

test "RPC stream events publish into query state without leaks" {
    var emitted = false;
    var env: StreamEnv = .{ .allocator = std.testing.allocator, .io = std.testing.io, .state = undefined, .emitted = &emitted, .request = 7 };
    var source = switch (fx.InvalidationRegistry.init(&env, 8)) {
        .success => |value| value,
        else => return error.InvalidationCreationFailed,
    };
    defer source.deinit();
    var key = switch (fx.PrimaryKey.encodeDirect(&env, "stream-proof", &.{.{ .text = "query" }})) {
        .success => |value| value,
        else => return error.KeyEncodingFailed,
    };
    defer key.deinit();
    var state = switch (fx.ReactiveQueryState(u8, void).init(&env, &source, key.items, 8)) {
        .success => |value| value,
        else => return error.QueryCreationFailed,
    };
    defer state.deinit();
    env.state = &state;
    const program = fx.ReactiveRpc.streamIntoQuery(StreamClient, StreamEnv, StreamSelectors.request, StreamSelectors.state);
    var result = fx.run(&env, program);
    defer result.deinit();
    try std.testing.expect(result == .success and result.success == .completed);
    var observed = state.read();
    defer observed.deinit();
    try std.testing.expect(observed == .success and observed.success == .success);
    try std.testing.expectEqual(@as(u8, 7), observed.success.success);
}
