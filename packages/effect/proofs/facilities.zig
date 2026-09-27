const std = @import("std");
const fx = @import("effect");
const E = enum { expected, syntax, shape };
const Env = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    cancellation: *fx.Cancellation,
    async_engine: fx.Executor,
    parallel_engine: fx.Executor,
    blocking_engine: fx.Executor,
    count: *std.atomic.Value(usize),
    released: *std.atomic.Value(usize),
    clock: fx.Clock,
    tracer: fx.Tracer = undefined,
    logger: fx.Logger = undefined,
    random: fx.Random = undefined,
    json: []const u8 = "23",
    http_transport: fx.Http.Transport = undefined,
    websocket_transport: fx.WebSocket.Transport = undefined,
};
const P = fx.Effect(u32, E, Env);
const Unit = fx.Effect(void, E, Env);
const Harness = struct {
    engine: fx.NativeIoExecutor = undefined,
    cancellation: fx.Cancellation = .{},
    count: std.atomic.Value(usize) = .init(0),
    released: std.atomic.Value(usize) = .init(0),
    io: std.Io = std.testing.io,
    fn init(self: *@This()) void {
        self.engine = .{ .allocator = std.testing.allocator, .io = self.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = true }, .maximum_owned = 16 };
    }
    fn env(self: *@This()) Env {
        return .{ .allocator = std.testing.allocator, .io = self.io, .cancellation = &self.cancellation, .async_engine = self.engine.executor(), .parallel_engine = self.engine.executor(), .blocking_engine = self.engine.executor(), .count = &self.count, .released = &self.released, .clock = fx.Clock.fromIo(&self.io) };
    }
    fn deinit(self: *@This()) void {
        self.engine.deinit();
    }
};
fn releaseScope(scope: *fx.Scope(E), env: *const Env) void {
    var closed = fx.run(env, Unit.closeScope(scope));
    closed.deinit();
    scope.deinit();
}

fn increment(value: u32) P {
    return P.succeed(value + 1);
}
fn unit(_: u32) Unit {
    return Unit.succeed({});
}
fn recovery(_: E) P {
    return P.succeed(17);
}
fn mapped(_: E) u32 {
    return 9;
}
fn count(env: *const Env) fx.Exit(u32, E) {
    return .{ .success = @intCast(env.count.fetchAdd(1, .monotonic)) };
}
fn failOnce(env: *const Env) fx.Exit(u32, E) {
    return if (env.count.fetchAdd(1, .monotonic) == 0) .{ .fail = .expected } else .{ .success = 3 };
}

test "sequential collection, error mapping and bounded retry retain typed outcomes" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    const env = harness.env();
    const zipped = comptime P.succeed(1).zip(P.succeed(2));
    const pair = fx.run(&env, zipped).success;
    try std.testing.expectEqual(@as(u32, 1), pair.left);
    try std.testing.expectEqual(@as(u32, 2), pair.right);
    var all = fx.run(&env, P.forEach(u32, &.{ 1, 2, 3 }, increment));
    defer all.deinit();
    try std.testing.expectEqualSlices(u32, &.{ 2, 3, 4 }, all.success.items);
    var discarded = fx.run(&env, P.forEachDiscard(u32, &.{ 1, 2, 3 }, unit));
    defer discarded.deinit();
    try std.testing.expect(discarded == .success);
    const err = comptime P.fail(.expected).mapError(mapped);
    try std.testing.expectEqual(@as(u32, 9), fx.run(&env, err).fail);
    const caught = comptime P.fail(.expected).catchAll(recovery);
    try std.testing.expectEqual(@as(u32, 17), fx.run(&env, caught).success);
    const retry = comptime P.fromOutcome(failOnce).retry(fx.Schedule.recurs(E, 1));
    try std.testing.expectEqual(@as(u32, 3), fx.run(&env, retry).success);
}

test "schema parsing owns document views and releases every allocation failure" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    var env = harness.env();
    const F = struct {
        fn decode(value: std.json.Value) fx.Result(u32, E) {
            if (value != .integer) return .{ .err = .shape };
            return .{ .ok = std.math.cast(u32, value.integer) orelse return .{ .err = .shape } };
        }
        fn text(environment: *const Env) []const u8 {
            return environment.json;
        }
        fn syntax(_: std.json.ParseError(std.json.Scanner)) E {
            return .syntax;
        }
    };
    const parser = comptime fx.Schema(u32, E).fromDecoder(F.decode).parse(Env, F.text, F.syntax);
    var tracking = std.testing.FailingAllocator.init(std.testing.allocator, .{});
    env.allocator = tracking.allocator();
    var parsed = fx.run(&env, parser);
    try std.testing.expectEqual(@as(u32, 23), parsed.success.value);
    parsed.deinit();
    try std.testing.expectEqual(tracking.allocated_bytes, tracking.freed_bytes);
    const allocations = tracking.alloc_index;
    for (0..allocations) |index| {
        var failing = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = index });
        env.allocator = failing.allocator();
        var failed = fx.run(&env, parser);
        defer failed.deinit();
        try std.testing.expect(failed == .die);
        try std.testing.expectEqual(fx.Defect.Kind.out_of_memory, failed.die.kind);
        try std.testing.expectEqual(failing.allocated_bytes, failing.freed_bytes);
    }
    env.allocator = std.testing.allocator;
    env.json = "{";
    try std.testing.expectEqual(E.syntax, fx.run(&env, parser).fail);
    env.json = "{}";
    try std.testing.expectEqual(E.shape, fx.run(&env, parser).fail);
}

test "scoped deferred, ref and rendezvous queue work through supplied I/O" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    const env = harness.env();
    var scope = fx.Scope(E).init(&env);
    defer releaseScope(&scope, &env);
    const D = fx.EffectDeferred(u32, E, Env);
    const deferred = fx.run(&env, D.make(&scope)).success;
    const complete = D.complete(deferred, struct {
        fn value(_: *const Env) fx.Exit(u32, E) {
            return .{ .success = 7 };
        }
    }.value);
    try std.testing.expect(fx.run(&env, complete).success);
    try std.testing.expect(!fx.run(&env, complete).success);
    try std.testing.expectEqual(@as(u32, 7), fx.run(&env, D.awaitValue(deferred)).success);
    var polled = fx.run(&env, D.poll(deferred));
    defer polled.deinit();
    try std.testing.expectEqual(@as(u32, 7), polled.success.?.success);
    const Ref = fx.EffectRef(u32, E, Env);
    const ref = fx.run(&env, Ref.make(&scope, struct {
        fn initial(_: *const Env) u32 {
            return 1;
        }
    }.initial)).success;
    _ = fx.run(&env, Ref.update(ref, struct {
        fn update(value: u32) u32 {
            return value + 1;
        }
    }.update));
    try std.testing.expectEqual(@as(u32, 2), fx.run(&env, Ref.get(ref)).success);
    var modified = fx.run(&env, Ref.modify(ref, struct {
        fn modify(value: u32) struct { result: u32, value: u32 } {
            return .{ .result = value, .value = value + 1 };
        }
    }.modify));
    defer modified.deinit();
    try std.testing.expectEqual(@as(u32, 2), modified.success);
    const Q = fx.EffectQueue(u32, E, Env);
    const queue = fx.run(&env, Q.make(&scope, struct {
        fn capacity(_: *const Env) ?usize {
            return 0;
        }
    }.capacity)).success;
    var offer = Q.offerValue(queue, 11);
    defer offer.deinit();
    var offering = fx.runAsync(&env, offer);
    defer offering.deinit();
    var taken = fx.run(&env, Q.take(queue));
    defer taken.deinit();
    try std.testing.expectEqual(@as(u32, 11), taken.success);
    var offered = offering.join();
    defer offered.deinit();
    try std.testing.expect(offered == .success);
}

test "independent CPU and blocking engines compose in ordered concurrent traversal" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    const env = harness.env();
    const F = struct {
        fn work(environment: *const Env, _: *const fx.Cancellation) fx.Exit(u32, E) {
            return .{ .success = @intCast(environment.count.fetchAdd(1, .monotonic)) };
        }
    };
    const cpu = comptime P.onCpu(F.work);
    const blocking = comptime P.onBlocking(F.work);
    const inputs = [_]P{ cpu, blocking };
    var run = fx.runAsync(&env, P.allConcurrent(2, &inputs));
    defer run.deinit();
    var result = run.join();
    defer result.deinit();
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(usize, 2), result.success.items.len);
    try std.testing.expectEqual(@as(usize, 2), harness.count.load(.acquire));
}

test "race joins both branches and scope closes an observed fiber" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    const env = harness.env();
    const race = comptime P.fromOutcome(count).race(P.succeed(99));
    var result = fx.run(&env, race);
    defer result.deinit();
    try std.testing.expect(result == .success);
    var scope = fx.Scope(E).init(&env);
    defer releaseScope(&scope, &env);
    const program = comptime P.succeed(31);
    var forked = fx.run(&env, program.fork(&scope));
    defer forked.deinit();
    const fiber = forked.success;
    var observed = fx.run(&env, P.awaitFiberExit(fiber));
    defer observed.deinit();
    try std.testing.expectEqual(@as(u32, 31), observed.success.success);
}

