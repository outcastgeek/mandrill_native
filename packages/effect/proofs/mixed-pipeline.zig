const std = @import("std");
const fx = @import("effect");
const Threads = @import("support/thread_executor.zig").ThreadExecutor;
const MemoryAudit = @import("support/memory_audit.zig").MemoryAudit;
const E = enum { io, malformed, lookup };
const Mode = enum { success, failure, cancellation };
const Env = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    async_engine: fx.Executor,
    parallel_engine: fx.Executor,
    blocking_engine: fx.Executor,
    cancellation: *fx.Cancellation,
    dir: std.Io.Dir,
    mode: Mode,
    counters: *Counters,
};
const Counters = struct {
    acquired: std.atomic.Value(usize) = .init(0),
    released: std.atomic.Value(usize) = .init(0),
    fetched: std.atomic.Value(usize) = .init(0),
    parsed: std.atomic.Value(usize) = .init(0),
    queried: std.atomic.Value(usize) = .init(0),
    saved: std.atomic.Value(usize) = .init(0),
    active: std.atomic.Value(usize) = .init(0),
    peak: std.atomic.Value(usize) = .init(0),
    arrivals: std.atomic.Value(usize) = .init(0),
    ready: std.Io.Event = .unset,
    fn enter(self: *@This()) void {
        const active = self.active.fetchAdd(1, .acq_rel) + 1;
        var peak = self.peak.load(.acquire);
        while (peak < active) {
            if (self.peak.cmpxchgWeak(peak, active, .acq_rel, .acquire)) |changed| peak = changed else break;
        }
    }
};
const Found = struct { session: *Session, value: u32 };
const Query = fx.Effect(Found, E, Env);
const Session = struct {
    allocator: std.mem.Allocator,
    // Fixture values; production storage/policy does not inherit these limits.
    buffer: [32]u8 = undefined,
    len: usize = 0,
    value: u32 = 0,
    total: u32 = 0,
    queries: [3]Query = undefined,
};
const S = fx.Effect(*Session, E, Env);
const P = fx.Effect(u32, E, Env);
const Unit = fx.Effect(void, E, Env);
const F = struct {
    fn acquire(env: *const Env) fx.Exit(*Session, E) {
        const session = env.allocator.create(Session) catch return .{ .die = fx.Defect.out_of_memory };
        session.* = .{ .allocator = env.allocator };
        _ = env.counters.acquired.fetchAdd(1, .monotonic);
        return .{ .success = session };
    }
    fn fetch(session: *const Session, env: *const Env) fx.Exit(*Session, E) {
        const owned = @constCast(session);
        const bytes = env.dir.readFile(env.io, "request", &owned.buffer) catch return .{ .fail = .io };
        owned.len = bytes.len;
        _ = env.counters.fetched.fetchAdd(1, .monotonic);
        return .{ .success = owned };
    }
    fn parse(session: *const Session, env: *const Env, token: *const fx.Cancellation) fx.Exit(*Session, E) {
        if (token.isRequested()) return .interrupted;
        const owned = @constCast(session);
        owned.value = std.fmt.parseInt(u32, owned.buffer[0..owned.len], 10) catch return .{ .fail = .malformed };
        _ = env.counters.parsed.fetchAdd(1, .monotonic);
        return .{ .success = owned };
    }
    fn parsed(session: *Session, _: *const Env) S {
        return S.onCpuBorrowed(Session, session, parse);
    }
    fn meet(env: *const Env, token: ?*const fx.Cancellation) bool {
        // First two admitted branches must both enter before either returns.
        const arrived = env.counters.arrivals.fetchAdd(1, .acq_rel) + 1;
        if (arrived == 2) env.counters.ready.set(env.io);
        env.counters.ready.waitTimeout(env.io, .{ .duration = .{ .raw = .fromSeconds(5), .clock = .awake } }) catch return false;
        if (env.mode == .cancellation) {
            const Wait = struct {
                io: std.Io,
                ready: std.Io.Event = .unset,
                fn wake(data: ?*anyopaque) void {
                    const self: *@This() = @ptrCast(@alignCast(data.?));
                    self.ready.set(self.io);
                }
            };
            var waiter: Wait = .{ .io = env.io };
            var registration: fx.Cancellation.Registration = .{ .data = &waiter, .forward = Wait.wake };
            token.?.register(&registration);
            defer registration.unregister();
            waiter.ready.waitUncancelable(env.io);
        }
        return true;
    }
    fn ioLookup(session: *const Session, env: *const Env, token: ?*const fx.Cancellation) fx.Exit(Found, E) {
        env.counters.enter();
        defer _ = env.counters.active.fetchSub(1, .acq_rel);
        _ = env.counters.queried.fetchAdd(1, .monotonic);
        if (!meet(env, token)) return .{ .die = fx.Defect.init("mixed I/O/CPU progress barrier timed out") };
        if (token.?.isRequested()) return .interrupted;
        var buffer: [32]u8 = undefined;
        const bytes = env.dir.readFile(env.io, "lookup", &buffer) catch return .{ .fail = .io };
        const value = std.fmt.parseInt(u32, bytes, 10) catch return .{ .fail = .malformed };
        return .{ .success = .{ .session = @constCast(session), .value = value + session.value } };
    }
    fn score(session: *const Session, env: *const Env, token: *const fx.Cancellation) fx.Exit(Found, E) {
        env.counters.enter();
        defer _ = env.counters.active.fetchSub(1, .acq_rel);
        _ = env.counters.queried.fetchAdd(1, .monotonic);
        if (!meet(env, token)) return .{ .die = fx.Defect.init("mixed CPU/I/O progress barrier timed out") };
        if (token.isRequested()) return .interrupted;
        if (env.mode == .failure) return .{ .fail = .lookup };
        return .{ .success = .{ .session = @constCast(session), .value = session.value * 2 } };
    }
    fn blocking(session: *const Session, env: *const Env, token: *const fx.Cancellation) fx.Exit(Found, E) {
        env.counters.enter();
        defer _ = env.counters.active.fetchSub(1, .acq_rel);
        _ = env.counters.queried.fetchAdd(1, .monotonic);
        if (token.isRequested()) return .interrupted;
        var file = env.dir.openFile(env.io, "lookup", .{}) catch return .{ .fail = .io };
        defer file.close(env.io);
        var buffer: [32]u8 = undefined;
        const len = file.readPositionalAll(env.io, &buffer, 0) catch return .{ .fail = .io };
        const value = std.fmt.parseInt(u32, buffer[0..len], 10) catch return .{ .fail = .malformed };
        return .{ .success = .{ .session = @constCast(session), .value = value } };
    }
    fn lookup(session: *Session, _: *const Env) fx.Effect(fx.OwnedSlice(Found), E, Env) {
        session.queries = .{
            Query.fromNativeCancellableBorrowed(Session, session, ioLookup),
            Query.onCpuBorrowed(Session, session, score),
            Query.onBlockingBorrowed(Session, session, blocking),
        };
        return Query.allConcurrent(2, &session.queries);
    }
    fn saved(values: fx.OwnedSlice(Found), _: *const Env) P {
        var owned = values;
        defer owned.deinit();
        const session = owned.items[0].session;
        session.total = 0;
        for (owned.items) |item| session.total += item.value;
        return P.fromNativeBorrowed(Session, session, save);
    }
    fn save(session: *const Session, env: *const Env) fx.Exit(u32, E) {
        var buffer: [32]u8 = undefined;
        const bytes = std.fmt.bufPrint(&buffer, "{d}", .{session.total}) catch unreachable;
        env.dir.writeFile(env.io, .{ .sub_path = "saved", .data = bytes }) catch return .{ .fail = .io };
        _ = env.counters.saved.fetchAdd(1, .monotonic);
        return .{ .success = session.total };
    }
    fn body(session: *Session, env: *const Env) P {
        var fetched = S.fromNativeBorrowed(Session, session, fetch);
        var parsed_program = fetched.flatMapOwned(env, parsed);
        var lookups = parsed_program.flatMapOwned(env, lookup);
        return lookups.flatMapOwned(env, saved);
    }
    fn release(session: *Session, _: fx.PrimaryOutcome(E)) Unit {
        return Unit.fromBorrowed(Session, session, struct {
            fn close(value: *const Session, env: *const Env) fx.Exit(void, E) {
                value.allocator.destroy(@constCast(value));
                _ = env.counters.released.fetchAdd(1, .monotonic);
                return .{ .success = {} };
            }
        }.close);
    }
};
const pipeline = S.fromOutcome(F.acquire).acquireRelease(F.body, F.release);

