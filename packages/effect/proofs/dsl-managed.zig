const std = @import("std");
const fx = @import("effect");
const gates = @import("support/dsl_gate.zig");
const E = enum { expected, capacity };
const Env = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, async_engine: fx.Executor, clock: fx.Clock = fx.Clock.fromIo(&std.testing.io), gate: ?*gates.Gate(u32, E) = null };
const P = fx.Effect(u32, E, Env);
const Unit = fx.Effect(void, E, Env);
const Counts = struct {
    live: std.atomic.Value(usize) = .init(1),
    configured: std.atomic.Value(usize) = .init(0),
    loaded: std.atomic.Value(usize) = .init(0),
    validated: std.atomic.Value(usize) = .init(0),
    disposed: std.atomic.Value(usize) = .init(0),
    values: std.atomic.Value(usize) = .init(0),
    matched: std.Io.Event = .unset,
};
const Owner = struct {
    pub const effect_owned_payload = true;
    counts: *Counts,
    pending: ?*gates.Gate(u32, E) = null,
    pub fn deinit(self: *@This()) void {
        if (self.pending) |gate| std.debug.assert(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
        std.debug.assert(self.counts.live.fetchSub(1, .acq_rel) == 1);
    }
};
const Inputs = struct { owner: Owner, multiplier: u32, invalid_policy: bool = false, ttl: ?std.Io.Duration = null, fail_after_first_create: bool = false, fail_validation: bool = false, fail_disposal: bool = false, inflight: usize = 2, dependency_limit: usize = 1, dependency_count: usize = 1 };
fn policy() fx.CacheKeyPolicy(u32) {
    return .{ .userdata = null, .equal = struct {
        fn equal(_: ?*anyopaque, a: u32, b: u32) bool {
            return a == b;
        }
    }.equal, .retain = struct {
        fn retain(_: ?*anyopaque, value: u32) error{OutOfMemory}!u32 {
            return value;
        }
    }.retain, .release = struct {
        fn release(_: ?*anyopaque, _: *u32) void {}
    }.release };
}
fn dependencyPolicy() fx.DependencyPolicy(u32) {
    const selected = policy();
    return .{ .userdata = selected.userdata, .equal = selected.equal, .retain = selected.retain, .release = selected.release };
}
const CacheRoles = struct {
    fn configure(inputs: *const Inputs, _: *const Env) fx.CacheConfig(u32, E) {
        _ = inputs.owner.counts.configured.fetchAdd(1, .monotonic);
        return .{ .keys = policy(), .time_to_live = if (inputs.invalid_policy) .{ .nanoseconds = -1 } else inputs.ttl, .capacity = 2, .maximum_inflight = inputs.inflight, .cache_failure = struct {
            fn cache(_: E) bool {
                return false;
            }
        }.cache };
    }
    fn load(inputs: *const Inputs, key: u32, _: *const Env) P {
        if (inputs.owner.counts.live.load(.acquire) != 1) return P.die(fx.Defect.init("cache capture ended before loader"));
        _ = inputs.owner.counts.loaded.fetchAdd(1, .monotonic);
        return P.succeed(key * inputs.multiplier);
    }
    fn trackedPolicy(inputs: *const Inputs, current: *const Env) fx.CacheConfig(u32, E) {
        var selected = configure(inputs, current);
        selected.keys.userdata = inputs.owner.counts;
        selected.keys.equal = struct {
            fn equal(data: ?*anyopaque, a: u32, b: u32) bool {
                const counts: *Counts = @ptrCast(@alignCast(data.?));
                if (a == b) counts.matched.set(std.testing.io);
                return a == b;
            }
        }.equal;
        return selected;
    }
    fn pending(inputs: *const Inputs, _: u32, _: *const Env) P {
        if (inputs.owner.counts.live.load(.acquire) != 1) return P.die(fx.Defect.init("cache capture ended before suspended load"));
        _ = inputs.owner.counts.loaded.fetchAdd(1, .monotonic);
        return P.fromAsync(struct {
            fn start(current: *const Env) fx.Async(u32, E).Start {
                return current.gate.?.start();
            }
        }.start);
    }
    fn boundedPolicy(inputs: *const Inputs, current: *const Env) fx.CacheConfig(u32, E) {
        var selected = configure(inputs, current);
        selected.capacity_error = struct {
            fn capacity() E {
                return .capacity;
            }
        }.capacity;
        return selected;
    }
};
const DependencyRoles = struct {
    fn configure(inputs: *const Inputs, env: *const Env) fx.DependencyCacheConfig(u32, u32, E) {
        return .{ .retention = CacheRoles.configure(inputs, env), .maximum_inflight = inputs.inflight, .maximum_dependencies_per_entry = inputs.dependency_limit, .dependency_policy = dependencyPolicy(), .capacity_error = struct {
            fn capacity() E {
                return .capacity;
            }
        }.capacity };
    }
    const Loaded = fx.DependencyCacheLoaded(u32, u32);
    fn load(inputs: *const Inputs, key: u32, env: *const Env) fx.Effect(Loaded, E, Env) {
        const Capture = struct { inputs: *const Inputs, key: u32 };
        return fx.Effect(Loaded, E, Env).fromOwned(env, Capture{ .inputs = inputs, .key = key }, struct {
            fn run(value: *const Capture, current: *const Env) fx.Exit(Loaded, E) {
                if (value.inputs.owner.counts.live.load(.acquire) != 1) return .{ .die = fx.Defect.init("dependency capture ended before loader") };
                _ = value.inputs.owner.counts.loaded.fetchAdd(1, .monotonic);
                const ids = [_]u32{ value.key % 2, value.key % 2 + 2 };
                return .{ .success = Loaded.init(current, value.key * value.inputs.multiplier, ids[0..value.inputs.dependency_count], dependencyPolicy()) catch return .{ .die = fx.Defect.out_of_memory } };
            }
        }.run);
    }
};
fn close(scope: *fx.Scope(E), env: *const Env) !void {
    var result = fx.run(env, Unit.closeScope(scope));
    defer result.deinit();
    scope.deinit();
    try std.testing.expect(result == .success);
}

test "FAC-03 CAP-04 inferred cache roles retain generated captures after description teardown and preserve invalidation" {
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = false, .blocking = false }, .maximum_owned = 8 };
    defer engine.deinit();
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor() };
    var counts: Counts = .{};
    var scope = fx.Scope(E).init(&env);
    var described = fx.dsl.makeCache(&env, &scope, Inputs{ .owner = .{ .counts = &counts }, .multiplier = 3 }, .{ .load = CacheRoles.load, .configure = CacheRoles.configure });
    comptime if (@TypeOf(described) != fx.Effect(*fx.Cache(u32, u32, E), E, Env)) @compileError("cache facade must return exact Effect");
    try std.testing.expectEqual(@as(usize, 0), counts.configured.load(.acquire));
    var made = fx.run(&env, described);
    described.deinit();
    defer made.deinit();
    try std.testing.expect(made == .success);
    const Operations = fx.EffectCache(u32, u32, E, Env);
    var get = Operations.get(made.success, 7);
    defer get.deinit();
    for (0..2) |_| {
        var result = fx.run(&env, get);
        defer result.deinit();
        try std.testing.expect(result == .success and result.success == 21);
    }
    try std.testing.expectEqual(@as(usize, 1), counts.loaded.load(.acquire));
    var invalidate = Operations.invalidate(made.success, 7);
    defer invalidate.deinit();
    var invalidated = fx.run(&env, invalidate);
    defer invalidated.deinit();
    try std.testing.expect(invalidated == .success);
    var loaded = fx.run(&env, get);
    defer loaded.deinit();
    try std.testing.expect(loaded == .success and loaded.success == 21);
    try std.testing.expectEqual(@as(usize, 2), counts.loaded.load(.acquire));
    try std.testing.expectEqual(@as(usize, 1), counts.configured.load(.acquire));
    try std.testing.expectEqual(@as(usize, 1), counts.live.load(.acquire));
    try close(&scope, &env);
    try std.testing.expectEqual(@as(usize, 0), counts.live.load(.acquire));
}