test "streams transform chunks, enforce limits and release iterators" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    const env = harness.env();
    const Iterator = struct { index: u32 = 0 };
    const F = struct {
        fn create(environment: *const Env) fx.Exit(*Iterator, E) {
            const iterator = environment.allocator.create(Iterator) catch return .{ .die = fx.Defect.out_of_memory };
            iterator.* = .{};
            return .{ .success = iterator };
        }
        fn next(iterator: *Iterator, _: *const Env) fx.Exit(?u32, E) {
            if (iterator.index == 3) return .{ .success = null };
            iterator.index += 1;
            return .{ .success = iterator.index };
        }
        fn release(iterator: *Iterator) Unit {
            return Unit.fromBorrowed(Iterator, iterator, struct {
                fn destroy(value: *const Iterator, environment: *const Env) fx.Exit(void, E) {
                    environment.allocator.destroy(@constCast(value));
                    _ = environment.released.fetchAdd(1, .monotonic);
                    return .{ .success = {} };
                }
            }.destroy);
        }
    };
    const stream = comptime fx.Stream(u32, E, Env).fromIterable(Iterator, F.create, F.next, F.release).mapEffect(increment);
    var collected = fx.run(&env, stream.runCollect(3));
    defer collected.deinit();
    try std.testing.expectEqualSlices(u32, &.{ 2, 3, 4 }, collected.success.items);
    try std.testing.expectEqual(@as(usize, 1), harness.released.load(.acquire));
    var limited = fx.run(&env, stream.runCollect(2));
    defer limited.deinit();
    try std.testing.expect(limited == .die);
    try std.testing.expectEqual(@as(usize, 2), harness.released.load(.acquire));
    var consumed = fx.run(&env, stream.runForEach(unit));
    defer consumed.deinit();
    try std.testing.expect(consumed == .success);
}

fn cacheConfig(_: *const Env) fx.CacheConfig(u32, E) {
    return .{ .keys = .{ .userdata = null, .equal = struct {
        fn equal(_: ?*anyopaque, left: u32, right: u32) bool {
            return left == right;
        }
    }.equal, .retain = struct {
        fn retain(_: ?*anyopaque, value: u32) error{OutOfMemory}!u32 {
            return value;
        }
    }.retain, .release = struct {
        fn release(_: ?*anyopaque, _: *u32) void {}
    }.release }, .time_to_live = null, .capacity = 2, .maximum_inflight = 2, .cache_failure = struct {
        fn cache(_: E) bool {
            return false;
        }
    }.cache };
}
fn load(key: u32) P {
    return P.succeed(key * 2);
}
test "scoped cache shares settled values and invalidation drains ownership" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    const env = harness.env();
    var scope = fx.Scope(E).init(&env);
    defer releaseScope(&scope, &env);
    const C = fx.EffectCache(u32, u32, E, Env);
    const cache = fx.run(&env, C.make(&scope, cacheConfig, load)).success;
    var get = C.get(cache, 5);
    defer get.deinit();
    try std.testing.expectEqual(@as(u32, 10), fx.run(&env, get).success);
    try std.testing.expectEqual(@as(u32, 10), fx.run(&env, get).success);
    var invalidation = C.invalidate(cache, 5);
    defer invalidation.deinit();
    var invalidated = fx.run(&env, invalidation);
    defer invalidated.deinit();
    try std.testing.expect(invalidated == .success);
    try std.testing.expectEqual(@as(u32, 10), fx.run(&env, get).success);
}

fn poolConfig(_: *const Env) fx.PoolConfig {
    return .{ .capacity = 2, .idle_time_to_live = null };
}
fn validate(_: u32) fx.Effect(bool, E, Env) {
    return fx.Effect(bool, E, Env).succeed(true);
}
fn dispose(_: u32) Unit {
    return Unit.sync(struct {
        fn release(environment: *const Env) void {
            _ = environment.released.fetchAdd(1, .monotonic);
        }
    }.release);
}
test "pool population, exclusive leasing, release and discard drain exactly once" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    const env = harness.env();
    var pool_scope = fx.Scope(E).init(&env);
    defer releaseScope(&pool_scope, &env);
    const Pool = fx.EffectPool(u32, E, Env);
    const factory = comptime P.fromOutcome(count);
    const pool = fx.run(&env, Pool.make(&pool_scope, poolConfig, factory, validate, dispose)).success;
    var population = Pool.ensureSize(pool, 2);
    defer population.deinit();
    var populated = fx.run(&env, population);
    defer populated.deinit();
    try std.testing.expect(populated == .success);
    try std.testing.expectEqual(@as(usize, 2), harness.count.load(.acquire));
    var lease_scope = fx.Scope(E).init(&env);
    var first_acquire = Pool.acquire(&lease_scope, pool);
    defer first_acquire.deinit();
    const first = fx.run(&env, first_acquire).success;
    var second_acquire = Pool.acquire(&lease_scope, pool);
    defer second_acquire.deinit();
    const second = fx.run(&env, second_acquire).success;
    try std.testing.expect(Pool.value(first) != Pool.value(second));
    var returned = fx.run(&env, Pool.release(first));
    defer returned.deinit();
    try std.testing.expect(returned == .success);
    var retired = fx.run(&env, Pool.discard(second));
    defer retired.deinit();
    try std.testing.expect(retired == .success);
    releaseScope(&lease_scope, &env);
    try std.testing.expectEqual(@as(usize, 1), harness.released.load(.acquire));
}

test "managed HTTP owns responses, validates status, and releases bodies" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    var env = harness.env();
    const Mock = struct {
        env: *const Env,
        status: std.http.Status = .ok,
        sends: usize = 0,
        disposed: usize = 0,
        cancelled: usize = 0,
        fn send(data: *anyopaque, _: fx.Http.Request, _: ?*const fx.Cancellation) fx.Exit(fx.Http.NativeResponse, fx.Http.Failure) {
            const mock: *@This() = @ptrCast(@alignCast(data));
            mock.sends += 1;
            return .{ .success = .{ .userdata = mock, .status = mock.status, .vtable = &.{ .read = read, .cancel = cancel, .dispose = release } } };
        }
        fn read(data: *anyopaque, _: ?*const fx.Cancellation) fx.Exit(fx.OwnedSlice(u8), fx.Http.Failure) {
            const mock: *@This() = @ptrCast(@alignCast(data));
            return .{ .success = .{ .items = mock.env.allocator.dupe(u8, "body") catch return .{ .die = fx.Defect.out_of_memory }, .allocator = mock.env.allocator } };
        }
        fn cancel(data: *anyopaque) ?fx.Defect {
            const mock: *@This() = @ptrCast(@alignCast(data));
            mock.cancelled += 1;
            return null;
        }
        fn release(data: *anyopaque) fx.Exit(void, fx.Http.Failure) {
            const mock: *@This() = @ptrCast(@alignCast(data));
            mock.disposed += 1;
            return .{ .success = {} };
        }
    };
    var mock: Mock = .{ .env = &env };
    env.http_transport = .{ .userdata = &mock, .send = Mock.send };
    const F = struct {
        fn request(_: *const Env) fx.Http.Request {
            return .{ .uri = "http://example.test", .method = .GET, .headers = &.{}, .body = null };
        }
        fn accept(status: std.http.Status) bool {
            return status == .ok;
        }
    };
    const fetch = comptime fx.Fetch(Env).fetchText(F.request, F.accept);
    var text = fx.run(&env, fetch);
    defer text.deinit();
    try std.testing.expect(text == .success);
    try std.testing.expectEqualStrings("body", text.success.items);
    try std.testing.expectEqual(@as(usize, 1), mock.disposed);
    mock.status = .bad_request;
    var rejected = fx.run(&env, fetch);
    defer rejected.deinit();
    try std.testing.expectEqual(std.http.Status.bad_request, rejected.fail.http);
    try std.testing.expectEqual(@as(usize, 2), mock.disposed);
    try std.testing.expectEqual(mock.disposed, mock.cancelled);
}

test "native HTTP adapter rejects invalid URI without leaking reservations" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    var env = harness.env();
    var client: std.http.Client = .{ .allocator = env.allocator, .io = env.io };
    defer client.deinit();
    var native: fx.Http.Native = .{ .client = &client, .options = .{}, .redirect_buffer_bytes = 64, .transfer_buffer_bytes = 64, .maximum_body_bytes = 256 };
    env.http_transport = native.transport();
    const F = struct {
        fn request(_: *const Env) fx.Http.Request {
            return .{ .uri = ":", .method = .GET, .headers = &.{}, .body = null };
        }
        fn accept(_: std.http.Status) bool {
            return true;
        }
    };
    var result = fx.run(&env, comptime fx.Fetch(Env).fetchText(F.request, F.accept));
    defer result.deinit();
    try std.testing.expect(result == .fail);
    try std.testing.expect(result.fail == .transport);
}

