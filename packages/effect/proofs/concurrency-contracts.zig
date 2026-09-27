const std = @import("std");
const fx = @import("effect");
const Threads = @import("support/thread_executor.zig").ThreadExecutor;
const E = enum { expected, cleanup };
const Barrier = struct {
    io: std.Io,
    required: usize,
    mutex: std.Io.Mutex = .init,
    arrived: usize = 0,
    ready: std.Io.Event = .unset,
    fn meet(self: *@This()) bool {
        self.mutex.lockUncancelable(self.io);
        self.arrived += 1;
        if (self.arrived == self.required) self.ready.set(self.io);
        self.mutex.unlock(self.io);
        self.ready.waitTimeout(self.io, .{ .duration = .{ .raw = .fromSeconds(5), .clock = .awake } }) catch return false;
        return true;
    }
};
const Env = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    async_engine: fx.Executor,
    parallel_engine: fx.Executor,
    blocking_engine: fx.Executor,
    cancellation: *fx.Cancellation,
    barrier: *Barrier,
    calls: *std.atomic.Value(usize),
};
const Id = std.Thread.Id;
const P = fx.Effect(Id, E, Env);
const Unit = fx.Effect(void, E, Env);
const Harness = struct {
    engine: Threads = .{ .allocator = std.testing.allocator, .capacity = 8, .capabilities = .{ .independent = true, .cpu = true, .blocking = true } },
    cancellation: fx.Cancellation = .{},
    calls: std.atomic.Value(usize) = .init(0),
    barrier: Barrier = .{ .io = std.testing.io, .required = 4 },
    fn env(self: *@This()) Env {
        return .{ .allocator = std.testing.allocator, .io = std.testing.io, .async_engine = self.engine.executor(), .parallel_engine = self.engine.executor(), .blocking_engine = self.engine.executor(), .cancellation = &self.cancellation, .barrier = &self.barrier, .calls = &self.calls };
    }
};
fn contestant(env: *const Env) fx.Exit(Id, E) {
    return if (env.barrier.meet()) .{ .success = std.Thread.getCurrentId() } else .{ .die = fx.Defect.init("synchronous barrier timed out") };
}

test "four synchronous bodies execute on four distinct consumer workers" {
    var harness: Harness = .{};
    defer harness.engine.deinit();
    const env = harness.env();
    const leaf = comptime P.fromOutcome(contestant);
    const jobs = [_]P{ leaf, leaf, leaf, leaf };
    var handle = fx.runAsync(&env, P.allConcurrent(jobs.len, &jobs));
    defer handle.deinit();
    var result = handle.join();
    defer result.deinit();
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(jobs.len, harness.barrier.arrived);
    for (result.success.items, 0..) |id, index| {
        for (result.success.items[0..index]) |other| try std.testing.expect(id != other);
    }
    try std.testing.expectEqual(@as(usize, 0), harness.engine.owned.load(.acquire));
}

test "two synchronous race competitors both cross the independent worker barrier" {
    var harness: Harness = .{};
    harness.barrier.required = 2;
    defer harness.engine.deinit();
    const env = harness.env();
    const race = comptime P.fromOutcome(contestant).race(P.fromOutcome(contestant));
    var handle = fx.runAsync(&env, race);
    defer handle.deinit();
    var result = handle.join();
    defer result.deinit();
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(usize, 2), harness.barrier.arrived);
    try std.testing.expectEqual(@as(usize, 0), harness.engine.owned.load(.acquire));
}

test "nested saturated CPU admission rejects without starting a child or deadlocking" {
    var harness: Harness = .{};
    defer harness.engine.deinit();
    var cpu: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = true, .cpu = true, .blocking = false } };
    defer cpu.deinit();
    var env = harness.env();
    env.parallel_engine = cpu.executor();
    const F = struct {
        fn work(environment: *const Env, _: *const fx.Cancellation) fx.Exit(Id, E) {
            _ = environment.calls.fetchAdd(1, .monotonic);
            return .{ .success = std.Thread.getCurrentId() };
        }
    };
    const nested = comptime P.onCpu(F.work).dispatch(.cpu);
    var handle = fx.runAsync(&env, nested);
    defer handle.deinit();
    var result = handle.join();
    defer result.deinit();
    try std.testing.expect(result == .die);
    try std.testing.expectEqual(fx.Defect.Kind.provider_unavailable, result.die.kind);
    try std.testing.expectEqual(@as(usize, 0), harness.calls.load(.acquire));
    try std.testing.expectEqual(@as(usize, 1), cpu.started.load(.acquire));
    try std.testing.expectEqual(@as(usize, 0), cpu.owned.load(.acquire));
}

fn failAfterBarrier(environment: *const Env) fx.Exit(Id, E) {
    if (!environment.barrier.meet()) return .{ .die = fx.Defect.init("failing racer did not reach barrier") };
    return .{ .fail = .expected };
}
fn waitForCancellation(_: *const void, environment: *const Env, token: ?*const fx.Cancellation) fx.Exit(Id, E) {
    if (!environment.barrier.meet()) return .{ .die = fx.Defect.init("losing racer did not reach barrier") };
    const Wait = struct {
        io: std.Io,
        event: std.Io.Event = .unset,
        fn wake(data: ?*anyopaque) void {
            const wait: *@This() = @ptrCast(@alignCast(data.?));
            wait.event.set(wait.io);
        }
    };
    var wait: Wait = .{ .io = environment.io };
    var registration: fx.Cancellation.Registration = .{ .data = &wait, .forward = Wait.wake };
    token.?.register(&registration);
    defer registration.unregister();
    wait.event.waitUncancelable(environment.io);
    _ = environment.calls.fetchAdd(1, .monotonic);
    return .interrupted;
}
fn losingBody(_: void) P {
    return P.fromCancellableBorrowed(void, &{}, waitForCancellation);
}
fn losingRelease(_: void, _: fx.PrimaryOutcome(E)) Unit {
    return Unit.fail(.cleanup);
}