test "FAC-03 FAC-07 dependency cache selects policy once across repeated loads and dependency invalidation" {
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = false, .blocking = false }, .maximum_owned = 8 };
    defer engine.deinit();
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor() };
    var counts: Counts = .{};
    var scope = fx.Scope(E).init(&env);
    var described = fx.dsl.makeDependencyCache(&env, &scope, Inputs{ .owner = .{ .counts = &counts }, .multiplier = 3 }, .{ .load = DependencyRoles.load, .configure = DependencyRoles.configure });
    const Operations = fx.DependencyCache(u32, u32, u32, E, Env);
    comptime if (@TypeOf(described) != fx.Effect(*Operations.Core, E, Env)) @compileError("dependency cache facade must return exact Effect");
    try std.testing.expectEqual(@as(usize, 0), counts.configured.load(.acquire));
    var made = fx.run(&env, described);
    described.deinit();
    defer made.deinit();
    try std.testing.expect(made == .success);
    var get = Operations.get(made.success, 7);
    defer get.deinit();
    for (0..2) |_| {
        var result = fx.run(&env, get);
        defer result.deinit();
        try std.testing.expect(result == .success and result.success == 21);
    }
    try std.testing.expectEqual(@as(usize, 1), counts.loaded.load(.acquire));
    var invalidate = Operations.invalidateDependency(made.success, 1);
    defer invalidate.deinit();
    var invalidated = fx.run(&env, invalidate);
    defer invalidated.deinit();
    try std.testing.expect(invalidated == .success);
    var loaded = fx.run(&env, get);
    defer loaded.deinit();
    try std.testing.expect(loaded == .success and loaded.success == 21);
    try std.testing.expectEqual(@as(usize, 2), counts.loaded.load(.acquire));
    try std.testing.expectEqual(@as(usize, 1), counts.configured.load(.acquire));
    try close(&scope, &env);
    try std.testing.expectEqual(@as(usize, 0), counts.live.load(.acquire));
}

