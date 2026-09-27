const std = @import("std");
const fx = @import("effect");
const Threads = @import("support/thread_executor.zig").ThreadExecutor;
const E = enum { expected, cleanup };
const Env = struct {
    allocator: std.mem.Allocator = std.testing.allocator,
    io: std.Io = std.testing.io,
    async_engine: fx.Executor,
    cancellation: *fx.Cancellation,
    clock: fx.Clock,
    state: *State,
};
const State = struct {
    entered: std.Io.Event = .unset,
    proceed: std.Io.Event = .unset,
    disposed: std.Io.Event = .unset,
    dispose_proceed: std.Io.Event = .unset,
    calls: std.atomic.Value(usize) = .init(0),
    releases: std.atomic.Value(usize) = .init(0),
    hold_load: bool = false,
    hold_dispose: bool = false,
    validation_failure: bool = false,
    cleanup_failure: bool = false,
    time: std.atomic.Value(i64) = .init(0),
    fn now(data: ?*anyopaque) std.Io.Timestamp {
        const self: *@This() = @ptrCast(@alignCast(data.?));
        return .{ .nanoseconds = self.time.load(.acquire) };
    }
    fn sleep(_: ?*anyopaque, _: std.Io.Duration) fx.Clock.SleepResult {
        return .done;
    }
    fn clock(self: *@This()) fx.Clock {
        return .{ .userdata = self, .vtable = &.{ .now = now, .sleep = sleep } };
    }
};
const Harness = struct {
    engine: Threads = .{ .allocator = std.testing.allocator, .capacity = 16, .capabilities = .{ .independent = true, .cpu = false, .blocking = false } },
    cancellation: fx.Cancellation = .{},
    state: State = .{},
    fn env(self: *@This()) Env {
        return .{ .async_engine = self.engine.executor(), .cancellation = &self.cancellation, .clock = self.state.clock(), .state = &self.state };
    }
};
fn wait(event: *std.Io.Event) !void {
    try event.waitTimeout(std.testing.io, .{ .duration = .{ .raw = .fromSeconds(5), .clock = .awake } });
}
fn Task(comptime A: type) type {
    return struct {
        data: *anyopaque,
        callback: *const fn (*anyopaque) fx.Exit(A, E),
        thread: ?std.Thread = null,
        value: ?fx.Exit(A, E) = null,
        completed: std.atomic.Value(bool) = .init(false),
        fn start(self: *@This()) !void {
            self.thread = try std.Thread.spawn(.{ .allocator = std.testing.allocator }, work, .{self});
        }
        fn work(self: *@This()) void {
            self.value = self.callback(self.data);
            self.completed.store(true, .release);
        }
        fn join(self: *@This()) fx.Exit(A, E) {
            if (self.thread) |thread| {
                thread.join();
                self.thread = null;
            }
            return self.value.?;
        }
        fn deinit(self: *@This()) void {
            _ = self.join();
            self.value.?.deinit();
        }
    };
}
const C = fx.Cache(u32, u32, E);
const CacheWork = struct {
    cache: *C,
    token: ?*const fx.Cancellation = null,
    key: u32 = 1,
    fn get(data: *anyopaque) fx.Exit(u32, E) {
        const self: *@This() = @ptrCast(@alignCast(data));
        return self.cache.getToken(self.key, self.token);
    }
};
fn cacheConfig() fx.CacheConfig(u32, E) {
    const Keys = struct {
        fn equal(_: ?*anyopaque, a: u32, b: u32) bool {
            return a == b;
        }
        fn retain(_: ?*anyopaque, key: u32) error{OutOfMemory}!u32 {
            return key;
        }
        fn release(_: ?*anyopaque, _: *u32) void {}
        fn failure(_: E) bool {
            return false;
        }
    };
    return .{ .keys = .{ .userdata = null, .equal = Keys.equal, .retain = Keys.retain, .release = Keys.release }, .time_to_live = .fromSeconds(1), .capacity = 2, .maximum_inflight = 2, .cache_failure = Keys.failure };
}
fn loader(state: *State) C.Loader {
    return .{ .userdata = state, .deinit = struct {
        fn noOp(_: *anyopaque) void {}
    }.noOp, .run = struct {
        fn load(data: *anyopaque, key: u32, _: *const fx.Cancellation) fx.Exit(u32, E) {
            const self: *State = @ptrCast(@alignCast(data));
            _ = self.calls.fetchAdd(1, .acq_rel);
            self.entered.set(std.testing.io);
            if (self.hold_load) self.proceed.waitUncancelable(std.testing.io);
            return .{ .success = key * 2 };
        }
    }.load };
}
fn closeCache(cache: *C) void {
    var closed = cache.shutdown();
    closed.deinit();
    cache.deinit();
}