test "managed WebSocket reassembles UTF-8 fragments and finalizes native ownership" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    var env = harness.env();
    var clock = fx.ManualClock.init(&env, .{ .nanoseconds = 0 });
    defer clock.deinit();
    env.clock = clock.service();
    const Mock = struct {
        index: usize = 0,
        disposed: usize = 0,
        aborted: usize = 0,
        closed: bool = false,
        fn socket(self: *@This()) fx.WebSocket.NativeSocket {
            return .{ .userdata = self, .vtable = &.{ .send = send, .receive = receive, .close_output = finish, .close = finish, .abort = abort, .dispose = disposeSocket, .sub_protocol = protocol, .is_open = isOpen } };
        }
        fn send(_: *anyopaque, _: fx.WebSocket.Kind, _: []const u8, _: ?*const fx.Cancellation) fx.Exit(void, fx.WebSocket.Error) {
            return .{ .success = {} };
        }
        fn receive(data: *anyopaque, buffer: []u8, _: ?*const fx.Cancellation) fx.Exit(fx.WebSocket.Fragment, fx.WebSocket.Error) {
            const mock: *@This() = @ptrCast(@alignCast(data));
            const text = "é";
            if (mock.index == text.len) return .{ .success = .{ .kind = .close, .count = 0, .end_of_message = true, .close = .{ .status = null, .description = null } } };
            buffer[0] = text[mock.index];
            mock.index += 1;
            return .{ .success = .{ .kind = .text, .count = 1, .end_of_message = mock.index == text.len, .close = .{ .status = null, .description = null } } };
        }
        fn finish(_: *anyopaque, _: ?*const fx.Cancellation) fx.Exit(void, fx.WebSocket.Error) {
            return .{ .success = {} };
        }
        fn abort(data: *anyopaque) ?fx.Defect {
            const mock: *@This() = @ptrCast(@alignCast(data));
            mock.aborted += 1;
            mock.closed = true;
            return null;
        }
        fn disposeSocket(data: *anyopaque) fx.Exit(void, fx.WebSocket.Error) {
            const mock: *@This() = @ptrCast(@alignCast(data));
            mock.disposed += 1;
            return .{ .success = {} };
        }
        fn protocol(_: *anyopaque) ?[]const u8 {
            return "example";
        }
        fn isOpen(data: *anyopaque) bool {
            const mock: *@This() = @ptrCast(@alignCast(data));
            return !mock.closed;
        }
        fn connect(data: *anyopaque, _: fx.WebSocket.ConnectOptions, _: ?*const fx.Cancellation) fx.Exit(fx.WebSocket.NativeSocket, fx.WebSocket.Error) {
            const mock: *@This() = @ptrCast(@alignCast(data));
            return .{ .success = mock.socket() };
        }
    };
    var mock: Mock = .{};
    env.websocket_transport = .{ .userdata = &mock, .connect = Mock.connect };
    const F = struct {
        fn options(_: *const Env) fx.WebSocket.ConnectOptions {
            return .{ .uri = "ws://example.test", .headers = &.{}, .sub_protocols = &.{"example"}, .limits = .{ .maximum_message_bytes = 2, .receive_buffer_bytes = 1, .close_timeout = .fromMilliseconds(100) } };
        }
        fn use(connection: *fx.WebSocket.Connection) fx.Effect(fx.WebSocket.Receive, fx.WebSocket.Error, Env) {
            return fx.EffectWebSocket(Env).receive(connection);
        }
    };
    const program = comptime fx.EffectWebSocket(Env).withConnection(F.options, F.use);
    var received = fx.run(&env, program);
    defer received.deinit();
    try std.testing.expect(received == .success);
    try std.testing.expectEqualStrings("é", received.success.message.text.items);
    try std.testing.expectEqual(@as(usize, 1), mock.disposed);
    try std.testing.expectEqual(@as(usize, 1), mock.aborted);
}

test "manual clock sleep suspends runAsync and cancellation removes the sleeper" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    var env = harness.env();
    var clock = fx.ManualClock.init(&env, .{ .nanoseconds = 0 });
    defer clock.deinit();
    env.clock = clock.service();
    var pending = fx.runAsync(&env, Unit.sleep(.fromSeconds(1)));
    defer pending.deinit();
    var admitted = clock.awaitPending(1);
    defer admitted.deinit();
    try std.testing.expect(admitted == .success);
    try std.testing.expectEqual(@as(usize, 1), clock.pending());
    pending.request();
    var cancelled = pending.join();
    defer cancelled.deinit();
    try std.testing.expect(cancelled == .interrupted);
    try std.testing.expectEqual(@as(usize, 0), clock.pending());
}

// Failure injection shares an allocator across real workers. Serialize the
// counter as well as the backing allocator so each failure position is exact.
const LockedAllocator = struct {
    child: std.mem.Allocator,
    locked: std.atomic.Value(bool) = .init(false),
    fn enter(self: *@This()) void {
        while (self.locked.swap(true, .acquire)) std.atomic.spinLoopHint();
    }
    fn leave(self: *@This()) void {
        self.locked.store(false, .release);
    }
    fn allocator(self: *@This()) std.mem.Allocator {
        return .{ .ptr = self, .vtable = &.{ .alloc = alloc, .resize = resize, .remap = remap, .free = free } };
    }
    fn alloc(data: *anyopaque, len: usize, alignment: std.mem.Alignment, caller: usize) ?[*]u8 {
        const self: *@This() = @ptrCast(@alignCast(data));
        self.enter();
        defer self.leave();
        return self.child.rawAlloc(len, alignment, caller);
    }
    fn resize(data: *anyopaque, memory: []u8, alignment: std.mem.Alignment, len: usize, caller: usize) bool {
        const self: *@This() = @ptrCast(@alignCast(data));
        self.enter();
        defer self.leave();
        return self.child.rawResize(memory, alignment, len, caller);
    }
    fn remap(data: *anyopaque, memory: []u8, alignment: std.mem.Alignment, len: usize, caller: usize) ?[*]u8 {
        const self: *@This() = @ptrCast(@alignCast(data));
        self.enter();
        defer self.leave();
        return self.child.rawRemap(memory, alignment, len, caller);
    }
    fn free(data: *anyopaque, memory: []u8, alignment: std.mem.Alignment, caller: usize) void {
        const self: *@This() = @ptrCast(@alignCast(data));
        self.enter();
        defer self.leave();
        self.child.rawFree(memory, alignment, caller);
    }
};
fn managedLifecycle(allocator: std.mem.Allocator, comptime is_pool: bool) !void {
    var harness: Harness = .{};
    harness.init();
    harness.engine.allocator = allocator;
    defer harness.deinit();
    var env = harness.env();
    env.allocator = allocator;
    var scope = fx.Scope(E).init(&env);
    defer releaseScope(&scope, &env);
    if (is_pool) {
        const Pool = fx.EffectPool(u32, E, Env);
        var made = fx.run(&env, Pool.make(&scope, poolConfig, comptime P.fromOutcome(count), validate, dispose));
        defer made.deinit();
        if (made == .success) {
            var lease_scope = fx.Scope(E).init(&env);
            defer releaseScope(&lease_scope, &env);
            var acquisition = Pool.acquire(&lease_scope, made.success);
            defer acquisition.deinit();
            var lease = fx.run(&env, acquisition);
            defer lease.deinit();
            if (lease == .success) {
                var returned = fx.run(&env, Pool.release(lease.success));
                defer returned.deinit();
            } else try std.testing.expect(lease == .die);
        } else try std.testing.expect(made == .die);
    } else {
        const C = fx.EffectCache(u32, u32, E, Env);
        var made = fx.run(&env, C.make(&scope, cacheConfig, load));
        defer made.deinit();
        if (made == .success) {
            var request = C.get(made.success, 5);
            defer request.deinit();
            var loaded = fx.run(&env, request);
            defer loaded.deinit();
            try std.testing.expect(loaded == .success or loaded == .die);
        } else try std.testing.expect(made == .die);
    }
}
test "cache and pool reclaim every allocation position across acquisition and shutdown" {
    inline for (.{ false, true }) |is_pool| {
        var measured = std.testing.FailingAllocator.init(std.testing.allocator, .{});
        var locked: LockedAllocator = .{ .child = measured.allocator() };
        try managedLifecycle(locked.allocator(), is_pool);
        try std.testing.expectEqual(measured.allocated_bytes, measured.freed_bytes);
        for (0..measured.alloc_index) |index| {
            var failed = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = index });
            var serialized: LockedAllocator = .{ .child = failed.allocator() };
            try managedLifecycle(serialized.allocator(), is_pool);
            try std.testing.expectEqual(failed.allocated_bytes, failed.freed_bytes);
        }
    }
}