const PoolRoles = struct {
    fn configure(inputs: *const Inputs, _: *const Env) fx.PoolConfig {
        _ = inputs.owner.counts.configured.fetchAdd(1, .monotonic);
        return .{ .capacity = if (inputs.invalid_policy) 0 else 2, .idle_time_to_live = inputs.ttl };
    }
    fn create(inputs: *const Inputs, _: *const Env) P {
        if (inputs.owner.counts.live.load(.acquire) != 1) return P.die(fx.Defect.init("pool capture ended before creation"));
        const ordinal = inputs.owner.counts.loaded.fetchAdd(1, .monotonic);
        if (inputs.fail_after_first_create and ordinal != 0) return P.fail(.expected);
        return P.succeed(@as(u32, @intCast(ordinal)) + inputs.multiplier);
    }
    fn validate(inputs: *const Inputs, _: u32, _: *const Env) fx.Effect(bool, E, Env) {
        if (inputs.owner.counts.live.load(.acquire) != 1) return fx.Effect(bool, E, Env).die(fx.Defect.init("pool capture ended before validation"));
        _ = inputs.owner.counts.validated.fetchAdd(1, .monotonic);
        return if (inputs.fail_validation) fx.Effect(bool, E, Env).fail(.expected) else fx.Effect(bool, E, Env).succeed(true);
    }
    fn dispose(inputs: *const Inputs, _: u32, _: *const Env) Unit {
        return Unit.fromBorrowed(Inputs, inputs, struct {
            fn run(value: *const Inputs, _: *const Env) fx.Exit(void, E) {
                if (value.owner.counts.live.load(.acquire) != 1) return .{ .die = fx.Defect.init("pool capture ended before disposal") };
                _ = value.owner.counts.disposed.fetchAdd(1, .monotonic);
                if (value.fail_disposal) return .{ .fail = .capacity };
                return .{ .success = {} };
            }
        }.run);
    }
};

test "FAC-04 CAP-04 inferred pool factories retain captures through validation leases discard and shutdown" {
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = false, .blocking = false }, .maximum_owned = 8 };
    defer engine.deinit();
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor() };
    var counts: Counts = .{};
    var scope = fx.Scope(E).init(&env);
    var described = fx.dsl.makePool(&env, &scope, Inputs{ .owner = .{ .counts = &counts }, .multiplier = 3 }, .{ .create = PoolRoles.create, .configure = PoolRoles.configure, .dispose = PoolRoles.dispose, .validate = PoolRoles.validate });
    comptime if (@TypeOf(described) != fx.Effect(*fx.Pool(u32, E), E, Env)) @compileError("pool facade must return exact Effect");
    try std.testing.expectEqual(@as(usize, 0), counts.configured.load(.acquire));
    var made = fx.run(&env, described);
    described.deinit();
    defer made.deinit();
    try std.testing.expect(made == .success);
    const Operations = fx.EffectPool(u32, E, Env);
    var population = Operations.ensureSize(made.success, 2);
    defer population.deinit();
    var populated = fx.run(&env, population);
    defer populated.deinit();
    try std.testing.expect(populated == .success);
    var lease_scope = fx.Scope(E).init(&env);
    var first_description = Operations.acquire(&lease_scope, made.success);
    defer first_description.deinit();
    var first = fx.run(&env, first_description);
    defer first.deinit();
    try std.testing.expect(first == .success);
    var second_description = Operations.acquire(&lease_scope, made.success);
    defer second_description.deinit();
    var second = fx.run(&env, second_description);
    defer second.deinit();
    try std.testing.expect(second == .success);
    try std.testing.expect(Operations.value(first.success) != Operations.value(second.success));
    var returned = fx.run(&env, Operations.release(first.success));
    defer returned.deinit();
    var discarded = fx.run(&env, Operations.discard(second.success));
    defer discarded.deinit();
    try std.testing.expect(returned == .success and discarded == .success);
    try close(&lease_scope, &env);
    try std.testing.expect(counts.validated.load(.acquire) >= 2);
    try std.testing.expectEqual(@as(usize, 1), counts.live.load(.acquire));
    try std.testing.expectEqual(@as(usize, 1), counts.configured.load(.acquire));
    try close(&scope, &env);
    try std.testing.expectEqual(@as(usize, 0), counts.live.load(.acquire));
    try std.testing.expectEqual(@as(usize, 2), counts.disposed.load(.acquire));
}