test "cache shares an inflight load and cancelling one observer preserves the loader and TTL at settlement" {
    var h: Harness = .{};
    h.state.hold_load = true;
    defer h.engine.deinit();
    const env = h.env();
    const cache = C.create(&env, cacheConfig(), loader(&h.state)).success;
    defer closeCache(cache);
    var token: fx.Cancellation = .{};
    var a: CacheWork = .{ .cache = cache, .token = &token };
    var b: CacheWork = .{ .cache = cache };
    var first: Task(u32) = .{ .data = &a, .callback = CacheWork.get };
    try first.start();
    defer first.deinit();
    defer h.state.proceed.set(env.io);
    try wait(&h.state.entered);
    var second: Task(u32) = .{ .data = &b, .callback = CacheWork.get };
    try second.start();
    defer second.deinit();
    defer h.state.proceed.set(env.io);
    // Observe the committed users under the facility lock, not a timing guess.
    const deadline = std.Io.Timestamp.now(env.io, .awake).addDuration(.fromSeconds(5));
    while (true) {
        cache.mutex.lockUncancelable(env.io);
        const both = cache.users == 2;
        cache.mutex.unlock(env.io);
        if (both) break;
        if (std.Io.Timestamp.now(env.io, .awake).nanoseconds >= deadline.nanoseconds) return error.CacheObserverAdmissionTimeout;
        try env.io.sleep(.fromMilliseconds(1), .awake);
    }
    token.request();
    try std.testing.expect(first.join() == .interrupted);
    try std.testing.expectEqual(@as(usize, 1), h.state.calls.load(.acquire));
    // Advance beyond the initial TTL while loading. TTL begins after settle.
    h.state.time.store(2 * std.time.ns_per_s, .release);
    h.state.proceed.set(env.io);
    try std.testing.expectEqual(@as(u32, 2), second.join().success);
    h.state.time.store(2 * std.time.ns_per_s + std.time.ns_per_s / 2, .release);
    try std.testing.expectEqual(@as(u32, 2), cache.getToken(1, null).success);
    try std.testing.expectEqual(@as(usize, 1), h.state.calls.load(.acquire));
    h.state.time.store(3 * std.time.ns_per_s, .release);
    try std.testing.expectEqual(@as(u32, 2), cache.getToken(1, null).success);
    try std.testing.expectEqual(@as(usize, 2), h.state.calls.load(.acquire));
    try std.testing.expect(token.first == null);
}

test "cache capacity evicts the least recently accessed settled entry" {
    var h: Harness = .{};
    defer h.engine.deinit();
    const env = h.env();
    var config = cacheConfig();
    config.time_to_live = null;
    const cache = C.create(&env, config, loader(&h.state)).success;
    defer closeCache(cache);
    for ([_]u32{ 1, 2, 1, 3, 1, 2 }) |key| {
        _ = h.state.time.fetchAdd(1, .acq_rel);
        try std.testing.expectEqual(key * 2, cache.getToken(key, null).success);
    }
    try std.testing.expectEqual(@as(usize, 4), h.state.calls.load(.acquire));
}