test "distinct CPU and blocking engines cross a barrier before either job returns" {
    const Barrier = struct {
        mutex: std.Io.Mutex = .init,
        arrived: usize = 0,
        ready: std.Io.Event = .unset,
        io: std.Io,
        fn meet(self: *@This()) bool {
            self.mutex.lockUncancelable(self.io);
            self.arrived += 1;
            if (self.arrived == 2) self.ready.set(self.io);
            self.mutex.unlock(self.io);
            const deadline = (std.Io.Timeout{ .duration = .{ .raw = .fromSeconds(5), .clock = .awake } }).toDeadline(self.io);
            while (!self.ready.isSet()) self.ready.waitTimeout(self.io, deadline) catch |err| switch (err) {
                error.Canceled => return false,
                error.Timeout => if (deadline.deadline.durationFromNow(self.io).raw.nanoseconds <= 0) return false,
            };
            return true;
        }
    };
    const R = struct { allocator: std.mem.Allocator, io: std.Io, async_engine: fx.Executor, parallel_engine: fx.Executor, blocking_engine: fx.Executor, barrier: *Barrier };
    const Program = fx.Effect(u32, E, R);
    const F = struct {
        fn work(environment: *const R, _: *const fx.Cancellation) fx.Exit(u32, E) {
            return if (environment.barrier.meet()) .{ .success = 1 } else .interrupted;
        }
    };
    var cpu: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = false }, .maximum_owned = 2 };
    defer cpu.deinit();
    var blocking: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = false, .blocking = true }, .maximum_owned = 2 };
    defer blocking.deinit();
    var async_engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = false, .blocking = false }, .maximum_owned = 4 };
    defer async_engine.deinit();
    var barrier: Barrier = .{ .io = std.testing.io };
    const environment: R = .{ .allocator = std.testing.allocator, .io = std.testing.io, .async_engine = async_engine.executor(), .parallel_engine = cpu.executor(), .blocking_engine = blocking.executor(), .barrier = &barrier };
    const programs = [_]Program{ comptime Program.onCpu(F.work), comptime Program.onBlocking(F.work) };
    var running = fx.runAsync(&environment, Program.allConcurrent(2, &programs));
    defer running.deinit();
    var outcome = running.join();
    defer outcome.deinit();
    try std.testing.expect(outcome == .success);
    try std.testing.expectEqual(@as(usize, 2), barrier.arrived);
    try std.testing.expectEqualSlices(u32, &.{ 1, 1 }, outcome.success.items);
}

test "layers provision typed dependencies and release partial construction" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    const env = harness.env();
    const Services = struct { left: *u32, right: *u32 };
    const Acquire = fx.Effect(*u32, E, Env);
    const F = struct {
        fn acquire(environment: *const Env) fx.Exit(*u32, E) {
            const allocated = environment.allocator.create(u32) catch return .{ .die = fx.Defect.out_of_memory };
            allocated.* = @intCast(environment.count.fetchAdd(1, .monotonic) + 1);
            return .{ .success = allocated };
        }
        fn release(pointer: *u32) Unit {
            return Unit.fromBorrowed(u32, pointer, struct {
                fn releaseResource(value: *const u32, environment: *const Env) fx.Exit(void, E) {
                    environment.allocator.destroy(@constCast(value));
                    _ = environment.released.fetchAdd(1, .monotonic);
                    return .{ .success = {} };
                }
            }.releaseResource);
        }
        fn combine(left: *u32, right: *u32) Services {
            return .{ .left = left, .right = right };
        }
        fn read(services: *const Services) u32 {
            return services.left.* + services.right.*;
        }
        fn middle(environment: *const Env) u32 {
            return @intCast(environment.count.load(.acquire));
        }
        fn doubleMiddle(value: *const u32) u32 {
            return value.* * 2;
        }
    };
    const L = fx.Layer(Env, E, *u32);
    const left = comptime L.scoped(Acquire.fromOutcome(F.acquire), F.release);
    const right = comptime L.scoped(Acquire.fromOutcome(F.acquire), F.release);
    const merged = comptime left.merge(right, F.combine);
    const program = comptime merged.provideTo(fx.Effect(u32, E, Services).service(F.read));
    var result = fx.run(&env, program);
    defer result.deinit();
    try std.testing.expectEqual(@as(u32, 3), result.success);
    try std.testing.expectEqual(@as(usize, 2), harness.released.load(.acquire));
    const failed = comptime left.merge(L.fromEffect(Acquire.fail(.expected)), F.combine).provideTo(fx.Effect(u32, E, Services).service(F.read));
    var failure = fx.run(&env, failed);
    defer failure.deinit();
    try std.testing.expectEqual(E.expected, failure.fail);
    try std.testing.expectEqual(@as(usize, 3), harness.released.load(.acquire));
    const middle = comptime fx.Layer(Env, E, u32).fromEffect(P.service(F.middle));
    const next = comptime fx.Layer(u32, E, u32).fromEffect(fx.Effect(u32, E, u32).service(F.doubleMiddle));
    var scope = fx.Scope(E).init(&env);
    defer releaseScope(&scope, &env);
    try std.testing.expectEqual(@as(u32, 6), fx.run(&env, (comptime middle.provide(next)).build(&scope)).success);
}

test "schedules use virtual time, fresh state and the selected exit channel" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    var env = harness.env();
    var clock = fx.ManualClock.init(&env, .{ .nanoseconds = 0 });
    defer clock.deinit();
    env.clock = clock.service();
    const policy = comptime fx.Schedule.intersect(fx.Schedule.recurs(E, 1), fx.Schedule.spaced(E, .fromSeconds(1)));
    const program = comptime P.fromOutcome(failOnce).retry(policy);
    var running = fx.runAsync(&env, program);
    defer running.deinit();
    var admitted = clock.awaitPending(1);
    defer admitted.deinit();
    try std.testing.expect(admitted == .success);
    var advanced = clock.advance(.fromSeconds(1));
    defer advanced.deinit();
    try std.testing.expect(advanced == .success);
    var completed = running.join();
    defer completed.deinit();
    try std.testing.expectEqual(@as(u32, 3), completed.success);
    try std.testing.expectEqual(@as(usize, 0), clock.pending());
    const repeats = comptime P.fromOutcome(count).repeat(fx.Schedule.recurs(u32, 2));
    try std.testing.expectEqual(@as(u32, 4), fx.run(&env, repeats).success);
    try std.testing.expectEqual(@as(u32, 7), fx.run(&env, repeats).success);
    const defective = comptime P.die(fx.Defect.init("fixture")).retry(policy);
    try std.testing.expect(fx.run(&env, defective) == .die);
    var jitter = fx.Schedule.jitter(0.5, 1.5, fx.Schedule.exponential(E, .fromSeconds(1), 2));
    try std.testing.expectEqual(@as(i96, std.time.ns_per_s), jitter.step(.expected, .zero, 0.5).continue_with.delay.nanoseconds);
    try std.testing.expectEqual(@as(i96, std.time.ns_per_s * 3), jitter.step(.expected, .zero, 1).continue_with.delay.nanoseconds);
    try std.testing.expect(jitter.step(.expected, .zero, std.math.nan(f64)) == .invalid);
}

test "tracing crosses worker boundaries, restores annotations and isolates exporter failures" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    var env = harness.env();
    const Recorder = struct {
        starts: usize = 0,
        ends: usize = 0,
        abandoned: usize = 0,
        diagnostics: usize = 0,
        lineage: bool = true,
        visible: bool = true,
        logs: usize = 0,
        fn begin(data: ?*anyopaque, start: fx.SpanStart) fx.Exit(fx.SpanId, fx.Defect) {
            const recorder: *@This() = @ptrCast(@alignCast(data.?));
            recorder.starts += 1;
            if (recorder.starts == 1) recorder.lineage = start.parent == null else recorder.lineage = recorder.lineage and start.parent != null and start.parent.?.value == 1;
            var annotations = start.annotations.iterator();
            const annotation = annotations.next();
            recorder.visible = recorder.visible and annotation != null and std.mem.eql(u8, annotation.?.key, "request") and annotations.next() == null;
            return .{ .success = .{ .value = recorder.starts } };
        }
        fn end(data: ?*anyopaque, finish: fx.SpanFinish) fx.Exit(void, fx.Defect) {
            const recorder: *@This() = @ptrCast(@alignCast(data.?));
            recorder.ends += 1;
            recorder.lineage = recorder.lineage and finish.outcome == .succeeded;
            return .{ .fail = fx.Defect.init("export fixture") };
        }
        fn abandon(data: ?*anyopaque, _: fx.SpanId) fx.Exit(void, fx.Defect) {
            const recorder: *@This() = @ptrCast(@alignCast(data.?));
            recorder.abandoned += 1;
            return .{ .success = {} };
        }
        fn diagnostic(data: ?*anyopaque, _: fx.Defect) void {
            const recorder: *@This() = @ptrCast(@alignCast(data.?));
            recorder.diagnostics += 1;
        }
        fn redact(_: ?*anyopaque, annotation: fx.Annotation) ?fx.Annotation {
            return if (std.mem.eql(u8, annotation.key, "secret")) null else annotation;
        }
        fn log(data: ?*anyopaque, record: fx.LogRecord) fx.Exit(void, fx.Defect) {
            const recorder: *@This() = @ptrCast(@alignCast(data.?));
            recorder.logs += 1;
            var annotations = record.annotations.iterator();
            if (recorder.logs == 1) {
                const annotation = annotations.next();
                recorder.visible = recorder.visible and record.span != null and annotation != null and std.mem.eql(u8, annotation.?.key, "request") and annotations.next() == null;
            } else recorder.visible = recorder.visible and record.span == null and annotations.next() == null;
            return .{ .die = fx.Defect.init("logger fixture") };
        }
    };
    var recorder: Recorder = .{};
    const diagnostics: fx.DiagnosticSink = .{ .userdata = &recorder, .report_fn = Recorder.diagnostic };
    const redactor: fx.Redactor = .{ .userdata = null, .redact_fn = Recorder.redact };
    env.tracer = .{ .userdata = &recorder, .vtable = &.{ .begin = Recorder.begin, .end = Recorder.end, .abandon = Recorder.abandon }, .redactor = redactor, .diagnostics = diagnostics };
    env.logger = .{ .userdata = &recorder, .minimum = .info, .redactor = redactor, .diagnostics = diagnostics, .write_fn = Recorder.log };
    const program = comptime Unit.log(.info, "inside").withSpan("child").dispatch(.cpu).withSpan("parent").annotate(&.{ .{ .key = "request", .value = "fixture" }, .{ .key = "secret", .value = "credential" } });
    var result = fx.run(&env, program);
    defer result.deinit();
    try std.testing.expect(result == .success);
    try std.testing.expect(fx.run(&env, comptime Unit.log(.info, "outside")) == .success);
    try std.testing.expect(recorder.lineage and recorder.visible);
    try std.testing.expectEqual(@as(usize, 2), recorder.starts);
    try std.testing.expectEqual(recorder.starts, recorder.ends);
    try std.testing.expectEqual(recorder.ends, recorder.abandoned);
    try std.testing.expectEqual(@as(usize, 4), recorder.diagnostics);
}