const Rejected = struct {
    submissions: usize = 0,
    fn executor(self: *@This()) fx.Executor {
        return .{ .userdata = self, .capabilities = .{ .independent = true, .cpu = false, .blocking = false }, .submit_fn = submit };
    }
    fn submit(data: ?*anyopaque, _: fx.ExecutionKind, _: fx.ExecutionJob) fx.Executor.Submission {
        const self: *@This() = @ptrCast(@alignCast(data.?));
        self.submissions += 1;
        return .{ .rejected = .{ .kind = .provider_unavailable, .message = "managed provider denied" } };
    }
};

test "FAC-03 FAC-04 LIFE-07 managed construction unwinds captures and provider contexts at every allocation failure" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            inline for (.{ .cache, .dependent, .pool }) |facility| {
                var rejected: Rejected = .{};
                const env: Env = .{ .allocator = allocator, .async_engine = rejected.executor() };
                var counts: Counts = .{};
                var scope = fx.Scope(E).init(&env);
                const inputs: Inputs = .{ .owner = .{ .counts = &counts }, .multiplier = 3 };
                var described = switch (facility) {
                    .cache => fx.dsl.makeCache(&env, &scope, inputs, .{ .configure = CacheRoles.configure, .load = CacheRoles.load }),
                    .dependent => fx.dsl.makeDependencyCache(&env, &scope, inputs, .{ .configure = DependencyRoles.configure, .load = DependencyRoles.load }),
                    .pool => fx.dsl.makePool(&env, &scope, inputs, .{ .configure = PoolRoles.configure, .create = PoolRoles.create, .validate = PoolRoles.validate, .dispose = PoolRoles.dispose }),
                    else => unreachable,
                };
                var result = fx.run(&env, described);
                described.deinit();
                defer result.deinit();
                try close(&scope, &env);
                try std.testing.expectEqual(@as(usize, 0), counts.live.load(.acquire));
                try std.testing.expectEqual(@as(usize, 0), counts.loaded.load(.acquire));
                if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
                try std.testing.expect(result == .die and result.die.kind == .provider_unavailable);
                try std.testing.expectEqualStrings("managed provider denied", result.die.message);
                try std.testing.expectEqual(@as(usize, 1), rejected.submissions);
            }
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

test "FAC-03 FAC-04 FAC-07 invalid runtime cache dependency and pool policy rejects before provider admission" {
    inline for (.{ .cache, .dependent, .pool }) |facility| {
        var rejected: Rejected = .{};
        const env: Env = .{ .allocator = std.testing.allocator, .async_engine = rejected.executor() };
        var counts: Counts = .{};
        var scope = fx.Scope(E).init(&env);
        const inputs: Inputs = .{ .owner = .{ .counts = &counts }, .multiplier = 3, .invalid_policy = true };
        var described = switch (facility) {
            .cache => fx.dsl.makeCache(&env, &scope, inputs, .{ .configure = CacheRoles.configure, .load = CacheRoles.load }),
            .dependent => fx.dsl.makeDependencyCache(&env, &scope, inputs, .{ .configure = DependencyRoles.configure, .load = DependencyRoles.load }),
            .pool => fx.dsl.makePool(&env, &scope, inputs, .{ .configure = PoolRoles.configure, .create = PoolRoles.create, .validate = PoolRoles.validate, .dispose = PoolRoles.dispose }),
            else => unreachable,
        };
        var result = fx.run(&env, described);
        described.deinit();
        defer result.deinit();
        if (facility == .dependent) try std.testing.expect(result == .fail and result.fail == .capacity) else try std.testing.expect(result == .die and result.die.kind == .invalid_configuration);
        try close(&scope, &env);
        try std.testing.expectEqual(@as(usize, 0), counts.live.load(.acquire));
        try std.testing.expectEqual(@as(usize, 1), counts.configured.load(.acquire));
        try std.testing.expectEqual(@as(usize, 0), rejected.submissions);
    }
}