test "cache rechecks closure and invalidation after reentrant user callbacks" {
    const Reentry = struct {
        cache: ?*C = null,
        armed: std.atomic.Value(bool) = .init(false),
        invalidate: bool = false,
        fn act(self: *@This()) void {
            if (!self.armed.swap(false, .acq_rel)) return;
            var result = if (self.invalidate) self.cache.?.invalidate(1) else self.cache.?.shutdown();
            defer result.deinit();
            std.debug.assert(result == .success);
        }
        fn equal(data: ?*anyopaque, a: u32, b: u32) bool {
            const self: *@This() = @ptrCast(@alignCast(data.?));
            self.act();
            return a == b;
        }
        fn now(data: ?*anyopaque) std.Io.Timestamp {
            const self: *@This() = @ptrCast(@alignCast(data.?));
            self.act();
            return .zero;
        }
        fn sleep(_: ?*anyopaque, _: std.Io.Duration) fx.Clock.SleepResult {
            return .done;
        }
    };
    for ([_]enum { clock_close, equality_close, equality_invalidate }{ .clock_close, .equality_close, .equality_invalidate }) |action| {
        var h: Harness = .{};
        defer h.engine.deinit();
        var env = h.env();
        var reentry: Reentry = .{ .invalidate = action == .equality_invalidate };
        var config = cacheConfig();
        config.time_to_live = null;
        if (action == .clock_close) env.clock = .{ .userdata = &reentry, .vtable = &.{ .now = Reentry.now, .sleep = Reentry.sleep } } else {
            config.keys.userdata = &reentry;
            config.keys.equal = Reentry.equal;
        }
        const cache = C.create(&env, config, loader(&h.state)).success;
        defer closeCache(cache);
        reentry.cache = cache;
        if (action != .clock_close) try std.testing.expectEqual(@as(u32, 2), cache.getToken(1, null).success);
        reentry.armed.store(true, .release);
        var result = cache.getToken(1, null);
        defer result.deinit();
        if (action == .equality_invalidate) {
            try std.testing.expectEqual(@as(u32, 2), result.success);
            try std.testing.expectEqual(@as(usize, 2), h.state.calls.load(.acquire));
        } else {
            try std.testing.expect(result == .die);
            try std.testing.expectEqual(@as(usize, @intFromBool(action == .equality_close)), h.state.calls.load(.acquire));
        }
    }
}

const Pool = fx.Pool(u32, E);
fn factory(state: *State) Pool.Factory {
    const F = struct {
        fn create(data: *anyopaque, _: *const fx.Cancellation) fx.Exit(u32, E) {
            const self: *State = @ptrCast(@alignCast(data));
            return .{ .success = @intCast(self.calls.fetchAdd(1, .acq_rel)) };
        }
        fn validate(data: *anyopaque, _: u32, _: *const fx.Cancellation) fx.Exit(bool, E) {
            const self: *State = @ptrCast(@alignCast(data));
            return if (self.validation_failure) .{ .fail = .expected } else .{ .success = true };
        }
        fn dispose(data: *anyopaque, _: u32) fx.Exit(void, E) {
            const self: *State = @ptrCast(@alignCast(data));
            self.disposed.set(std.testing.io);
            if (self.hold_dispose) self.dispose_proceed.waitUncancelable(std.testing.io);
            _ = self.releases.fetchAdd(1, .acq_rel);
            return if (self.cleanup_failure) .{ .fail = .cleanup } else .{ .success = {} };
        }
        fn deinit(_: *anyopaque) void {}
    };
    return .{ .userdata = state, .create = F.create, .validate = F.validate, .dispose = F.dispose, .deinit = F.deinit };
}
fn closePool(pool: *Pool) void {
    var result = pool.shutdown();
    result.deinit();
    pool.deinit();
}
fn endLease(lease: *Pool.Lease) void {
    var result = lease.end(false);
    result.deinit();
    lease.deinit();
}
const Borrow = struct {
    pool: *Pool,
    token: ?*const fx.Cancellation = null,
    fn acquire(data: *anyopaque) fx.Exit(*Pool.Lease, E) {
        const self: *@This() = @ptrCast(@alignCast(data));
        return self.pool.acquireToken(self.token);
    }
};
fn awaitWaiters(pool: *Pool, count: usize) !void {
    const deadline = std.Io.Timestamp.now(pool.io, .awake).addDuration(.fromSeconds(5));
    while (true) {
        pool.mutex.lockUncancelable(pool.io);
        var actual: usize = 0;
        var next = pool.first_waiter;
        while (next) |waiter| {
            actual += 1;
            next = waiter.next;
        }
        pool.mutex.unlock(pool.io);
        if (actual == count) return;
        if (std.Io.Timestamp.now(pool.io, .awake).nanoseconds >= deadline.nanoseconds) return error.PoolWaiterAdmissionTimeout;
        try pool.io.sleep(.fromMilliseconds(1), .awake);
    }
}

