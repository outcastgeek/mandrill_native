const std = @import("std");
const fx = @import("effect");
const gates = @import("support/dsl_gate.zig");
const E = enum { body, admission, invalid_key };
const Mode = enum { success, fail, die, interrupted };
const State = struct { acquired: usize = 0, released: usize = 0, order: [2]u8 = .{ 0, 0 }, bodies: usize = 0, live: std.atomic.Value(usize) = .init(0), gate: ?*gates.Gate(u32, E) = null, invalid: bool = false };
const Env = struct {
    allocator: std.mem.Allocator,
    io: std.Io = std.testing.io,
    async_engine: fx.Executor = undefined,
    state: *State,
    mode: Mode = .success,
    fail_right: bool = false,
    semaphore: *fx.Semaphore = undefined,
    partitions: *fx.PartitionedSemaphore = undefined,
    gate: ?*gates.Gate(u32, E) = null,
    key: []const u8 = "",
};
const Unit = fx.Effect(void, E, Env);
const Resource = struct { allocator: std.mem.Allocator, value: u32, id: u8, origin: *const Env };
const Services = struct { left: *Resource, right: *Resource, right_env: BodyEnv };
const LayerRoles = struct {
    fn acquire(comptime id: u8) fx.Effect(*Resource, E, Env) {
        return fx.Effect(*Resource, E, Env).fromOutcome(struct {
            fn run(env: *const Env) fx.Exit(*Resource, E) {
                if (id == 2 and env.fail_right) return .{ .fail = .body };
                const value = env.allocator.create(Resource) catch return .{ .die = fx.Defect.out_of_memory };
                value.* = .{ .allocator = env.allocator, .value = id * 10, .id = id, .origin = env };
                env.state.acquired += 1;
                return .{ .success = value };
            }
        }.run);
    }
    fn release(resource: *Resource, _: *const Env) Unit {
        return Unit.fromBorrowed(Resource, resource, struct {
            fn run(value: *const Resource, env: *const Env) fx.Exit(void, E) {
                env.state.order[env.state.released] = value.id;
                env.state.released += 1;
                value.allocator.destroy(@constCast(value));
                return .{ .success = {} };
            }
        }.run);
    }
    fn combine(first: anytype, second: *Resource) Services {
        return .{ .left = first, .right = second, .right_env = .{ .resource = second } };
    }
};
const left = fx.dsl.layer(.{ .acquire = LayerRoles.acquire(1), .release = LayerRoles.release });
const right = fx.dsl.layer(.{ .acquire = LayerRoles.acquire(2), .release = LayerRoles.release });
const merged = fx.dsl.mergeLayers(left, right, .{ .combine = LayerRoles.combine });
const BodyEnv = struct { resource: *Resource };
const body = fx.Effect(u32, E, BodyEnv).sync(struct {
    fn run(env: *const BodyEnv) u32 {
        return env.resource.value;
    }
}.run);

test "RES-04 COMP-03 layers infer borrowed merged services and honor explicit body projection" {
    const Result = fx.Effect(u32, E, Services).sync(struct {
        fn run(services: *const Services) u32 {
            return services.left.value + services.right.value;
        }
    }.run);
    const described = comptime fx.dsl.provideLayer(merged, Result);
    comptime if (@TypeOf(described) != fx.Effect(u32, E, Env)) @compileError("provided layer must return exact Effect");
    var state: State = .{};
    const env: Env = .{ .allocator = std.testing.allocator, .state = &state };
    try std.testing.expectEqual(@as(usize, 0), state.acquired);
    try std.testing.expectEqual(@as(u32, 30), fx.run(&env, described).success);
    try std.testing.expectEqualSlices(u8, &.{ 2, 1 }, &state.order);
    const projected = comptime body.provideSome(Services, struct {
        fn project(services: *const Services) *const BodyEnv {
            return &services.right_env;
        }
    }.project);
    state = .{};
    try std.testing.expectEqual(@as(u32, 20), fx.run(&env, comptime fx.dsl.provideLayer(merged, projected)).success);
    try std.testing.expectEqualSlices(u8, &.{ 2, 1 }, &state.order);
}