test "FAC-03 shared cache load survives one observer cancellation and joins before capture teardown" {
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = false, .blocking = false }, .maximum_owned = 8 };
    defer engine.deinit();
    var gate: gates.Gate(u32, E) = .{ .result = .{ .success = 21 } };
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor(), .gate = &gate };
    var counts: Counts = .{};
    var scope = fx.Scope(E).init(&env);
    var described = fx.dsl.makeCache(&env, &scope, Inputs{ .owner = .{ .counts = &counts }, .multiplier = 3 }, .{ .configure = CacheRoles.trackedPolicy, .load = CacheRoles.pending });
    var made = fx.run(&env, described);
    described.deinit();
    defer made.deinit();
    try std.testing.expect(made == .success);
    const Core = fx.Cache(u32, u32, E);
    const Observed = fx.Effect(fx.CacheLookup(u32), E, Env);
    const observation = Observed.fromNativeCancellableBorrowed(Core, made.success, struct {
        fn get(value: *const Core, _: *const Env, token: ?*const fx.Cancellation) fx.Exit(fx.CacheLookup(u32), E) {
            return @constCast(value).getObservedToken(7, token);
        }
    }.get);
    var first = fx.runAsync(&env, observation);
    defer first.deinit();
    gate.started.waitUncancelable(env.io);
    var second = fx.runAsync(&env, observation);
    defer second.deinit();
    counts.matched.waitUncancelable(env.io);
    first.request();
    var cancelled = first.join();
    defer cancelled.deinit();
    try std.testing.expect(cancelled == .interrupted);
    try std.testing.expect(!gate.cancelled.load(.acquire));
    try std.testing.expectEqual(@as(usize, 1), counts.live.load(.acquire));
    gate.proceed.set(env.io);
    var completed = second.join();
    defer completed.deinit();
    try std.testing.expect(completed == .success and completed.success.value == 21 and completed.success.observation == .shared);
    try std.testing.expectEqual(@as(usize, 1), counts.loaded.load(.acquire));
    try std.testing.expect(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
    try close(&scope, &env);
    try std.testing.expectEqual(@as(usize, 0), counts.live.load(.acquire));
}

test "FAC-04 partial pool creation failure preserves typed failure and disposes its successful sibling" {
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = false, .blocking = false }, .maximum_owned = 8 };
    defer engine.deinit();
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor() };
    var counts: Counts = .{};
    var scope = fx.Scope(E).init(&env);
    var scope_closed = false;
    defer if (!scope_closed) {
        close(&scope, &env) catch {};
    };
    var described = fx.dsl.makePool(&env, &scope, Inputs{ .owner = .{ .counts = &counts }, .multiplier = 3, .fail_after_first_create = true }, .{ .create = PoolRoles.create, .configure = PoolRoles.configure, .dispose = PoolRoles.dispose, .validate = PoolRoles.validate });
    var made = fx.run(&env, described);
    described.deinit();
    defer made.deinit();
    try std.testing.expect(made == .success);
    var population = fx.EffectPool(u32, E, Env).ensureSize(made.success, 2);
    defer population.deinit();
    var populated = fx.run(&env, population);
    defer populated.deinit();
    if (!(populated == .fail and populated.fail == .expected)) std.debug.print("unexpected partial pool outcome: {any}\n", .{populated});
    try std.testing.expect(populated == .fail and populated.fail == .expected);
    scope_closed = true;
    try close(&scope, &env);
    try std.testing.expectEqual(@as(usize, 2), counts.loaded.load(.acquire));
    try std.testing.expectEqual(@as(usize, 1), counts.disposed.load(.acquire));
    try std.testing.expectEqual(@as(usize, 0), counts.live.load(.acquire));
}

test "FAC-04 LIFE-04 pool validation and disposal failures keep both typed outcomes and release capture once" {
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = false, .blocking = false }, .maximum_owned = 8 };
    defer engine.deinit();
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor() };
    var counts: Counts = .{};
    var scope = fx.Scope(E).init(&env);
    var described = fx.dsl.makePool(&env, &scope, Inputs{ .owner = .{ .counts = &counts }, .multiplier = 3, .fail_validation = true, .fail_disposal = true }, .{ .create = PoolRoles.create, .configure = PoolRoles.configure, .dispose = PoolRoles.dispose, .validate = PoolRoles.validate });
    var made = fx.run(&env, described);
    described.deinit();
    defer made.deinit();
    try std.testing.expect(made == .success);
    const Operations = fx.EffectPool(u32, E, Env);
    var population = Operations.ensureSize(made.success, 1);
    defer population.deinit();
    var populated = fx.run(&env, population);
    defer populated.deinit();
    try std.testing.expect(populated == .success);
    var lease_scope = fx.Scope(E).init(&env);
    var acquire = Operations.acquire(&lease_scope, made.success);
    defer acquire.deinit();
    var result = fx.run(&env, acquire);
    defer result.deinit();
    try std.testing.expect(result == .finalization_failed);
    try std.testing.expectEqual(E.expected, result.finalization_failed.primary.failed);
    try std.testing.expectEqual(E.capacity, result.finalization_failed.first.release_failure);
    try close(&lease_scope, &env);
    try close(&scope, &env);
    try std.testing.expectEqual(@as(usize, 1), counts.disposed.load(.acquire));
    try std.testing.expectEqual(@as(usize, 0), counts.live.load(.acquire));
}