test "pool FIFO skips a cancelled waiter and a stale disposition cannot retire a new lease" {
    var h: Harness = .{};
    defer h.engine.deinit();
    const env = h.env();
    const pool = Pool.create(&env, .{ .capacity = 1, .idle_time_to_live = null }, factory(&h.state)).success;
    defer closePool(pool);
    const held = pool.acquireToken(null).success;
    defer held.deinit();
    var cancelled: fx.Cancellation = .{};
    var a: Borrow = .{ .pool = pool, .token = &cancelled };
    var b: Borrow = .{ .pool = pool };
    var first: Task(*Pool.Lease) = .{ .data = &a, .callback = Borrow.acquire };
    try first.start();
    defer first.deinit();
    try awaitWaiters(pool, 1);
    var second: Task(*Pool.Lease) = .{ .data = &b, .callback = Borrow.acquire };
    try second.start();
    defer second.deinit();
    try awaitWaiters(pool, 2);
    cancelled.request();
    try std.testing.expect(first.join() == .interrupted);
    var returned = held.end(false);
    defer returned.deinit();
    const next = second.join().success;
    defer endLease(next);
    var stale = held.end(true);
    defer stale.deinit();
    try std.testing.expect(stale == .success);
    try std.testing.expectEqual(@as(u32, 0), next.value());
    try std.testing.expectEqual(@as(usize, 0), h.state.releases.load(.acquire));
    try std.testing.expect(cancelled.first == null);
}

test "pool keeps capacity reserved while disposal is pending" {
    var h: Harness = .{};
    h.state.hold_dispose = true;
    defer h.engine.deinit();
    const env = h.env();
    const pool = Pool.create(&env, .{ .capacity = 1, .idle_time_to_live = null }, factory(&h.state)).success;
    defer closePool(pool);
    const held = pool.acquireToken(null).success;
    defer held.deinit();
    const Dispose = struct {
        fn run(data: *anyopaque) fx.Exit(void, E) {
            const lease: *Pool.Lease = @ptrCast(@alignCast(data));
            return lease.end(true);
        }
    };
    var disposal: Task(void) = .{ .data = held, .callback = Dispose.run };
    try disposal.start();
    defer disposal.deinit();
    defer h.state.dispose_proceed.set(env.io);
    try wait(&h.state.disposed);
    var b: Borrow = .{ .pool = pool };
    var waiting: Task(*Pool.Lease) = .{ .data = &b, .callback = Borrow.acquire };
    try waiting.start();
    defer waiting.deinit();
    defer h.state.dispose_proceed.set(env.io);
    try awaitWaiters(pool, 1);
    try std.testing.expectEqual(@as(usize, 1), h.state.calls.load(.acquire));
    h.state.dispose_proceed.set(env.io);
    try std.testing.expect(disposal.join() == .success);
    const lease = waiting.join().success;
    defer endLease(lease);
    try std.testing.expectEqual(@as(u32, 1), lease.value());
}

test "pool validation failure retains its typed disposal failure" {
    var h: Harness = .{};
    defer h.engine.deinit();
    const env = h.env();
    const pool = Pool.create(&env, .{ .capacity = 1, .idle_time_to_live = null }, factory(&h.state)).success;
    defer closePool(pool);
    const first = pool.acquireToken(null).success;
    var returned = first.end(false);
    returned.deinit();
    first.deinit();
    h.state.validation_failure = true;
    h.state.cleanup_failure = true;
    var result = pool.acquireToken(null);
    defer result.deinit();
    try std.testing.expect(result == .finalization_failed);
    try std.testing.expectEqual(E.expected, result.finalization_failed.primary.failed);
    try std.testing.expectEqual(E.cleanup, result.finalization_failed.first.release_failure);
    try std.testing.expectEqual(@as(usize, 1), h.state.releases.load(.acquire));
}