test "RES-04 LIFE-03 layer partial acquisition and every allocation failure release only installed resources" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            inline for (.{ false, true }) |fail_right| {
                var state: State = .{};
                const env: Env = .{ .allocator = allocator, .state = &state, .fail_right = fail_right };
                const program = comptime fx.Effect(u32, E, Services).sync(struct {
                    fn run(value: *const Services) u32 {
                        return value.left.value + value.right.value;
                    }
                }.run);
                var result = fx.run(&env, comptime fx.dsl.provideLayer(merged, program));
                defer result.deinit();
                try std.testing.expectEqual(state.acquired, state.released);
                if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
                if (fail_right) {
                    try std.testing.expect(result == .fail and result.fail == .body);
                    try std.testing.expectEqual(@as(usize, 1), state.released);
                    try std.testing.expectEqual(@as(u8, 1), state.order[0]);
                } else try std.testing.expectEqual(@as(u32, 30), result.success);
            }
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

test "RES-04 composed layers preserve inferred intermediate environments and scoped ownership" {
    const second = comptime fx.dsl.layer(.{ .acquire = fx.Effect(BodyEnv, E, *Resource).sync(struct {
        fn run(value: *const *Resource) BodyEnv {
            return .{ .resource = value.* };
        }
    }.run) });
    const composed = comptime fx.dsl.composeLayers(left, second);
    const described = comptime fx.dsl.provideLayer(composed, body);
    comptime if (@TypeOf(described) != fx.Effect(u32, E, Env)) @compileError("layer composition must return exact Effect");
    var state: State = .{};
    const env: Env = .{ .allocator = std.testing.allocator, .state = &state };
    var result = fx.run(&env, described);
    defer result.deinit();
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(u32, 10), result.success);
    try std.testing.expectEqual(@as(usize, 1), state.released);
}

test "RES-04 LIFE-01 layer provision preserves body terminal channels and reverse release" {
    const described = comptime fx.dsl.provideLayer(merged, fx.Effect(u32, E, Services).fromOutcome(struct {
        fn run(value: *const Services) fx.Exit(u32, E) {
            return switch (value.left.origin.mode) {
                .success => .{ .success = value.left.value + value.right.value },
                .fail => .{ .fail = .body },
                .die => .{ .die = fx.Defect.init("layer body defect") },
                .interrupted => .interrupted,
            };
        }
    }.run));
    for (std.meta.tags(Mode)) |mode| {
        var state: State = .{};
        const env: Env = .{ .allocator = std.testing.allocator, .state = &state, .mode = mode };
        var result = fx.run(&env, described);
        defer result.deinit();
        switch (mode) {
            .success => {
                try std.testing.expect(result == .success);
                try std.testing.expectEqual(@as(u32, 30), result.success);
            },
            .fail => try std.testing.expect(result == .fail and result.fail == .body),
            .die => try std.testing.expect(result == .die),
            .interrupted => try std.testing.expect(result == .interrupted),
        }
        try std.testing.expectEqualSlices(u8, &.{ 2, 1 }, &state.order);
    }
}

const PermitRoles = struct {
    fn select(env: *const Env) *fx.Semaphore {
        return env.semaphore;
    }
    fn partitioned(env: *const Env) *fx.PartitionedSemaphore {
        return env.partitions;
    }
    fn key(env: *const Env) []const u8 {
        return env.key;
    }
    fn mapError(_: fx.SemaphoreError) E {
        return .admission;
    }
    fn mapPartitionError(problem: fx.PartitionedSemaphoreError) E {
        return if (problem == .invalid_key) .invalid_key else .admission;
    }
    fn body(_: *const Env) fx.Effect(u32, E, Env) {
        return fx.Effect(u32, E, Env).fromOutcome(struct {
            fn run(current: *const Env) fx.Exit(u32, E) {
                current.state.bodies += 1;
                return switch (current.mode) {
                    .success => .{ .success = 41 },
                    .fail => .{ .fail = .body },
                    .die => .{ .die = fx.Defect.init("permit body defect") },
                    .interrupted => .interrupted,
                };
            }
        }.run);
    }
};
const semaphore_roles = .{ .select = PermitRoles.select, .map_error = PermitRoles.mapError, .body = PermitRoles.body };
const partition_roles = .{ .select = PermitRoles.partitioned, .key = PermitRoles.key, .map_error = PermitRoles.mapPartitionError, .body = PermitRoles.body };
const semaphore_program = fx.dsl.withPermit(semaphore_roles);
const partition_program = fx.dsl.withPermit(partition_roles);