test "FAC-03 CAP-11 captured cache TTL follows the caller clock and reloads only at expiry" {
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = false, .blocking = false }, .maximum_owned = 8 };
    defer engine.deinit();
    var clock = fx.ManualClock.init(.{ .io = std.testing.io }, .{ .nanoseconds = 0 });
    defer clock.deinit();
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor(), .clock = clock.service() };
    var counts: Counts = .{};
    var scope = fx.Scope(E).init(&env);
    var described = fx.dsl.makeCache(&env, &scope, Inputs{ .owner = .{ .counts = &counts }, .multiplier = 3, .ttl = .fromSeconds(1) }, .{ .configure = CacheRoles.configure, .load = CacheRoles.load });
    var made = fx.run(&env, described);
    described.deinit();
    defer made.deinit();
    try std.testing.expect(made == .success);
    var get = fx.EffectCache(u32, u32, E, Env).get(made.success, 7);
    defer get.deinit();
    for (0..2) |_| {
        var result = fx.run(&env, get);
        defer result.deinit();
        try std.testing.expect(result == .success and result.success == 21);
    }
    try std.testing.expectEqual(@as(usize, 1), counts.loaded.load(.acquire));
    var advanced = clock.advance(.fromSeconds(1));
    defer advanced.deinit();
    try std.testing.expect(advanced == .success);
    var reloaded = fx.run(&env, get);
    defer reloaded.deinit();
    try std.testing.expect(reloaded == .success and reloaded.success == 21);
    try std.testing.expectEqual(@as(usize, 2), counts.loaded.load(.acquire));
    try std.testing.expectEqual(@as(usize, 1), counts.configured.load(.acquire));
    try close(&scope, &env);
    try std.testing.expectEqual(@as(usize, 0), counts.live.load(.acquire));
}

test "FAC-04 CAP-11 captured pool retires expired idle resources using the caller clock" {
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = false, .blocking = false }, .maximum_owned = 8 };
    defer engine.deinit();
    var clock = fx.ManualClock.init(.{ .io = std.testing.io }, .{ .nanoseconds = 0 });
    defer clock.deinit();
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor(), .clock = clock.service() };
    var counts: Counts = .{};
    var scope = fx.Scope(E).init(&env);
    var described = fx.dsl.makePool(&env, &scope, Inputs{ .owner = .{ .counts = &counts }, .multiplier = 3, .ttl = .fromSeconds(1) }, .{ .create = PoolRoles.create, .configure = PoolRoles.configure, .dispose = PoolRoles.dispose, .validate = PoolRoles.validate });
    var made = fx.run(&env, described);
    described.deinit();
    defer made.deinit();
    try std.testing.expect(made == .success);
    const Operations = fx.EffectPool(u32, E, Env);
    var population = Operations.ensureSize(made.success, 1);
    defer population.deinit();
    var populated = fx.run(&env, population);
    defer populated.deinit();
    try std.testing.expect(populated == .success);
    var advanced = clock.advance(.fromSeconds(1));
    defer advanced.deinit();
    try std.testing.expect(advanced == .success);
    var lease_scope = fx.Scope(E).init(&env);
    var acquire = Operations.acquire(&lease_scope, made.success);
    defer acquire.deinit();
    var result = fx.run(&env, acquire);
    defer result.deinit();
    try std.testing.expect(result == .success and Operations.value(result.success) == 4);
    try std.testing.expectEqual(@as(usize, 2), counts.loaded.load(.acquire));
    try std.testing.expectEqual(@as(usize, 1), counts.disposed.load(.acquire));
    try close(&lease_scope, &env);
    try close(&scope, &env);
    try std.testing.expectEqual(@as(usize, 2), counts.disposed.load(.acquire));
    try std.testing.expectEqual(@as(usize, 0), counts.live.load(.acquire));
}

test "FAC-03 FAC-04 CAP-08 public captured facility constructors derive nested input ownership" {
    inline for (.{ .cache, .dependent, .pool }) |facility| {
        var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = false, .blocking = false }, .maximum_owned = 4 };
        defer engine.deinit();
        const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor() };
        var counts: Counts = .{};
        var scope = fx.Scope(E).init(&env);
        const inputs: Inputs = .{ .owner = .{ .counts = &counts }, .multiplier = 3 };
        var described = switch (facility) {
            .cache => fx.EffectCache(u32, u32, E, Env).makeCaptured(&env, &scope, inputs, CacheRoles.configure, CacheRoles.load),
            .dependent => fx.DependencyCache(u32, u32, u32, E, Env).makeDependentCaptured(&env, &scope, inputs, DependencyRoles.configure, DependencyRoles.load),
            .pool => fx.EffectPool(u32, E, Env).makeCaptured(&env, &scope, inputs, PoolRoles.configure, PoolRoles.create, PoolRoles.validate, PoolRoles.dispose),
            else => unreachable,
        };
        var made = fx.run(&env, described);
        described.deinit();
        defer made.deinit();
        try std.testing.expect(made == .success);
        try std.testing.expectEqual(@as(usize, 1), counts.live.load(.acquire));
        try close(&scope, &env);
        try std.testing.expectEqual(@as(usize, 0), counts.live.load(.acquire));
    }
}