test "bounded traversal drains admitted work when the engine rejects a later submission" {
    var harness: Harness = .{};
    harness.init();
    harness.engine.maximum_owned = 1;
    defer harness.deinit();
    const env = harness.env();
    const F = struct {
        fn work(environment: *const Env, _: *const fx.Cancellation) fx.Exit(u32, E) {
            environment.io.sleep(.fromSeconds(5), .awake) catch return .interrupted;
            return .{ .success = 1 };
        }
    };
    const worker = comptime P.onCpu(F.work);
    const programs = [_]P{ worker, worker };
    var result = fx.run(&env, P.allConcurrent(2, &programs));
    defer result.deinit();
    try std.testing.expect(result.primary() == .defected);
    try std.testing.expectEqual(fx.Defect.Kind.provider_unavailable, result.primary().defected.kind);
    try std.testing.expectEqual(@as(usize, 0), harness.engine.owned.load(.acquire));
}

test "native HTTP performs a loopback request through the supplied client and I/O" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    var env = harness.env();
    const address: std.Io.net.IpAddress = .{ .ip4 = .loopback(0) };
    var listener = try address.listen(env.io, .{});
    defer listener.deinit(env.io);
    const Peer = struct {
        listener: *std.Io.net.Server,
        io: std.Io,
        result: ?anyerror = null,
        fn run(peer: *@This()) void {
            peer.serve() catch |err| {
                peer.result = err;
            };
        }
        fn serve(peer: *@This()) !void {
            const connection = try peer.listener.accept(peer.io);
            defer connection.close(peer.io);
            var input: [2048]u8 = undefined;
            var output: [2048]u8 = undefined;
            var reader = connection.reader(peer.io, &input);
            var writer = connection.writer(peer.io, &output);
            var server = std.http.Server.init(&reader.interface, &writer.interface);
            var request = try server.receiveHead();
            try request.respond("native-body", .{ .keep_alive = false });
        }
    };
    var peer: Peer = .{ .listener = &listener, .io = env.io };
    var serving = try env.io.concurrent(Peer.run, .{&peer});
    defer serving.cancel(env.io);
    const uri = try std.fmt.allocPrint(env.allocator, "http://127.0.0.1:{d}/", .{listener.socket.address.ip4.port});
    defer env.allocator.free(uri);
    env.json = uri;
    var client: std.http.Client = .{ .allocator = env.allocator, .io = env.io };
    defer client.deinit();
    var native: fx.Http.Native = .{ .client = &client, .options = .{}, .redirect_buffer_bytes = 2048, .transfer_buffer_bytes = 2048, .maximum_body_bytes = 64 };
    env.http_transport = native.transport();
    const F = struct {
        fn request(environment: *const Env) fx.Http.Request {
            return .{ .uri = environment.json, .method = .GET, .headers = &.{}, .body = null };
        }
        fn accept(status: std.http.Status) bool {
            return status == .ok;
        }
    };
    var result = fx.run(&env, comptime fx.Fetch(Env).fetchText(F.request, F.accept));
    defer result.deinit();
    serving.await(env.io);
    try std.testing.expectEqual(@as(?anyerror, null), peer.result);
    try std.testing.expect(result == .success);
    try std.testing.expectEqualStrings("native-body", result.success.items);
}

test "application workflow composes schema, layers, tracing, bounded workers and retry" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    var env = harness.env();
    const Spans = struct {
        opened: std.atomic.Value(usize) = .init(0),
        ended: std.atomic.Value(usize) = .init(0),
        fn begin(data: ?*anyopaque, _: fx.SpanStart) fx.Exit(fx.SpanId, fx.Defect) {
            const self: *@This() = @ptrCast(@alignCast(data.?));
            return .{ .success = .{ .value = self.opened.fetchAdd(1, .monotonic) + 1 } };
        }
        fn end(data: ?*anyopaque, _: fx.SpanFinish) fx.Exit(void, fx.Defect) {
            const self: *@This() = @ptrCast(@alignCast(data.?));
            _ = self.ended.fetchAdd(1, .monotonic);
            return .{ .success = {} };
        }
        fn abandon(_: ?*anyopaque, _: fx.SpanId) fx.Exit(void, fx.Defect) {
            return .{ .success = {} };
        }
        fn diagnostic(_: ?*anyopaque, _: fx.Defect) void {}
    };
    var spans: Spans = .{};
    env.tracer = .{ .userdata = &spans, .vtable = &.{ .begin = Spans.begin, .end = Spans.end, .abandon = Spans.abandon }, .redactor = .identity, .diagnostics = .{ .userdata = null, .report_fn = Spans.diagnostic } };
    const F = struct {
        fn text(environment: *const Env) []const u8 {
            return environment.json;
        }
        fn syntax(_: std.json.ParseError(std.json.Scanner)) E {
            return .syntax;
        }
        fn decode(value: std.json.Value) ?u32 {
            return if (value == .integer and value.integer > 0) std.math.cast(u32, value.integer) else null;
        }
        fn invalid(_: std.json.Value) E {
            return .shape;
        }
        fn work(_: *const Env, _: *const fx.Cancellation) fx.Exit(u32, E) {
            return .{ .success = 2 };
        }
        fn sum(values: fx.OwnedSlice(u32)) u32 {
            var owned = values;
            defer owned.deinit();
            var total: u32 = 0;
            for (owned.items) |item| total += item;
            return total;
        }
        fn validated(parsed: fx.Parsed(u32)) P {
            var document = parsed;
            defer document.deinit();
            if (document.value == 0) return P.fail(.shape);
            const tasks = comptime [_]P{ P.onCpu(work).withSpan("CPU"), P.onBlocking(work).withSpan("blocking") };
            return comptime P.allConcurrent(2, &tasks).map(sum);
        }
        fn validatedAfterRetry(_: u32) P {
            return comptime fx.Schema(u32, E).fromOptionDecoder(decode, invalid).parse(Env, text, syntax).flatMap(validated);
        }
        fn provision(environment: *const Env) Env {
            return environment.*;
        }
        fn release(_: Env) Unit {
            return Unit.sync(struct {
                fn close(environment: *const Env) void {
                    _ = environment.released.fetchAdd(1, .monotonic);
                }
            }.close);
        }
    };
    const program = comptime P.fromOutcome(failOnce).retry(fx.Schedule.recurs(E, 1)).flatMap(F.validatedAfterRetry).withSpan("request");
    const layer = comptime fx.Layer(Env, E, Env).scoped(fx.Effect(Env, E, Env).service(F.provision), F.release);
    const application = comptime layer.provideTo(program);
    var result = fx.run(&env, application);
    defer result.deinit();
    try std.testing.expectEqual(@as(u32, 4), result.success);
    try std.testing.expectEqual(@as(usize, 1), harness.released.load(.acquire));
    try std.testing.expectEqual(spans.opened.load(.acquire), spans.ended.load(.acquire));
    try std.testing.expectEqual(@as(usize, 0), harness.engine.owned.load(.acquire));
    env.json = "{}";
    var invalid = fx.run(&env, application);
    defer invalid.deinit();
    try std.testing.expectEqual(E.shape, invalid.fail);
    try std.testing.expectEqual(spans.opened.load(.acquire), spans.ended.load(.acquire));
    try std.testing.expectEqual(@as(usize, 2), harness.released.load(.acquire));
}