test "shared pool shutdown waits for outstanding leases and retains one disposal failure" {
    var h: Harness = .{};
    h.state.cleanup_failure = true;
    defer h.engine.deinit();
    const env = h.env();
    const pool = Pool.create(&env, .{ .capacity = 1, .idle_time_to_live = null }, factory(&h.state)).success;
    defer pool.deinit();
    const lease = pool.acquireToken(null).success;
    defer lease.deinit();
    const Close = struct {
        fn run(data: *anyopaque) fx.Exit(void, E) {
            const p: *Pool = @ptrCast(@alignCast(data));
            return p.shutdown();
        }
    };
    var first: Task(void) = .{ .data = pool, .callback = Close.run };
    try first.start();
    defer first.deinit();
    var second: Task(void) = .{ .data = pool, .callback = Close.run };
    try second.start();
    defer second.deinit();
    const deadline = std.Io.Timestamp.now(env.io, .awake).addDuration(.fromSeconds(5));
    while (true) {
        pool.mutex.lockUncancelable(env.io);
        const closed = pool.closed;
        pool.mutex.unlock(env.io);
        if (closed) break;
        if (std.Io.Timestamp.now(env.io, .awake).nanoseconds >= deadline.nanoseconds) return error.PoolShutdownAdmissionTimeout;
        try env.io.sleep(.fromMilliseconds(1), .awake);
    }
    try std.testing.expect(!first.completed.load(.acquire) and !second.completed.load(.acquire));
    var ended = lease.end(false);
    defer ended.deinit();
    for ([_]fx.Exit(void, E){ first.join(), second.join() }) |result| {
        try std.testing.expect(result == .finalization_failed);
        try std.testing.expectEqual(E.cleanup, result.finalization_failed.first.release_failure);
        try std.testing.expect(result.finalization_failed.rest == null);
    }
    try std.testing.expectEqual(@as(usize, 1), h.state.releases.load(.acquire));
}

const P = fx.Effect(u32, E, Env);
const Unit = fx.Effect(void, E, Env);
fn closeScope(scope: *fx.Scope(E), env: *const Env) void {
    var closed = fx.run(env, Unit.closeScope(scope));
    closed.deinit();
    scope.deinit();
}
test "closed scope immediately releases a late resource and retains release failure" {
    var h: Harness = .{};
    defer h.engine.deinit();
    const env = h.env();
    var scope = fx.Scope(E).init(&env);
    defer scope.deinit();
    var closed = fx.run(&env, Unit.closeScope(&scope));
    closed.deinit();
    const F = struct {
        fn release(_: u32) Unit {
            return Unit.fromOutcome(struct {
                fn run(e: *const Env) fx.Exit(void, E) {
                    _ = e.state.releases.fetchAdd(1, .acq_rel);
                    return .{ .fail = .cleanup };
                }
            }.run);
        }
    };
    var result = fx.run(&env, (comptime P.succeed(1)).scopeAcquire(&scope, F.release));
    defer result.deinit();
    try std.testing.expect(result == .finalization_failed);
    try std.testing.expectEqual(E.cleanup, result.finalization_failed.first.release_failure);
    try std.testing.expectEqual(@as(usize, 1), h.state.releases.load(.acquire));
}

test "scope close retains a completed unobserved protected child failure" {
    var h: Harness = .{};
    defer h.engine.deinit();
    const env = h.env();
    var scope = fx.Scope(E).init(&env);
    defer scope.deinit();
    const child = comptime P.fromOutcome(struct {
        fn body(e: *const Env) fx.Exit(u32, E) {
            e.state.entered.set(e.io);
            return .{ .fail = .expected };
        }
    }.body).uninterruptible();
    var started = fx.run(&env, child.fork(&scope));
    defer started.deinit();
    try std.testing.expect(started == .success);
    try wait(&h.state.entered);
    var result = fx.run(&env, Unit.closeScope(&scope));
    defer result.deinit();
    try std.testing.expect(result == .finalization_failed);
    try std.testing.expectEqual(E.expected, result.finalization_failed.first.release_failure);
}