test "FAC-03 LIFE-12 scope shutdown cancels and joins a pending cache loader before destroying its inputs" {
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = false, .blocking = false }, .maximum_owned = 8 };
    defer engine.deinit();
    var gate: gates.Gate(u32, E) = .{ .result = .{ .success = 21 } };
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor(), .gate = &gate };
    var counts: Counts = .{};
    var scope = fx.Scope(E).init(&env);
    var described = fx.dsl.makeCache(&env, &scope, Inputs{ .owner = .{ .counts = &counts, .pending = &gate }, .multiplier = 3 }, .{ .configure = CacheRoles.configure, .load = CacheRoles.pending });
    var made = fx.run(&env, described);
    described.deinit();
    defer made.deinit();
    try std.testing.expect(made == .success);
    var get = fx.EffectCache(u32, u32, E, Env).get(made.success, 7);
    defer get.deinit();
    var pending = fx.runAsync(&env, get);
    defer pending.deinit();
    gate.started.waitUncancelable(env.io);
    try close(&scope, &env);
    var result = pending.join();
    defer result.deinit();
    try std.testing.expect(result == .interrupted);
    try std.testing.expect(gate.cancelled.load(.acquire) and gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
    try std.testing.expectEqual(@as(usize, 0), counts.live.load(.acquire));
}

test "FAC-03 FAC-07 cache inflight admission and dependency metadata bounds retain explicit typed capacity errors" {
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = false, .blocking = false }, .maximum_owned = 8 };
    defer engine.deinit();
    var gate: gates.Gate(u32, E) = .{ .result = .{ .success = 21 } };
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor(), .gate = &gate };
    {
        var counts: Counts = .{};
        var scope = fx.Scope(E).init(&env);
        var described = fx.dsl.makeCache(&env, &scope, Inputs{ .owner = .{ .counts = &counts }, .multiplier = 3, .inflight = 1 }, .{ .configure = CacheRoles.boundedPolicy, .load = CacheRoles.pending });
        var made = fx.run(&env, described);
        described.deinit();
        defer made.deinit();
        try std.testing.expect(made == .success);
        const Operations = fx.EffectCache(u32, u32, E, Env);
        var first_description = Operations.get(made.success, 7);
        defer first_description.deinit();
        var first = fx.runAsync(&env, first_description);
        defer first.deinit();
        gate.started.waitUncancelable(env.io);
        var second_description = Operations.get(made.success, 8);
        defer second_description.deinit();
        var rejected = fx.run(&env, second_description);
        defer rejected.deinit();
        try std.testing.expect(rejected == .fail and rejected.fail == .capacity);
        gate.proceed.set(env.io);
        var completed = first.join();
        defer completed.deinit();
        try std.testing.expect(completed == .success and completed.success == 21);
        try std.testing.expectEqual(@as(usize, 1), counts.loaded.load(.acquire));
        try close(&scope, &env);
        try std.testing.expectEqual(@as(usize, 0), counts.live.load(.acquire));
    }
    {
        var counts: Counts = .{};
        var scope = fx.Scope(E).init(&env);
        var described = fx.dsl.makeDependencyCache(&env, &scope, Inputs{ .owner = .{ .counts = &counts }, .multiplier = 3, .dependency_count = 2, .dependency_limit = 1 }, .{ .configure = DependencyRoles.configure, .load = DependencyRoles.load });
        var made = fx.run(&env, described);
        described.deinit();
        defer made.deinit();
        try std.testing.expect(made == .success);
        var get = fx.DependencyCache(u32, u32, u32, E, Env).get(made.success, 7);
        defer get.deinit();
        var result = fx.run(&env, get);
        defer result.deinit();
        try std.testing.expect(result == .fail and result.fail == .capacity);
        try std.testing.expectEqual(@as(usize, 1), counts.configured.load(.acquire));
        try close(&scope, &env);
        try std.testing.expectEqual(@as(usize, 0), counts.live.load(.acquire));
    }
}