test "RES-06 LIFE-01 inferred semaphore and partitioned bodies release permits on all terminal channels" {
    inline for (.{ false, true }) |partitioned| {
        for (std.meta.tags(Mode)) |mode| {
            var state: State = .{};
            var env: Env = .{ .allocator = std.testing.allocator, .state = &state, .mode = mode };
            var encoded = fx.PrimaryKey.encodeDirect(&env, "permit-proof", &.{});
            defer encoded.deinit();
            try std.testing.expect(encoded == .success);
            env.key = encoded.success.items;
            var semaphore = fx.dsl.permits.semaphoreStatic(&env, 1).ok;
            defer semaphore.deinit();
            var partitions = fx.dsl.permits.partitionsStatic(&env, 2, 1).ok;
            defer partitions.deinit();
            defer partitions.shutdown();
            env.semaphore = &semaphore;
            env.partitions = &partitions;
            const described = if (partitioned) partition_program else semaphore_program;
            comptime if (@TypeOf(described) != fx.Effect(u32, E, Env)) @compileError("permit body must return exact Effect");
            try std.testing.expectEqual(@as(usize, 0), state.bodies);
            var result = fx.run(&env, described);
            defer result.deinit();
            const expected: @typeInfo(fx.Exit(u32, E)).@"union".tag_type.? = switch (mode) {
                .success => .success,
                .fail => .fail,
                .die => .die,
                .interrupted => .interrupted,
            };
            try std.testing.expectEqual(expected, std.meta.activeTag(result));
            switch (mode) {
                .success => try std.testing.expectEqual(@as(u32, 41), result.success),
                .fail => try std.testing.expect(result == .fail and result.fail == .body),
                .die => try std.testing.expect(result == .die),
                .interrupted => try std.testing.expect(result == .interrupted),
            }
            try std.testing.expectEqual(@as(usize, 1), state.bodies);
            try std.testing.expectEqual(semaphore.limit, semaphore.available);
            try std.testing.expectEqual(@as(usize, 0), partitions.entries.count());
        }
    }
}

test "RES-06 COLL-04 permit bounds and shutdown admission preserve explicit typed mapping" {
    var state: State = .{};
    var env: Env = .{ .allocator = std.testing.allocator, .state = &state };
    var encoded = fx.PrimaryKey.encodeDirect(&env, "permit-proof", &.{});
    defer encoded.deinit();
    try std.testing.expect(encoded == .success);
    env.key = encoded.success.items;
    try std.testing.expect(fx.dsl.permits.semaphore(&env, 0) == .err);
    try std.testing.expect(fx.dsl.permits.partitions(&env, 0, 1) == .err);
    try std.testing.expect(fx.dsl.permits.partitions(&env, 1, 0) == .err);
    var semaphore = fx.dsl.permits.semaphore(&env, 1).ok;
    defer semaphore.deinit();
    var partitions = fx.dsl.permits.partitions(&env, 1, 1).ok;
    defer partitions.deinit();
    env.semaphore = &semaphore;
    env.partitions = &partitions;
    semaphore.shutdown();
    partitions.shutdown();
    inline for (.{ false, true }) |partitioned| {
        var result = fx.run(&env, if (partitioned) partition_program else semaphore_program);
        defer result.deinit();
        try std.testing.expect(result == .fail and result.fail == .admission);
    }
    var invalid_partitions = fx.dsl.permits.partitions(&env, 1, 1).ok;
    defer invalid_partitions.deinit();
    defer invalid_partitions.shutdown();
    env.partitions = &invalid_partitions;
    env.key = "malformed";
    var invalid = fx.run(&env, partition_program);
    defer invalid.deinit();
    try std.testing.expect(invalid == .fail and invalid.fail == .invalid_key);
    try std.testing.expectEqual(@as(usize, 0), state.bodies);
}

const Capture = struct {
    pub const effect_owned_payload = true;
    state: *State,
    pub fn deinit(self: *@This()) void {
        if (self.state.gate) |gate| if (!gate.destroyed.load(.acquire)) {
            self.state.invalid = true;
        };
        _ = self.state.live.fetchSub(1, .acq_rel);
    }
};