test "shared scope closure suspends both callers until protected cleanup completes" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    var env = harness.env();
    var clock = fx.ManualClock.init(&env, .zero);
    defer clock.deinit();
    env.clock = clock.service();
    var scope = fx.Scope(E).init(&env);
    defer scope.deinit();
    const F = struct {
        fn release(_: void) Unit {
            return Unit.sleep(.fromSeconds(1));
        }
    };
    var acquired = fx.run(&env, (comptime Unit.succeed({})).scopeAcquire(&scope, F.release));
    defer acquired.deinit();
    try std.testing.expect(acquired == .success);
    var first = fx.runAsync(&env, Unit.closeScope(&scope));
    defer first.deinit();
    var ready = clock.awaitPending(1);
    defer ready.deinit();
    try std.testing.expect(ready == .success);
    var second = fx.runAsync(&env, Unit.closeScope(&scope));
    defer second.deinit();
    try std.testing.expect(first == .pending and second == .pending);
    var advanced = clock.advance(.fromSeconds(1));
    defer advanced.deinit();
    var closed_first = first.join();
    defer closed_first.deinit();
    var closed_second = second.join();
    defer closed_second.deinit();
    try std.testing.expect(closed_first == .success and closed_second == .success);
    try std.testing.expectEqual(@as(usize, 0), clock.pending());
}

test "timeout publishes its typed failure only after the virtual sleeper loser drains" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    var env = harness.env();
    var clock = fx.ManualClock.init(&env, .zero);
    defer clock.deinit();
    env.clock = clock.service();
    const F = struct {
        fn wait(_: *const void, environment: *const Env) fx.Exit(u32, E) {
            return switch (environment.clock.sleep(.fromSeconds(10))) {
                .done => .{ .success = 1 },
                .interrupted => .interrupted,
                .defect => |defect| .{ .die = defect },
            };
        }
        fn timeout() E {
            return .expected;
        }
    };
    const program = comptime P.fromNativeBorrowed(void, &{}, F.wait).timeout(.fromSeconds(1), F.timeout);
    var running = fx.runAsync(&env, program);
    defer running.deinit();
    var admitted = clock.awaitPending(2);
    defer admitted.deinit();
    try std.testing.expect(admitted == .success);
    var advanced = clock.advance(.fromSeconds(1));
    defer advanced.deinit();
    var timed_out = running.join();
    defer timed_out.deinit();
    try std.testing.expectEqual(E.expected, timed_out.fail);
    try std.testing.expectEqual(@as(usize, 0), clock.pending());
    try std.testing.expectEqual(@as(usize, 0), harness.engine.owned.load(.acquire));
}

test "resource builder Using reuses the ownership installed by Bind" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    const env = harness.env();
    const Build = fx.EffectBuilder(E, Env);
    const F = struct {
        fn release(_: u32, _: fx.PrimaryOutcome(E)) Unit {
            return Unit.sync(struct {
                fn close(environment: *const Env) void {
                    _ = environment.released.fetchAdd(1, .monotonic);
                }
            }.close);
        }
        fn body(value: fx.ResourceValue(u32)) P {
            return Build.using(value, use);
        }
        fn use(value: fx.ResourceValue(u32)) P {
            return P.succeed(value.value);
        }
    };
    const binding = comptime fx.resource(P.succeed(7), F.release);
    const program = comptime Build.bind(binding, F.body);
    try std.testing.expectEqual(@as(u32, 7), fx.run(&env, program).success);
    try std.testing.expectEqual(@as(usize, 1), harness.released.load(.acquire));
}

test "native console uses the consumer's injected writers" {
    var output = std.Io.Writer.Allocating.init(std.testing.allocator);
    defer output.deinit();
    var errors = std.Io.Writer.Allocating.init(std.testing.allocator);
    defer errors.deinit();
    var native: fx.NativeConsole = .{ .output = &output.writer, .errors = &errors.writer };
    const R = struct { console: fx.Console };
    const env: R = .{ .console = native.service() };
    const message = struct {
        fn text(_: *const R) []const u8 {
            return "fixture";
        }
    }.text;
    var written = fx.run(&env, comptime fx.EffectConsole(R).write(message));
    defer written.deinit();
    var error_line = fx.run(&env, comptime fx.EffectConsole(R).err(message));
    defer error_line.deinit();
    try std.testing.expect(written == .success and error_line == .success);
    try std.testing.expectEqualStrings("fixture\n", output.written());
    try std.testing.expectEqualStrings("fixture\n", errors.written());
}

test "builder loops and finally share the typed kernel" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    const env = harness.env();
    const B = fx.EffectBuilder(E, Env);
    const F = struct {
        fn guard(environment: *const Env) bool {
            return environment.count.load(.acquire) < 1000;
        }
        fn step(environment: *const Env) void {
            _ = environment.count.fetchAdd(1, .monotonic);
        }
        fn release(environment: *const Env) void {
            _ = environment.released.fetchAdd(1, .monotonic);
        }
        fn item(value: u32) Unit {
            return if (value == 2) Unit.fail(.expected) else Unit.succeed({});
        }
    };
    const loop = comptime B.tryFinally(B.whileLoop(F.guard, Unit.sync(F.step)), F.release);
    var completed = fx.run(&env, loop);
    defer completed.deinit();
    try std.testing.expect(completed == .success);
    try std.testing.expectEqual(@as(usize, 1000), harness.count.load(.acquire));
    var failed = fx.run(&env, B.forLoop(u32, &.{ 1, 2, 3 }, F.item));
    defer failed.deinit();
    try std.testing.expectEqual(E.expected, failed.fail);
    try std.testing.expectEqual(@as(usize, 1), harness.released.load(.acquire));
}

test "async iterable streams own and release their native iterator" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    const env = harness.env();
    const Iterator = struct { index: u32 = 0 };
    const F = struct {
        fn create(environment: *const Env) fx.Exit(*Iterator, E) {
            const iterator = environment.allocator.create(Iterator) catch return .{ .die = fx.Defect.out_of_memory };
            iterator.* = .{};
            return .{ .success = iterator };
        }
        fn read(iterator: *const Iterator, _: *const Env) fx.Exit(?u32, E) {
            const mutable = @constCast(iterator);
            if (mutable.index == 3) return .{ .success = null };
            mutable.index += 1;
            return .{ .success = mutable.index };
        }
        fn next(iterator: *const Iterator, environment: *const Env) fx.Async(?u32, E).Start {
            return fx.Async(?u32, E).nativeBorrowed(environment, iterator, read);
        }
        fn release(iterator: *Iterator) Unit {
            return Unit.fromBorrowed(Iterator, iterator, struct {
                fn destroy(value: *const Iterator, environment: *const Env) fx.Exit(void, E) {
                    environment.allocator.destroy(@constCast(value));
                    _ = environment.released.fetchAdd(1, .monotonic);
                    return .{ .success = {} };
                }
            }.destroy);
        }
    };
    const stream = comptime fx.Stream(u32, E, Env).fromAsyncIterable(Iterator, F.create, F.next, F.release);
    var running = fx.runAsync(&env, stream.runCollect(3));
    defer running.deinit();
    var collected = running.join();
    defer collected.deinit();
    try std.testing.expectEqualSlices(u32, &.{ 1, 2, 3 }, collected.success.items);
    try std.testing.expectEqual(@as(usize, 1), harness.released.load(.acquire));
}

test "queue shutdown disposes owned buffered values and latch opens once" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    const env = harness.env();
    var scope = fx.Scope(E).init(&env);
    defer releaseScope(&scope, &env);
    const L = fx.EffectLatch(E, Env);
    const latch = fx.run(&env, L.make(&scope)).success;
    var opened = fx.run(&env, L.open(latch));
    defer opened.deinit();
    try std.testing.expect(opened.success);
    var reopened = fx.run(&env, L.open(latch));
    defer reopened.deinit();
    try std.testing.expect(!reopened.success);
    var awaited = fx.run(&env, L.awaitValue(latch));
    defer awaited.deinit();
    try std.testing.expect(awaited == .success);
    const Bytes = fx.OwnedSlice(u8);
    var queue = fx.Queue(Bytes).init(&env, 2).success;
    defer queue.deinit();
    const first = Bytes{ .items = try env.allocator.dupe(u8, "first"), .allocator = env.allocator };
    var offered = queue.offerToken(first, null);
    defer offered.deinit();
    try std.testing.expect(offered == .success);
    const second = Bytes{ .items = try env.allocator.dupe(u8, "second"), .allocator = env.allocator };
    var another = queue.offerToken(second, null);
    defer another.deinit();
    try std.testing.expect(another == .success);
    queue.shutdown(false);
    var taken = queue.takeToken(null);
    defer taken.deinit();
    try std.testing.expectEqual(fx.QueueError.shutdown, taken.fail);
}