test "FAC-03 CAP-08 generated cached value owners preserve independent observations after scope closure" {
    const Value = struct {
        pub const effect_owned_payload = true;
        counts: *Counts,
        pub fn deinit(self: *@This()) void {
            _ = self.counts.values.fetchSub(1, .acq_rel);
        }
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            _ = self.counts.values.fetchAdd(1, .acq_rel);
            return self.*;
        }
    };
    const Payload = fx.dsl.ownership.Capture(struct { token: Value });
    const Result = fx.Effect(Payload, E, Env);
    const Roles = struct {
        fn load(inputs: *const Inputs, _: u32, _: *const Env) Result {
            return Result.fromBorrowed(Inputs, inputs, struct {
                fn create(value: *const Inputs, _: *const Env) fx.Exit(Payload, E) {
                    _ = value.owner.counts.values.fetchAdd(1, .acq_rel);
                    return .{ .success = .{ .value = .{ .token = .{ .counts = value.owner.counts } } } };
                }
            }.create);
        }
    };
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = false, .blocking = false }, .maximum_owned = 8 };
    defer engine.deinit();
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor() };
    var counts: Counts = .{};
    var scope = fx.Scope(E).init(&env);
    var described = fx.dsl.makeCache(&env, &scope, Inputs{ .owner = .{ .counts = &counts }, .multiplier = 3 }, .{ .configure = CacheRoles.configure, .load = Roles.load });
    var made = fx.run(&env, described);
    described.deinit();
    defer made.deinit();
    try std.testing.expect(made == .success);
    var get = fx.EffectCache(u32, Payload, E, Env).get(made.success, 7);
    var result = fx.run(&env, get);
    get.deinit();
    try std.testing.expect(result == .success);
    try close(&scope, &env);
    try std.testing.expectEqual(@as(usize, 0), counts.live.load(.acquire));
    try std.testing.expectEqual(@as(usize, 1), counts.values.load(.acquire));
    result.deinit();
    try std.testing.expectEqual(@as(usize, 0), counts.values.load(.acquire));
}

test "FAC-04 CAP-07 exclusive pooled values use generated ownership without a Clone requirement" {
    const Value = struct {
        pub const effect_owned_payload = true;
        counts: *Counts,
        pub fn deinit(self: *@This()) void {
            std.debug.assert(self.counts.values.fetchSub(1, .acq_rel) == 1);
        }
    };
    const Payload = fx.dsl.ownership.Capture(struct { token: Value });
    const Result = fx.Effect(Payload, E, Env);
    const Roles = struct {
        fn create(inputs: *const Inputs, _: *const Env) Result {
            return Result.fromBorrowed(Inputs, inputs, struct {
                fn allocate(value: *const Inputs, _: *const Env) fx.Exit(Payload, E) {
                    _ = value.owner.counts.values.fetchAdd(1, .acq_rel);
                    return .{ .success = .{ .value = .{ .token = .{ .counts = value.owner.counts } } } };
                }
            }.allocate);
        }
        fn validate(_: *const Inputs, _: Payload, _: *const Env) fx.Effect(bool, E, Env) {
            return fx.Effect(bool, E, Env).succeed(true);
        }
        fn dispose(_: *const Inputs, value: Payload, current: *const Env) Unit {
            return Unit.fromOwned(current, value, struct {
                fn run(owned: *const Payload, _: *const Env) fx.Exit(void, E) {
                    const counts = owned.value.token.counts;
                    if (counts.live.load(.acquire) != 1) return .{ .die = fx.Defect.init("pool inputs ended before owning disposal") };
                    _ = counts.disposed.fetchAdd(1, .monotonic);
                    return .{ .success = {} };
                }
            }.run);
        }
    };
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = false, .blocking = false }, .maximum_owned = 8 };
    defer engine.deinit();
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor() };
    var counts: Counts = .{};
    var scope = fx.Scope(E).init(&env);
    var described = fx.dsl.makePool(&env, &scope, Inputs{ .owner = .{ .counts = &counts }, .multiplier = 3 }, .{ .configure = PoolRoles.configure, .create = Roles.create, .validate = Roles.validate, .dispose = Roles.dispose });
    var made = fx.run(&env, described);
    described.deinit();
    defer made.deinit();
    try std.testing.expect(made == .success);
    const Operations = fx.EffectPool(Payload, E, Env);
    var lease_scope = fx.Scope(E).init(&env);
    var acquire = Operations.acquire(&lease_scope, made.success);
    defer acquire.deinit();
    var acquired = fx.run(&env, acquire);
    defer acquired.deinit();
    try std.testing.expect(acquired == .success);
    try std.testing.expectEqual(@as(usize, 1), counts.values.load(.acquire));
    try close(&lease_scope, &env);
    try std.testing.expectEqual(@as(usize, 1), counts.values.load(.acquire));
    try close(&scope, &env);
    try std.testing.expectEqual(@as(usize, 0), counts.values.load(.acquire));
    try std.testing.expectEqual(@as(usize, 0), counts.live.load(.acquire));
    try std.testing.expectEqual(@as(usize, 1), counts.disposed.load(.acquire));
}