const Iterator = struct { yielded: bool = false };
const StreamFixture = struct {
    fn create(env: *const Env) fx.Exit(*Iterator, E) {
        const iterator = env.allocator.create(Iterator) catch return .{ .die = fx.Defect.out_of_memory };
        iterator.* = .{};
        _ = env.state.calls.fetchAdd(1, .acq_rel);
        return .{ .success = iterator };
    }
    fn next(iterator: *const Iterator, env: *const Env) fx.Exit(fx.Chunk(u32), E) {
        if (iterator.yielded) return .{ .success = .end };
        @constCast(iterator).yielded = true;
        const values = env.allocator.dupe(u32, &.{ 1, 2, 3 }) catch return .{ .die = fx.Defect.out_of_memory };
        return .{ .success = fx.Chunk(u32).owned(.{ .items = values, .allocator = env.allocator }) };
    }
    fn pull(iterator: *Iterator) fx.Effect(fx.Chunk(u32), E, Env) {
        return fx.Effect(fx.Chunk(u32), E, Env).fromBorrowed(Iterator, iterator, next);
    }
    fn release(iterator: *Iterator) Unit {
        return Unit.fromNativeBorrowed(Iterator, iterator, close);
    }
    fn close(iterator: *const Iterator, env: *const Env) fx.Exit(void, E) {
        env.state.disposed.set(env.io);
        if (env.state.hold_dispose) env.state.dispose_proceed.waitUncancelable(env.io);
        env.allocator.destroy(@constCast(iterator));
        _ = env.state.releases.fetchAdd(1, .acq_rel);
        return .{ .fail = .cleanup };
    }
    fn map(value: u32) P {
        return if (value == 2) P.fail(.expected) else P.succeed(value);
    }
    fn readPending(_: *const Iterator, env: *const Env, token: ?*const fx.Cancellation) fx.Exit(?u32, E) {
        const Wake = struct {
            io: std.Io,
            event: std.Io.Event = .unset,
            fn wake(data: ?*anyopaque) void {
                const self: *@This() = @ptrCast(@alignCast(data.?));
                self.event.set(self.io);
            }
        };
        var wake: Wake = .{ .io = env.io };
        var registration: fx.Cancellation.Registration = .{ .data = &wake, .forward = Wake.wake };
        token.?.register(&registration);
        defer registration.unregister();
        env.state.entered.set(env.io);
        wake.event.waitUncancelable(env.io);
        return .interrupted;
    }
    fn move(iterator: *const Iterator, env: *const Env) fx.Async(?u32, E).Start {
        return fx.Async(?u32, E).nativeCancellable(env, iterator, readPending);
    }
};

test "owning stream chunks retain mapper failure and disposal failure on independent reruns" {
    var h: Harness = .{};
    defer h.engine.deinit();
    const env = h.env();
    const stream = comptime fx.Stream(u32, E, Env).fromPull(fx.Effect(*Iterator, E, Env).fromOutcome(StreamFixture.create), StreamFixture.pull, StreamFixture.release).mapEffect(StreamFixture.map);
    for (0..2) |_| {
        var result = fx.run(&env, stream.runCollect(3));
        defer result.deinit();
        try std.testing.expect(result == .finalization_failed);
        try std.testing.expectEqual(E.expected, result.finalization_failed.primary.failed);
        try std.testing.expectEqual(E.cleanup, result.finalization_failed.first.release_failure);
    }
    try std.testing.expectEqual(@as(usize, 2), h.state.calls.load(.acquire));
    try std.testing.expectEqual(@as(usize, 2), h.state.releases.load(.acquire));
}