fn runCase(comptime program: P, mode: Mode, malformed: bool, iterations: usize) !void {
    var audit: MemoryAudit = .{};
    var async_engine: Threads = .{ .allocator = audit.allocator(), .capacity = 3, .capabilities = .{ .independent = true, .cpu = false, .blocking = false } };
    var cpu: Threads = .{ .allocator = audit.allocator(), .capacity = 1, .capabilities = .{ .independent = true, .cpu = true, .blocking = false } };
    var blocking: Threads = .{ .allocator = audit.allocator(), .capacity = 1, .capabilities = .{ .independent = true, .cpu = false, .blocking = true } };
    defer async_engine.deinit();
    defer cpu.deinit();
    defer blocking.deinit();
    var temp = std.testing.tmpDir(.{});
    defer temp.cleanup();
    try temp.dir.writeFile(std.testing.io, .{ .sub_path = "lookup", .data = "3" });
    const workload_seed = 0xeffec7;
    var random = std.Random.DefaultPrng.init(workload_seed);
    const started = std.Io.Timestamp.now(std.testing.io, .awake);
    var allocations_per_run: ?usize = null;
    for (0..iterations) |_| {
        const input = random.random().intRangeAtMost(u32, 1, 100);
        const expected = input * 3 + 6;
        var request_buffer: [32]u8 = undefined;
        const request = try std.fmt.bufPrint(&request_buffer, "{d}", .{input});
        try temp.dir.writeFile(std.testing.io, .{ .sub_path = "request", .data = if (malformed) "invalid" else request });
        var cancellation: fx.Cancellation = .{};
        var counters: Counters = .{};
        const env: Env = .{ .allocator = audit.allocator(), .io = std.testing.io, .async_engine = async_engine.executor(), .parallel_engine = cpu.executor(), .blocking_engine = blocking.executor(), .cancellation = &cancellation, .dir = temp.dir, .mode = mode, .counters = &counters };
        const before = audit.allocations;
        {
            var running = fx.runAsync(&env, program);
            defer running.deinit();
            if (mode == .cancellation) {
                try counters.ready.waitTimeout(env.io, .{ .duration = .{ .raw = .fromSeconds(5), .clock = .awake } });
                cancellation.request();
            }
            var result = running.join();
            defer result.deinit();
            if (malformed) try std.testing.expectEqual(E.malformed, result.fail) else switch (mode) {
                .success => {
                    try std.testing.expectEqual(expected, result.success);
                    var output: [32]u8 = undefined;
                    var expected_buffer: [32]u8 = undefined;
                    try std.testing.expectEqualStrings(try std.fmt.bufPrint(&expected_buffer, "{d}", .{expected}), try temp.dir.readFile(env.io, "saved", &output));
                    try std.testing.expectEqual(@as(usize, 3), counters.queried.load(.acquire));
                },
                .failure => try std.testing.expectEqual(E.lookup, result.fail),
                .cancellation => try std.testing.expect(result == .interrupted),
            }
            try std.testing.expectEqual(@as(usize, @intFromBool(mode == .success and !malformed)), counters.saved.load(.acquire));
        }
        try std.testing.expectEqual(@as(usize, 1), counters.released.load(.acquire));
        try std.testing.expectEqual(counters.acquired.load(.acquire), counters.released.load(.acquire));
        try std.testing.expectEqual(@as(usize, 0), counters.active.load(.acquire));
        try std.testing.expect(cancellation.first == null);
        try std.testing.expectEqual(@as(usize, 0), audit.live);
        try std.testing.expectEqual(@as(usize, 0), audit.bytes);
        if (!malformed) try std.testing.expectEqual(@as(usize, 2), counters.peak.load(.acquire));
        try std.testing.expectEqual(@as(usize, 0), async_engine.owned.load(.acquire));
        try std.testing.expectEqual(@as(usize, 0), cpu.owned.load(.acquire));
        try std.testing.expectEqual(@as(usize, 0), blocking.owned.load(.acquire));
        if (iterations > 1) {
            if (allocations_per_run) |prior| try std.testing.expectEqual(prior, audit.allocations - before);
            allocations_per_run = audit.allocations - before;
        }
    }
    const elapsed = started.durationTo(std.Io.Timestamp.now(std.testing.io, .awake)).nanoseconds;
    if (iterations > 1) std.debug.print("mixed workload seed=0x{x} runs={d} elapsed_ns={d} runs_per_second={d:.1} allocations/run={d} peak_live={d} peak_bytes={d} owned_jobs_peak(async/cpu/blocking)={d}/{d}/{d} jobs_started={d} jobs_joined={d}\n", .{
        workload_seed, iterations, elapsed, @as(f64, @floatFromInt(iterations)) * std.time.ns_per_s / @as(f64, @floatFromInt(elapsed)), allocations_per_run.?, audit.peak_live, audit.peak_bytes, async_engine.peak.load(.acquire), cpu.peak.load(.acquire), blocking.peak.load(.acquire), async_engine.started.load(.acquire) + cpu.started.load(.acquire) + blocking.started.load(.acquire), async_engine.joined.load(.acquire) + cpu.joined.load(.acquire) + blocking.joined.load(.acquire),
    });
}

test "dependent I/O fetch CPU parse bounded I/O CPU blocking lookup and I/O save" {
    try runCase(pipeline, .success, false, 1);
    try runCase(pipeline, .failure, false, 1);
    try runCase(pipeline, .cancellation, false, 1);
    try runCase(pipeline, .success, true, 1);
}

test "resource builder supplies the current environment to dependent continuations" {
    const B = fx.EffectBuilder(E, Env);
    const use = struct {
        fn body(resource: fx.ResourceValue(*Session), env: *const Env) P {
            return F.body(resource.value, env);
        }
    }.body;
    const built = comptime B.bind(fx.resource(S.fromOutcome(F.acquire), F.release), use);
    try runCase(built, .success, false, 1);
}

test "sustained mixed workload returns allocations jobs and registrations to baseline" {
    try runCase(pipeline, .success, false, 64);
}