test "WebSocket timeout drains the handshake and retains its cleanup failure" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    var env = harness.env();
    var clock = fx.ManualClock.init(&env, .zero);
    defer clock.deinit();
    env.clock = clock.service();
    const Mock = struct {
        io: std.Io,
        aborted: std.Io.Event = .unset,
        disposed: bool = false,
        fn socket(self: *@This()) fx.WebSocket.NativeSocket {
            return .{ .userdata = self, .vtable = &.{ .send = send, .receive = receive, .close_output = handshake, .close = handshake, .abort = abort, .dispose = disposeSocket, .sub_protocol = protocol, .is_open = isOpen } };
        }
        fn send(_: *anyopaque, _: fx.WebSocket.Kind, _: []const u8, _: ?*const fx.Cancellation) fx.Exit(void, fx.WebSocket.Error) {
            return .{ .success = {} };
        }
        fn receive(_: *anyopaque, _: []u8, _: ?*const fx.Cancellation) fx.Exit(fx.WebSocket.Fragment, fx.WebSocket.Error) {
            return .interrupted;
        }
        fn handshake(data: *anyopaque, _: ?*const fx.Cancellation) fx.Exit(void, fx.WebSocket.Error) {
            const mock: *@This() = @ptrCast(@alignCast(data));
            mock.aborted.waitUncancelable(mock.io);
            return .{ .finalization_failed = .{ .primary = .was_interrupted, .first = .{ .release_defect = fx.Defect.init("handshake cleanup") } } };
        }
        fn abort(data: *anyopaque) ?fx.Defect {
            const mock: *@This() = @ptrCast(@alignCast(data));
            mock.aborted.set(mock.io);
            return null;
        }
        fn disposeSocket(data: *anyopaque) fx.Exit(void, fx.WebSocket.Error) {
            const mock: *@This() = @ptrCast(@alignCast(data));
            mock.disposed = true;
            return .{ .success = {} };
        }
        fn protocol(_: *anyopaque) ?[]const u8 {
            return null;
        }
        fn isOpen(_: *anyopaque) bool {
            return true;
        }
    };
    var mock: Mock = .{ .io = env.io };
    const F = struct {
        fn limits(_: *const Env) fx.WebSocket.Limits {
            return .{ .maximum_message_bytes = 8, .receive_buffer_bytes = 8, .close_timeout = .fromSeconds(1) };
        }
        fn use(_: *fx.WebSocket.Connection) fx.Effect(void, fx.WebSocket.Error, Env) {
            return fx.Effect(void, fx.WebSocket.Error, Env).succeed({});
        }
    };
    env.websocket_transport = .{ .userdata = &mock, .connect = struct {
        fn connect(data: *anyopaque, _: fx.WebSocket.ConnectOptions, _: ?*const fx.Cancellation) fx.Exit(fx.WebSocket.NativeSocket, fx.WebSocket.Error) {
            const value: *Mock = @ptrCast(@alignCast(data));
            return .{ .success = value.socket() };
        }
    }.connect };
    const options = struct {
        fn select(environment: *const Env) fx.WebSocket.ConnectOptions {
            return .{ .uri = "ws://fixture.test", .headers = &.{}, .sub_protocols = &.{}, .limits = F.limits(environment) };
        }
    }.select;
    const program = comptime fx.EffectWebSocket(Env).withConnection(options, F.use);
    var running = fx.runAsync(&env, program);
    defer running.deinit();
    var ready = clock.awaitPending(1);
    defer ready.deinit();
    try std.testing.expect(ready == .success);
    var advanced = clock.advance(.fromSeconds(1));
    defer advanced.deinit();
    var result = running.join();
    defer result.deinit();
    try std.testing.expect(result == .finalization_failed);
    try std.testing.expect(result.finalization_failed.primary == .succeeded);
    var failures = result.finalization_failed.iterator();
    try std.testing.expect(failures.nextFailure().?.release_failure == .close_timeout);
    try std.testing.expectEqualStrings("handshake cleanup", failures.nextFailure().?.release_defect.message);
    try std.testing.expect(failures.nextFailure() == null);
    try std.testing.expect(mock.disposed);
    try std.testing.expectEqual(@as(usize, 0), clock.pending());
}

test "population observes factories already committed to borrowers" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    const Fixture = struct { started: std.Io.Event = .unset, proceed: std.Io.Event = .unset };
    var fixture: Fixture = .{};
    const R = struct {
        allocator: std.mem.Allocator,
        io: std.Io,
        cancellation: *fx.Cancellation,
        async_engine: fx.Executor,
        fixture: *Fixture,
    };
    const env: R = .{ .allocator = std.testing.allocator, .io = harness.io, .cancellation = &harness.cancellation, .async_engine = harness.engine.executor(), .fixture = &fixture };
    const Pool = fx.EffectPool(u32, E, R);
    const F = struct {
        fn config(_: *const R) fx.PoolConfig {
            return .{ .capacity = 1, .idle_time_to_live = null };
        }
        fn create(environment: *const R) fx.Exit(u32, E) {
            environment.fixture.started.set(environment.io);
            environment.fixture.proceed.waitUncancelable(environment.io);
            return .{ .fail = .expected };
        }
        fn validate(_: u32) fx.Effect(bool, E, R) {
            return fx.Effect(bool, E, R).succeed(true);
        }
        fn dispose(_: u32) fx.Effect(void, E, R) {
            return fx.Effect(void, E, R).succeed({});
        }
    };
    var scope = fx.Scope(E).init(&env);
    defer scope.deinit();
    const pool = fx.run(&env, Pool.make(&scope, F.config, fx.Effect(u32, E, R).fromOutcome(F.create), F.validate, F.dispose)).success;
    var leases = fx.Scope(E).init(&env);
    defer leases.deinit();
    var acquisition = Pool.acquire(&leases, pool);
    defer acquisition.deinit();
    var borrower = fx.runAsync(&env, acquisition);
    defer borrower.deinit();
    // Ensure every failure path releases the test's native wait before teardown.
    defer fixture.proceed.set(env.io);
    fixture.started.waitUncancelable(env.io);
    var description = Pool.ensureSize(pool, 1);
    defer description.deinit();
    var population = fx.runAsync(&env, description);
    defer population.deinit();
    pool.mutex.lockUncancelable(env.io);
    while (pool.borrower_creations.?.references.load(.acquire) < 3)
        pool.changed.waitUncancelable(env.io, &pool.mutex);
    pool.mutex.unlock(env.io);
    fixture.proceed.set(env.io);
    var populated = population.join();
    defer populated.deinit();
    var acquired = borrower.join();
    defer acquired.deinit();
    try std.testing.expectEqual(E.expected, populated.fail);
    try std.testing.expectEqual(E.expected, acquired.fail);
    var closed_leases = fx.run(&env, fx.Effect(void, E, R).closeScope(&leases));
    defer closed_leases.deinit();
    var closed_pool = fx.run(&env, fx.Effect(void, E, R).closeScope(&scope));
    defer closed_pool.deinit();
    try std.testing.expect(closed_leases == .success and closed_pool == .success);
}

test "failed observer copies preserve the source's owning outcome" {
    var failing = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 1 });
    const allocator = failing.allocator();
    const Bytes = fx.OwnedSlice(u8);
    var source: fx.Exit(void, Bytes) = .{ .fail = .{ .items = try allocator.dupe(u8, "owned error"), .allocator = allocator } };
    defer source.deinit();
    const env = .{ .io = std.testing.io };
    var deferred = fx.Deferred(void, Bytes).init(&env);
    defer deferred.deinit();
    try std.testing.expect(deferred.completeCopy(source));
    var observed = deferred.waitObserved(null);
    defer observed.exit.deinit();
    try std.testing.expect(observed.clone_failed);
    try std.testing.expect(observed.exit == .die);
    try std.testing.expectEqualStrings("owned error", source.fail.items);
}

test "runtime worker captures use selected engines with bounded admission" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    var cpu: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = harness.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = false }, .maximum_owned = 2 };
    defer cpu.deinit();
    var blocking: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = harness.io, .capabilities = .{ .independent = true, .cpu = false, .blocking = true }, .maximum_owned = 2 };
    defer blocking.deinit();
    var env = harness.env();
    env.parallel_engine = cpu.executor();
    env.blocking_engine = blocking.executor();
    const Fixture = struct {
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        changed: std.Io.Condition = .init,
        proceed: std.Io.Event = .unset,
        first_two: std.Io.Event = .unset,
        active: usize = 0,
        peak: usize = 0,
        started: usize = 0,
    };
    const Work = struct { fixture: *Fixture, input: u32, kind: fx.ExecutionKind };
    const F = struct {
        fn execute(work: *const Work, _: *const Env, _: *const fx.Cancellation) fx.Exit(u32, E) {
            const fixture = work.fixture;
            fixture.mutex.lockUncancelable(fixture.io);
            fixture.active += 1;
            fixture.started += 1;
            if (fixture.started == 2) fixture.first_two.set(fixture.io);
            fixture.peak = @max(fixture.active, fixture.peak);
            fixture.changed.broadcast(fixture.io);
            fixture.mutex.unlock(fixture.io);
            fixture.proceed.waitUncancelable(fixture.io);
            fixture.mutex.lockUncancelable(fixture.io);
            fixture.active -= 1;
            fixture.mutex.unlock(fixture.io);
            return .{ .success = work.input * 2 };
        }
        fn program(work: *const Work) P {
            return switch (work.kind) {
                .cpu => P.onCpuBorrowed(Work, work, execute),
                .blocking => P.onBlockingBorrowed(Work, work, execute),
                else => unreachable,
            };
        }
    };
    var fixture: Fixture = .{ .io = env.io };
    const work = [_]Work{
        .{ .fixture = &fixture, .input = 1, .kind = .cpu },
        .{ .fixture = &fixture, .input = 2, .kind = .blocking },
        .{ .fixture = &fixture, .input = 3, .kind = .cpu },
        .{ .fixture = &fixture, .input = 4, .kind = .blocking },
    };
    const inputs = [_]*const Work{ &work[0], &work[1], &work[2], &work[3] };
    const description = P.forEachConcurrent(*const Work, 2, &inputs, F.program);
    try std.testing.expectEqual(@as(usize, 0), fixture.started);
    var running = fx.runAsync(&env, description);
    defer running.deinit();
    defer fixture.proceed.set(env.io);
    try fixture.first_two.waitTimeout(env.io, .{ .duration = .{ .raw = .fromSeconds(5), .clock = .awake } });
    fixture.proceed.set(env.io);
    var result = running.join();
    defer result.deinit();
    try std.testing.expectEqualSlices(u32, &.{ 2, 4, 6, 8 }, result.success.items);
    try std.testing.expectEqual(@as(usize, 2), fixture.peak);
    try std.testing.expectEqual(@as(usize, 4), fixture.started);
    try std.testing.expectEqual(@as(usize, 0), fixture.active);
}