test "race failure wins and publishes only after retaining loser cleanup" {
    var harness: Harness = .{};
    harness.barrier.required = 2;
    defer harness.engine.deinit();
    const env = harness.env();
    const loser = comptime Unit.succeed({}).acquireRelease(losingBody, losingRelease);
    const race = comptime P.fromOutcome(failAfterBarrier).race(loser);
    var handle = fx.runAsync(&env, race);
    defer handle.deinit();
    var result = handle.join();
    defer result.deinit();
    try std.testing.expect(result == .finalization_failed);
    try std.testing.expectEqual(E.expected, result.finalization_failed.primary.failed);
    try std.testing.expectEqual(E.cleanup, result.finalization_failed.first.release_failure);
    try std.testing.expect(result.finalization_failed.rest == null);
    try std.testing.expectEqual(@as(usize, 1), harness.calls.load(.acquire));
    try std.testing.expectEqual(@as(usize, 0), harness.engine.owned.load(.acquire));
}

test "race retains cancellation forwarding and every join failure" {
    var harness: Harness = .{};
    harness.barrier.required = 2;
    harness.engine.request_failure = fx.Defect.init("request fault");
    harness.engine.join_failure = fx.Defect.init("join fault");
    defer harness.engine.deinit();
    const env = harness.env();
    const loser = comptime Unit.succeed({}).acquireRelease(losingBody, losingRelease);
    const race = comptime P.fromOutcome(failAfterBarrier).race(loser);
    var handle = fx.runAsync(&env, race);
    defer handle.deinit();
    var result = handle.join();
    defer result.deinit();
    try std.testing.expect(result == .finalization_failed);
    try std.testing.expectEqual(E.expected, result.finalization_failed.primary.failed);
    var request_faults: usize = 0;
    var join_faults: usize = 0;
    var releases: usize = 0;
    var iterator = result.finalization_failed.iterator();
    while (iterator.nextFailure()) |failure| switch (failure) {
        .release_failure => |err| {
            try std.testing.expectEqual(E.cleanup, err);
            releases += 1;
        },
        .release_defect => |defect| {
            if (std.mem.eql(u8, defect.message, "request fault")) request_faults += 1 else if (std.mem.eql(u8, defect.message, "join fault")) join_faults += 1 else return error.UnexpectedProviderFault;
        },
        .release_interrupted => return error.UnexpectedCleanupInterruption,
    };
    try std.testing.expectEqual(@as(usize, 1), request_faults);
    try std.testing.expectEqual(@as(usize, 3), join_faults);
    try std.testing.expectEqual(@as(usize, 1), releases);
    try std.testing.expectEqual(@as(usize, 0), harness.engine.owned.load(.acquire));
}

test "bounded traversal claims the next item when either worker completes and preserves fifty input positions" {
    var harness: Harness = .{};
    defer harness.engine.deinit();
    const env = harness.env();
    const Traversal = struct {
        callbacks: std.atomic.Value(usize) = .init(0),
        active: std.atomic.Value(usize) = .init(0),
        peak: std.atomic.Value(usize) = .init(0),
        third: std.Io.Event = .unset,
    };
    const Work = struct { index: usize, state: *Traversal };
    const Number = fx.Effect(usize, E, Env);
    const F = struct {
        fn transform(work: *const Work) Number {
            _ = work.state.callbacks.fetchAdd(1, .acq_rel);
            return Number.onCpuBorrowed(Work, work, execute);
        }
        fn execute(work: *const Work, environment: *const Env, _: *const fx.Cancellation) fx.Exit(usize, E) {
            const state = work.state;
            const active = state.active.fetchAdd(1, .acq_rel) + 1;
            defer _ = state.active.fetchSub(1, .acq_rel);
            var peak = state.peak.load(.acquire);
            while (peak < active) {
                if (state.peak.cmpxchgWeak(peak, active, .acq_rel, .acquire)) |changed| peak = changed else break;
            }
            if (work.index == 0) state.third.waitTimeout(environment.io, .{ .duration = .{ .raw = .fromSeconds(5), .clock = .awake } }) catch return .{ .die = fx.Defect.init("traversal waited for a whole batch before advancing") };
            if (work.index == 2) state.third.set(environment.io);
            return .{ .success = work.index };
        }
    };
    var state: Traversal = .{};
    var work: [50]Work = undefined;
    var pointers: [work.len]*const Work = undefined;
    for (&work, &pointers, 0..) |*item, *pointer, index| {
        item.* = .{ .index = index, .state = &state };
        pointer.* = item;
    }
    const program = Number.forEachConcurrent(*const Work, 2, &pointers, F.transform);
    try std.testing.expectEqual(@as(usize, 0), state.callbacks.load(.acquire));
    var running = fx.runAsync(&env, program);
    defer running.deinit();
    var result = running.join();
    defer result.deinit();
    try std.testing.expect(result == .success);
    for (result.success.items, 0..) |value, expected| try std.testing.expectEqual(expected, value);
    try std.testing.expectEqual(work.len, state.callbacks.load(.acquire));
    try std.testing.expectEqual(@as(usize, 2), state.peak.load(.acquire));
    try std.testing.expectEqual(@as(usize, 0), state.active.load(.acquire));
    try std.testing.expectEqual(@as(usize, 0), harness.engine.owned.load(.acquire));
}