test "RES-06 CAP-04 suspended captured permit bodies drain cancellation before owner teardown" {
    const Pending = struct {
        fn body(stored: *const Capture, _: *const Env) fx.Effect(u32, E, Env) {
            return fx.Effect(u32, E, Env).fromAsyncBorrowed(Capture, stored, struct {
                fn start(value: *const Capture, env: *const Env) fx.Async(u32, E).Start {
                    if (env.state.live.load(.acquire) == 0) return .{ .completed = .{ .die = fx.Defect.init("permit capture ended before pending body") } };
                    if (value.state != env.state) env.state.invalid = true;
                    return env.gate.?.start();
                }
            }.start);
        }
    };
    inline for (.{ false, true }) |partitioned| {
        var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = true }, .maximum_owned = 4 };
        defer engine.deinit();
        var state: State = .{};
        var gate: gates.Gate(u32, E) = .{ .result = .{ .success = 41 } };
        state.gate = &gate;
        var env: Env = .{ .allocator = std.testing.allocator, .state = &state, .gate = &gate, .async_engine = engine.executor() };
        var encoded = fx.PrimaryKey.encodeDirect(&env, "permit-proof", &.{});
        defer encoded.deinit();
        try std.testing.expect(encoded == .success);
        env.key = encoded.success.items;
        var semaphore = fx.dsl.permits.semaphore(&env, 1).ok;
        defer semaphore.deinit();
        var partitions = fx.dsl.permits.partitions(&env, 1, 1).ok;
        defer partitions.deinit();
        defer partitions.shutdown();
        env.semaphore = &semaphore;
        env.partitions = &partitions;
        _ = state.live.fetchAdd(1, .acq_rel);
        var described = if (partitioned)
            fx.dsl.withPermitCaptured(&env, Capture{ .state = &state }, .{ .select = PermitRoles.partitioned, .key = PermitRoles.key, .map_error = PermitRoles.mapPartitionError, .body = Pending.body })
        else
            fx.dsl.withPermitCaptured(&env, Capture{ .state = &state }, .{ .select = PermitRoles.select, .map_error = PermitRoles.mapError, .body = Pending.body });
        var pending = fx.runAsync(&env, described);
        defer pending.deinit();
        gate.started.waitUncancelable(env.io);
        try std.testing.expectEqual(@as(usize, 1), state.live.load(.acquire));
        pending.request();
        var result = pending.join();
        try std.testing.expect(result == .interrupted);
        result.deinit();
        described.deinit();
        try std.testing.expect(gate.destroyed.load(.acquire));
        try std.testing.expectEqual(@as(usize, 0), state.live.load(.acquire));
        try std.testing.expectEqual(semaphore.limit, semaphore.available);
        try std.testing.expectEqual(@as(usize, 0), partitions.entries.count());
        try std.testing.expect(!state.invalid);
    }
}

test "RES-06 LIFE-07 captured permit allocation failures leave no permit partition or input owner" {
    const Roles = struct {
        fn body(_: *const Capture, env: *const Env) fx.Effect(u32, E, Env) {
            return PermitRoles.body(env);
        }
    };
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            inline for (.{ false, true }) |partitioned| {
                var state: State = .{};
                var env: Env = .{ .allocator = allocator, .state = &state };
                var encoded = fx.PrimaryKey.encodeDirect(&env, "permit-proof", &.{});
                defer encoded.deinit();
                if (encoded == .die and encoded.die.kind == .out_of_memory) return error.OutOfMemory;
                try std.testing.expect(encoded == .success);
                env.key = encoded.success.items;
                var semaphore = fx.dsl.permits.semaphore(&env, 1).ok;
                defer semaphore.deinit();
                var partitions = fx.dsl.permits.partitions(&env, 1, 1).ok;
                defer partitions.deinit();
                defer partitions.shutdown();
                env.semaphore = &semaphore;
                env.partitions = &partitions;
                _ = state.live.fetchAdd(1, .acq_rel);
                var described = if (partitioned)
                    fx.dsl.withPermitCaptured(&env, Capture{ .state = &state }, .{ .select = PermitRoles.partitioned, .key = PermitRoles.key, .map_error = PermitRoles.mapPartitionError, .body = Roles.body })
                else
                    fx.dsl.withPermitCaptured(&env, Capture{ .state = &state }, .{ .select = PermitRoles.select, .map_error = PermitRoles.mapError, .body = Roles.body });
                var result = fx.run(&env, described);
                described.deinit();
                defer result.deinit();
                try std.testing.expectEqual(@as(usize, 0), state.live.load(.acquire));
                try std.testing.expectEqual(semaphore.limit, semaphore.available);
                try std.testing.expectEqual(@as(usize, 0), partitions.entries.count());
                if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
                try std.testing.expectEqual(@as(u32, 41), result.success);
            }
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}