test "cache and pool TTL use the injected clock" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    var env = harness.env();
    var clock = fx.ManualClock.init(&env, .zero);
    defer clock.deinit();
    env.clock = clock.service();
    const F = struct {
        fn cacheConfigTtl(environment: *const Env) fx.CacheConfig(u32, E) {
            var config = cacheConfig(environment);
            config.time_to_live = .fromSeconds(1);
            return config;
        }
        fn loadCount(_: u32) P {
            return P.fromOutcome(count);
        }
        fn poolConfigTtl(_: *const Env) fx.PoolConfig {
            return .{ .capacity = 1, .idle_time_to_live = .fromSeconds(1) };
        }
        fn any(_: u32) bool {
            return true;
        }
    };
    var scope = fx.Scope(E).init(&env);
    defer releaseScope(&scope, &env);
    const C = fx.EffectCache(u32, u32, E, Env);
    const cache = fx.run(&env, C.make(&scope, F.cacheConfigTtl, F.loadCount)).success;
    var get = C.get(cache, 5);
    defer get.deinit();
    try std.testing.expectEqual(@as(u32, 0), fx.run(&env, get).success);
    try std.testing.expectEqual(@as(u32, 0), fx.run(&env, get).success);
    var advance = clock.advance(.fromSeconds(1));
    defer advance.deinit();
    try std.testing.expectEqual(@as(u32, 1), fx.run(&env, get).success);
    const Pool = fx.EffectPool(u32, E, Env);
    const pool = fx.run(&env, Pool.make(&scope, F.poolConfigTtl, P.fromOutcome(count), validate, dispose)).success;
    var leases = fx.Scope(E).init(&env);
    defer releaseScope(&leases, &env);
    var acquire = Pool.acquire(&leases, pool);
    defer acquire.deinit();
    const first = fx.run(&env, acquire).success;
    try std.testing.expectEqual(@as(u32, 2), Pool.value(first));
    var released = fx.run(&env, Pool.release(first));
    defer released.deinit();
    var idle = fx.run(&env, Pool.existsIdle(pool, F.any));
    defer idle.deinit();
    try std.testing.expect(idle.success);
    var later = clock.advance(.fromSeconds(1));
    defer later.deinit();
    const second = fx.run(&env, acquire).success;
    try std.testing.expectEqual(@as(u32, 3), Pool.value(second));
    try std.testing.expectEqual(@as(usize, 1), harness.released.load(.acquire));
    var returned = fx.run(&env, Pool.release(second));
    defer returned.deinit();
}

test "native adapters and outcome conversions preserve channel selection" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    const env = harness.env();
    const F = struct {
        fn none() E {
            return .expected;
        }
        fn nativeFailure(_: *const Env) error{Fixture}!u32 {
            return error.Fixture;
        }
        fn nativeSuccess(_: *const Env) error{Fixture}!u32 {
            return 21;
        }
        fn errorMap(_: error{Fixture}) E {
            return .expected;
        }
        fn defectMap(_: fx.Defect) E {
            return .expected;
        }
        fn release(environment: *const Env) void {
            _ = environment.released.fetchAdd(1, .monotonic);
        }
    };
    try std.testing.expectEqual(@as(u32, 7), fx.run(&env, P.fromResult(.{ .ok = 7 })).success);
    try std.testing.expectEqual(E.expected, fx.run(&env, P.fromOption(null, F.none)).fail);
    const failed = comptime P.trySync(F.nativeFailure, F.errorMap);
    try std.testing.expectEqual(E.expected, fx.run(&env, failed).fail);
    const native = comptime P.tryNative(F.nativeSuccess, F.errorMap);
    var running = fx.runAsync(&env, native);
    defer running.deinit();
    var completed = running.join();
    defer completed.deinit();
    try std.testing.expectEqual(@as(u32, 21), completed.success);
    const either = comptime P.fail(.expected).either();
    var converted = fx.run(&env, either);
    defer converted.deinit();
    try std.testing.expectEqual(E.expected, converted.success.err);
    const option = comptime P.fail(.expected).option();
    try std.testing.expect(fx.run(&env, option).success == null);
    const sandbox = comptime P.die(fx.Defect.init("fixture")).sandbox(F.defectMap);
    try std.testing.expectEqual(E.expected, fx.run(&env, sandbox).fail);
    const interrupted = comptime Unit.interrupted().onInterrupt(Unit.sync(F.release));
    var cleaned = fx.run(&env, interrupted);
    defer cleaned.deinit();
    try std.testing.expect(cleaned == .interrupted);
    try std.testing.expectEqual(@as(usize, 1), harness.released.load(.acquire));
}

test "borrowed schema decode and bounded custom schedules retain concrete types" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    const env = harness.env();
    const F = struct {
        fn decode(value: std.json.Value) ?u32 {
            return if (value == .integer) std.math.cast(u32, value.integer) else null;
        }
        fn invalid(_: std.json.Value) E {
            return .shape;
        }
        fn step(state: *usize, _: E, _: std.Io.Duration, _: f64) fx.Schedule.Decision(u32) {
            if (state.* == 2) return .{ .stop = @intCast(state.*) };
            state.* += 1;
            return .{ .continue_with = .{ .output = @intCast(state.*), .delay = .zero } };
        }
    };
    const schema = comptime fx.Schema(u32, E).fromOptionDecoder(F.decode, F.invalid);
    var json: std.json.Value = .{ .integer = 4 };
    try std.testing.expectEqual(@as(u32, 4), fx.run(&env, schema.decode(Env, &json)).success);
    json = .null;
    try std.testing.expectEqual(E.shape, fx.run(&env, schema.decode(Env, &json)).fail);
    var bounded = fx.Schedule.upTo(.fromSeconds(1), fx.Schedule.spaced(E, .fromSeconds(1)));
    try std.testing.expect(bounded.step(.expected, .zero, 0) == .continue_with);
    try std.testing.expect(bounded.step(.expected, .fromSeconds(1), 0) == .stop);
    var custom = fx.Schedule.unfold(E, u32, @as(usize, 0), false, F.step);
    try std.testing.expectEqual(@as(u32, 1), custom.step(.expected, .zero, 0).continue_with.output);
    try std.testing.expectEqual(@as(u32, 2), custom.step(.expected, .zero, 0).continue_with.output);
    try std.testing.expectEqual(@as(u32, 2), custom.step(.expected, .zero, 0).stop);
}

test "released lease bookkeeping survives pool scope destruction" {
    var harness: Harness = .{};
    harness.init();
    defer harness.deinit();
    const env = harness.env();
    var pool_scope = fx.Scope(E).init(&env);
    var pool_closed = false;
    defer {
        if (!pool_closed) {
            var closed = fx.run(&env, Unit.closeScope(&pool_scope));
            closed.deinit();
        }
        pool_scope.deinit();
    }
    const Pool = fx.EffectPool(u32, E, Env);
    const pool = fx.run(&env, Pool.make(&pool_scope, poolConfig, P.fromOutcome(count), validate, dispose)).success;
    var lease_scope = fx.Scope(E).init(&env);
    defer releaseScope(&lease_scope, &env);
    var acquisition = Pool.acquire(&lease_scope, pool);
    defer acquisition.deinit();
    const lease = fx.run(&env, acquisition).success;
    var released = fx.run(&env, Pool.release(lease));
    defer released.deinit();
    try std.testing.expect(released == .success);
    var closed = fx.run(&env, Unit.closeScope(&pool_scope));
    defer closed.deinit();
    pool_closed = true;
    try std.testing.expect(closed == .success);
    try std.testing.expectEqual(@as(usize, 1), harness.released.load(.acquire));
    // Lease scope still owns bookkeeping, but its resource/pool are now gone.
    var again = fx.run(&env, Pool.release(lease));
    defer again.deinit();
    try std.testing.expect(again == .success);
}