test "async stream cancellation joins the pending move and protected disposal before publishing" {
    var h: Harness = .{};
    h.state.hold_dispose = true;
    defer h.engine.deinit();
    const env = h.env();
    const stream = comptime fx.Stream(u32, E, Env).fromAsyncIterable(Iterator, StreamFixture.create, StreamFixture.move, StreamFixture.release);
    var running = fx.runAsync(&env, stream.runCollect(1));
    defer running.deinit();
    defer h.state.dispose_proceed.set(env.io);
    try wait(&h.state.entered);
    const Run = @TypeOf(running);
    const Join = struct {
        fn call(data: *anyopaque) fx.Exit(fx.OwnedSlice(u32), E) {
            const handle: *Run = @ptrCast(@alignCast(data));
            return handle.join();
        }
    };
    var observing: Task(fx.OwnedSlice(u32)) = .{ .data = &running, .callback = Join.call };
    try observing.start();
    defer observing.deinit();
    defer h.state.dispose_proceed.set(env.io);
    running.request();
    try wait(&h.state.disposed);
    try std.testing.expect(!observing.completed.load(.acquire));
    try std.testing.expectEqual(@as(usize, 0), h.state.releases.load(.acquire));
    h.state.dispose_proceed.set(env.io);
    const result = observing.join();
    try std.testing.expect(result == .finalization_failed);
    try std.testing.expect(result.finalization_failed.primary == .was_interrupted);
    try std.testing.expectEqual(E.cleanup, result.finalization_failed.first.release_failure);
    try std.testing.expectEqual(@as(usize, 1), h.state.releases.load(.acquire));
    try std.testing.expect(h.cancellation.first == null);
}

test "10000 nested trace spans preserve lineage and unwind through the shared iterative kernel" {
    const Audit = @import("support/memory_audit.zig").MemoryAudit;
    const Recorder = struct {
        started: usize = 0,
        ended: usize = 0,
        valid: bool = true,
        fn begin(data: ?*anyopaque, span: fx.SpanStart) fx.Exit(fx.SpanId, fx.Defect) {
            const self: *@This() = @ptrCast(@alignCast(data.?));
            self.valid = self.valid and if (self.started == 0) span.parent == null else span.parent != null and span.parent.?.value == self.started;
            self.started += 1;
            return .{ .success = .{ .value = self.started } };
        }
        fn end(data: ?*anyopaque, span: fx.SpanFinish) fx.Exit(void, fx.Defect) {
            const self: *@This() = @ptrCast(@alignCast(data.?));
            self.valid = self.valid and span.id.value == self.started - self.ended and span.outcome == .succeeded;
            self.ended += 1;
            return .{ .success = {} };
        }
        fn abandon(_: ?*anyopaque, _: fx.SpanId) fx.Exit(void, fx.Defect) {
            return .{ .success = {} };
        }
        fn diagnostic(_: ?*anyopaque, _: fx.Defect) void {
            @panic("unexpected trace diagnostic");
        }
    };
    const TraceEnv = struct { allocator: std.mem.Allocator, io: std.Io, remaining: *usize, tracer: fx.Tracer };
    const Traced = fx.Effect(u32, E, TraceEnv);
    const recurse = struct {
        fn recurse(env: *const TraceEnv) Traced {
            if (env.remaining.* == 0) return Traced.succeed(42);
            env.remaining.* -= 1;
            return comptime Traced.deferEffect(@This().recurse).withSpan("nested");
        }
    }.recurse;
    var audit: Audit = .{};
    var recorder: Recorder = .{};
    const depth = 10_000;
    var remaining: usize = depth;
    const env: TraceEnv = .{ .allocator = audit.allocator(), .io = std.testing.io, .remaining = &remaining, .tracer = .{ .userdata = &recorder, .vtable = &.{ .begin = Recorder.begin, .end = Recorder.end, .abandon = Recorder.abandon }, .redactor = .identity, .diagnostics = .{ .userdata = null, .report_fn = Recorder.diagnostic } } };
    var result = fx.run(&env, comptime Traced.deferEffect(recurse));
    defer result.deinit();
    try std.testing.expectEqual(@as(u32, 42), result.success);
    try std.testing.expect(recorder.valid);
    try std.testing.expectEqual(@as(usize, depth), recorder.started);
    try std.testing.expectEqual(recorder.started, recorder.ended);
    try std.testing.expectEqual(@as(usize, 0), audit.live);
    try std.testing.expectEqual(@as(usize, 0), audit.bytes);
}
