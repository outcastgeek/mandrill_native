const std = @import("std");
const fx = @import("effect");
const gates = @import("support/dsl_gate.zig");
const Threads = @import("support/thread_executor.zig").ThreadExecutor;
const Gate = gates.Gate(u32, u8);
const ReleaseGate = gates.Gate(void, u8);
const Counts = struct { acquired: usize = 0, projected: usize = 0, service_closed: usize = 0, environment_closed: usize = 0, release_calls: usize = 0, handler_calls: usize = 0 };
const Env = struct {
    allocator: std.mem.Allocator,
    io: std.Io = std.testing.io,
    async_engine: fx.Executor,
    counts: *Counts,
    gate: *Gate,
    release_gate: ?*ReleaseGate = null,
    projection_gate: ?*ReleaseGate = null,
};
const Service = struct {
    pub const effect_owned_payload = true;
    counts: *Counts,
    value: u32,
    pub fn deinit(self: *@This()) void {
        std.debug.assert(self.counts.environment_closed == self.counts.projected);
        self.counts.service_closed += 1;
    }
};
const HandlerEnv = struct {
    pub const effect_owned_payload = true;
    allocator: std.mem.Allocator,
    io: std.Io,
    async_engine: fx.Executor,
    service: *const Service,
    gate: *Gate,
    pub fn deinit(self: *@This()) void {
        std.debug.assert(self.service.counts.service_closed == 0);
        self.service.counts.environment_closed += 1;
    }
};
const Acquire = fx.Effect(Service, u8, Env);
const Body = fx.Effect(u32, u8, HandlerEnv);
const Adapter = struct {
    fn acquire(env: *const Env) fx.Exit(Service, u8) {
        env.counts.acquired += 1;
        return .{ .success = .{ .counts = env.counts, .value = 41 } };
    }
    fn project(input: *const u32, service: *const Service, env: *const Env) HandlerEnv {
        std.debug.assert(input.* == 1);
        env.counts.projected += 1;
        return .{ .allocator = env.allocator, .io = env.io, .async_engine = env.async_engine, .service = service, .gate = env.gate };
    }
    fn use(input: *const u32, env: *const HandlerEnv) Body {
        std.debug.assert(input.* + env.service.value == 42);
        return Body.fromAsync(struct {
            fn start(current: *const HandlerEnv) fx.Async(u32, u8).Start {
                std.debug.assert(current.service.counts.service_closed == 0);
                return current.gate.start();
            }
        }.start);
    }
};

test "RPC-03 LIFE-12 runtime middleware shares the original owner and keeps service and projected environment through pending outcomes" {
    const Mode = enum { success, fail, die, interrupted, cancel, finalization_failed };
    inline for (std.meta.tags(Mode)) |mode| {
        var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 4, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
        defer threads.deinit();
        var counts: Counts = .{};
        var gate: Gate = .{ .result = switch (mode) {
            .success, .cancel => .{ .success = 42 },
            .fail => .{ .fail = 9 },
            .die => .{ .die = fx.Defect.init("middleware body") },
            .interrupted => .interrupted,
            .finalization_failed => .{ .finalization_failed = .{ .primary = .{ .failed = 7 }, .first = .{ .release_failure = 8 } } },
        } };
        const env: Env = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &counts, .gate = &gate };
        var middleware: fx.RpcMiddleware(Service, u8, Env) = .{ .acquire = Acquire.fromOutcome(Adapter.acquire) };
        const input: u32 = 1;
        var description = middleware.applyOwnedTo(u32, u32, u8, HandlerEnv, Adapter.project, Adapter.use, &input, &env);
        comptime if (@TypeOf(description) != fx.Effect(u32, fx.RpcMiddlewareFailure(u8, u8), Env)) @compileError("middleware must remain an exact Effect");
        try std.testing.expectEqual(@as(usize, 0), counts.acquired);
        var pending = fx.runAsync(&env, description);
        description.deinit();
        gate.started.waitUncancelable(env.io);
        try std.testing.expectEqual(@as(usize, 1), counts.acquired);
        try std.testing.expectEqual(@as(usize, 1), counts.projected);
        try std.testing.expectEqual(@as(usize, 0), counts.environment_closed);
        try std.testing.expectEqual(@as(usize, 0), counts.service_closed);
        if (mode == .cancel) pending.request() else gate.proceed.set(env.io);
        var result = pending.join();
        pending.deinit();
        defer result.deinit();
        try std.testing.expect(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
        try std.testing.expectEqual(@as(usize, 1), counts.environment_closed);
        try std.testing.expectEqual(@as(usize, 1), counts.service_closed);
        switch (mode) {
            .success => try std.testing.expectEqual(@as(u32, 42), result.success),
            .fail => try std.testing.expectEqual(@as(u8, 9), result.fail.domain),
            .die => try std.testing.expect(result == .die),
            .interrupted, .cancel => try std.testing.expect(result == .interrupted),
            .finalization_failed => {
                try std.testing.expectEqual(@as(u8, 7), result.finalization_failed.primary.failed.domain);
                try std.testing.expectEqual(@as(u8, 8), result.finalization_failed.first.release_failure.domain);
            },
        }
    }
}

const ConfigurationOwner = struct {
    pub const effect_owned_payload = true;
    allocator: std.mem.Allocator,
    bytes: []u8,
    live: *usize,
    fn init(allocator: std.mem.Allocator, live: *usize) !@This() {
        const bytes = try allocator.dupe(u8, "middleware configuration");
        live.* += 1;
        return .{ .allocator = allocator, .bytes = bytes, .live = live };
    }
    pub fn deinit(self: *@This()) void {
        self.allocator.free(self.bytes);
        self.live.* -= 1;
    }
};
const Inferred = struct {
    fn acquire(configuration: anytype, _: *const Env) Acquire {
        const C = @typeInfo(@TypeOf(configuration)).pointer.child;
        return Acquire.fromBorrowed(C, configuration, struct {
            fn run(stored: *const C, current: *const Env) fx.Exit(Service, u8) {
                std.debug.assert(std.mem.eql(u8, stored.owner.bytes, "middleware configuration"));
                if (stored.acquire_failure) return .{ .fail = 20 };
                return Adapter.acquire(current);
            }
        }.run);
    }
    fn project(configuration: anytype, service: *const Service, current: *const Env) fx.Result(HandlerEnv, u8) {
        if (configuration.project_failure) return .{ .err = 21 };
        const input: u32 = 1;
        return .{ .ok = Adapter.project(&input, service, current) };
    }
    fn use(configuration: anytype, current: *const HandlerEnv) Body {
        current.service.counts.handler_calls += 1;
        std.debug.assert(std.mem.eql(u8, configuration.owner.bytes, "middleware configuration"));
        if (configuration.body_failure) return Body.fail(22);
        return Body.succeed(current.service.value + configuration.addend);
    }
    fn pending(configuration: anytype, current: *const HandlerEnv) Body {
        std.debug.assert(configuration.addend == 1);
        const input: u32 = 1;
        return Adapter.use(&input, current);
    }
};

// Public operation: withRpcMiddleware; setup: one owned anonymous declaration,
// two different execution environments; assertion: exact types and fresh owners.
// Harness registration: tests/dsl_execution.zon and tests/dsl_acceptance.json.
test "RPC-03 DECL-03 CAP-02 inferred middleware evaluates configuration once and acquires fresh service and projection for each run" {
    var live: usize = 0;
    var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
    defer threads.deinit();
    var first: Counts = .{};
    var second: Counts = .{};
    var gate: Gate = .{ .result = .{ .success = 42 } };
    const first_env: Env = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &first, .gate = &gate };
    const second_env: Env = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &second, .gate = &gate };
    var described = fx.dsl.withRpcMiddleware(&first_env, .{
        .definition = fx.dsl.declareMetadata(.{ .use = Inferred.use, .project = Inferred.project, .acquire = Inferred.acquire }),
        .inputs = .{ .owner = try ConfigurationOwner.init(std.testing.allocator, &live), .addend = @as(u32, 1), .acquire_failure = false, .project_failure = false, .body_failure = false },
    });
    comptime if (@TypeOf(described) != fx.Effect(u32, fx.RpcMiddlewareFailure(u8, u8), Env)) @compileError("inferred middleware changed Effect channels");
    try std.testing.expectEqual(@as(usize, 0), first.acquired);
    var a = fx.run(&first_env, described);
    var b = fx.run(&second_env, described);
    defer a.deinit();
    defer b.deinit();
    try std.testing.expectEqual(@as(u32, 42), a.success);
    try std.testing.expectEqual(@as(u32, 42), b.success);
    try std.testing.expectEqual(@as(usize, 1), first.service_closed);
    try std.testing.expectEqual(@as(usize, 1), second.service_closed);
    try std.testing.expectEqual(@as(usize, 1), live);
    described.deinit();
    try std.testing.expectEqual(@as(usize, 0), live);
}

test "RPC-05 inferred middleware keeps acquisition projection and body errors distinct and rolls back only initialized owners" {
    inline for (.{ .acquire, .project, .body }) |stage| {
        var live: usize = 0;
        var counts: Counts = .{};
        var gate: Gate = .{ .result = .{ .success = 42 } };
        var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
        defer threads.deinit();
        const env: Env = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &counts, .gate = &gate };
        var described = fx.dsl.withRpcMiddleware(&env, .{
            .definition = fx.dsl.declareMetadata(.{ .acquire = Inferred.acquire, .project = Inferred.project, .use = Inferred.use }),
            .inputs = .{ .owner = try ConfigurationOwner.init(std.testing.allocator, &live), .addend = @as(u32, 1), .acquire_failure = stage == .acquire, .project_failure = stage == .project, .body_failure = stage == .body },
        });
        var result = fx.run(&env, described);
        described.deinit();
        defer result.deinit();
        switch (stage) {
            .acquire => try std.testing.expectEqual(@as(u8, 20), result.fail.middleware),
            .project => try std.testing.expectEqual(@as(u8, 21), result.fail.middleware),
            .body => try std.testing.expectEqual(@as(u8, 22), result.fail.domain),
            else => unreachable,
        }
        try std.testing.expectEqual(@as(usize, if (stage == .body) 1 else 0), counts.environment_closed);
        try std.testing.expectEqual(@as(usize, if (stage == .acquire) 0 else 1), counts.service_closed);
        try std.testing.expectEqual(@as(usize, 0), live);
    }
}

test "RPC-03 CAP-04 LIFE-12 inferred middleware retains anonymous configuration after descriptor disposal until cancellation drains" {
    var live: usize = 0;
    var counts: Counts = .{};
    var gate: Gate = .{ .result = .{ .success = 42 } };
    var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
    defer threads.deinit();
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &counts, .gate = &gate };
    var described = fx.dsl.withRpcMiddleware(&env, .{
        .definition = fx.dsl.declareMetadata(.{ .acquire = Inferred.acquire, .project = Inferred.project, .use = Inferred.pending }),
        .inputs = .{ .owner = try ConfigurationOwner.init(std.testing.allocator, &live), .addend = @as(u32, 1), .acquire_failure = false, .project_failure = false },
    });
    var pending = fx.runAsync(&env, described);
    described.deinit();
    gate.started.waitUncancelable(env.io);
    try std.testing.expectEqual(@as(usize, 1), live);
    try std.testing.expectEqual(@as(usize, 0), counts.service_closed);
    pending.request();
    var result = pending.join();
    pending.deinit();
    defer result.deinit();
    try std.testing.expect(result == .interrupted);
    try std.testing.expect(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
    try std.testing.expectEqual(@as(usize, 1), counts.environment_closed);
    try std.testing.expectEqual(@as(usize, 1), counts.service_closed);
    try std.testing.expectEqual(@as(usize, 0), live);
}

test "RPC-03 RPC-05 LIFE-07 every middleware allocation failure rolls back configuration service and projected environment" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator, project_failure: bool) !void {
            var live: usize = 0;
            defer std.debug.assert(live == 0);
            var counts: Counts = .{};
            var gate: Gate = .{ .result = .{ .success = 42 } };
            var threads: Threads = .{ .allocator = allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
            defer threads.deinit();
            const env: Env = .{ .allocator = allocator, .async_engine = threads.executor(), .counts = &counts, .gate = &gate };
            var described = fx.dsl.withRpcMiddleware(&env, .{
                .definition = fx.dsl.declareMetadata(.{ .acquire = Inferred.acquire, .project = Inferred.project, .use = Inferred.use }),
                .inputs = .{ .owner = try ConfigurationOwner.init(allocator, &live), .addend = @as(u32, 1), .acquire_failure = false, .project_failure = project_failure, .body_failure = false },
            });
            defer described.deinit();
            var result = fx.run(&env, described);
            defer result.deinit();
            try std.testing.expectEqual(counts.acquired, counts.service_closed);
            try std.testing.expectEqual(counts.projected, counts.environment_closed);
            if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
            if (project_failure) try std.testing.expectEqual(@as(u8, 21), result.fail.middleware) else try std.testing.expectEqual(@as(u32, 42), result.success);
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{false});
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{true});
}

const Managed = struct {
    fn release(configuration: anytype, service: *const Service, primary: *const fx.PrimaryOutcome(fx.RpcMiddlewareFailure(u8, u8)), current: *const Env) fx.Effect(void, u8, Env) {
        std.debug.assert(service.counts.service_closed == 0);
        std.debug.assert(service.counts.projected == service.counts.environment_closed);
        std.debug.assert(std.mem.eql(u8, configuration.owner.bytes, "middleware configuration"));
        current.counts.release_calls += 1;
        if (configuration.body_failure and primary.* == .failed) std.debug.assert(primary.failed.domain == 22);
        if (current.release_gate != null) return fx.Effect(void, u8, Env).fromAsync(struct {
            fn start(env: *const Env) fx.Async(void, u8).Start {
                std.debug.assert(env.counts.service_closed == 0);
                return env.release_gate.?.start();
            }
        }.start);
        return if (configuration.release_failure) fx.Effect(void, u8, Env).fail(23) else fx.Effect(void, u8, Env).succeed({});
    }
    fn late(configuration: anytype, _: *const Env) Acquire {
        const C = @typeInfo(@TypeOf(configuration)).pointer.child;
        return Acquire.fromCancellableBorrowed(C, configuration, struct {
            fn start(_: *const C, env: *const Env, token: ?*const fx.Cancellation) fx.Exit(Service, u8) {
                const acquired = Adapter.acquire(env);
                @constCast(token.?).request();
                return acquired;
            }
        }.start);
    }
};

test "RPC-05 managed middleware preserves body and release failures and releases a successful late acquisition" {
    inline for (.{ false, true }) |late| {
        var live: usize = 0;
        var counts: Counts = .{};
        var gate: Gate = .{ .result = .{ .success = 42 } };
        var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
        defer threads.deinit();
        const env: Env = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &counts, .gate = &gate };
        var described = fx.dsl.withRpcMiddleware(&env, .{
            .definition = fx.dsl.declareMetadata(.{ .acquire = if (late) Managed.late else Inferred.acquire, .project = Inferred.project, .use = Inferred.use, .release = Managed.release }),
            .inputs = .{ .owner = try ConfigurationOwner.init(std.testing.allocator, &live), .addend = @as(u32, 1), .acquire_failure = false, .project_failure = false, .body_failure = !late, .release_failure = !late },
        });
        var result = if (late) blk: {
            var pending = fx.runAsync(&env, described);
            const joined = pending.join();
            pending.deinit();
            break :blk joined;
        } else fx.run(&env, described);
        described.deinit();
        defer result.deinit();
        if (late) {
            try std.testing.expect(result == .interrupted);
            try std.testing.expectEqual(@as(usize, 0), counts.projected);
        } else {
            try std.testing.expect(result == .finalization_failed);
            try std.testing.expectEqual(@as(u8, 22), result.finalization_failed.primary.failed.domain);
            try std.testing.expectEqual(@as(u8, 23), result.finalization_failed.first.release_failure.middleware);
        }
        try std.testing.expectEqual(@as(usize, 1), counts.release_calls);
        try std.testing.expectEqual(@as(usize, 1), counts.service_closed);
        try std.testing.expectEqual(counts.projected, counts.environment_closed);
        try std.testing.expectEqual(@as(usize, 0), live);
    }
}

test "RPC-03 RPC-05 LIFE-12 managed release suspends after projection cleanup and retains service and configuration through complete finalization" {
    inline for (.{ false, true }) |fails| {
        var live: usize = 0;
        var counts: Counts = .{};
        var gate: Gate = .{ .result = .{ .success = 42 } };
        var released: ReleaseGate = .{ .result = if (fails) .{ .fail = 24 } else .{ .success = {} } };
        var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
        defer threads.deinit();
        const env: Env = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &counts, .gate = &gate, .release_gate = &released };
        var described = fx.dsl.withRpcMiddleware(&env, .{
            .definition = fx.dsl.declareMetadata(.{ .acquire = Inferred.acquire, .project = Inferred.project, .use = Inferred.use, .release = Managed.release }),
            .inputs = .{ .owner = try ConfigurationOwner.init(std.testing.allocator, &live), .addend = @as(u32, 1), .acquire_failure = false, .project_failure = false, .body_failure = false, .release_failure = false },
        });
        var pending = fx.runAsync(&env, described);
        described.deinit();
        released.started.waitUncancelable(env.io);
        try std.testing.expectEqual(@as(usize, 1), counts.environment_closed);
        try std.testing.expectEqual(@as(usize, 0), counts.service_closed);
        try std.testing.expectEqual(@as(usize, 1), live);
        // Cancellation cannot discard an admitted protected finalizer.
        pending.request();
        released.proceed.set(env.io);
        var result = pending.join();
        pending.deinit();
        defer result.deinit();
        try std.testing.expect(released.joined.load(.acquire) and released.destroyed.load(.acquire));
        if (fails) {
            try std.testing.expect(result == .finalization_failed);
            try std.testing.expectEqual(@as(u8, 24), result.finalization_failed.first.release_failure.middleware);
        } else try std.testing.expect(result == .interrupted);
        try std.testing.expectEqual(@as(usize, 1), counts.release_calls);
        try std.testing.expectEqual(@as(usize, 1), counts.service_closed);
        try std.testing.expectEqual(@as(usize, 0), live);
    }
}

test "RPC-03 COMP-03 original static middleware API delegates to the same bracket and preserves exact Effect channels" {
    const Static = struct {
        fn project(service: *const Service, env: *const Env) HandlerEnv {
            const input: u32 = 1;
            return Adapter.project(&input, service, env);
        }
        fn handle(input: *const u32, env: *const HandlerEnv) Body {
            return Body.succeed(env.service.value + input.*);
        }
    };
    var counts: Counts = .{};
    var gate: Gate = .{ .result = .{ .success = 42 } };
    var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
    defer threads.deinit();
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &counts, .gate = &gate };
    const middleware = comptime fx.RpcMiddleware(Service, u8, Env).fromEffect(Acquire.fromOutcome(Adapter.acquire));
    const input: u32 = 1;
    var described = middleware.applyTo(u32, u32, u8, HandlerEnv, Static.project, Static.handle, &input, &env);
    var result = fx.run(&env, described);
    described.deinit();
    defer result.deinit();
    try std.testing.expectEqual(@as(u32, 42), result.success);
    try std.testing.expectEqual(@as(usize, 1), counts.environment_closed);
    try std.testing.expectEqual(@as(usize, 1), counts.service_closed);
}

test "RPC-05 LIFE-07 managed middleware allocation failures still invoke release once for every acquired service" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator, project_failure: bool, body_failure: bool, release_failure: bool) !void {
            var live: usize = 0;
            defer std.debug.assert(live == 0);
            var counts: Counts = .{};
            var gate: Gate = .{ .result = .{ .success = 42 } };
            var threads: Threads = .{ .allocator = allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
            defer threads.deinit();
            const env: Env = .{ .allocator = allocator, .async_engine = threads.executor(), .counts = &counts, .gate = &gate };
            var described = fx.dsl.withRpcMiddleware(&env, .{
                .definition = fx.dsl.declareMetadata(.{ .acquire = Inferred.acquire, .project = Inferred.project, .use = Inferred.use, .release = Managed.release }),
                .inputs = .{ .owner = try ConfigurationOwner.init(allocator, &live), .addend = @as(u32, 1), .acquire_failure = false, .project_failure = project_failure, .body_failure = body_failure, .release_failure = release_failure },
            });
            defer described.deinit();
            var result = fx.run(&env, described);
            defer result.deinit();
            try std.testing.expectEqual(counts.acquired, counts.service_closed);
            try std.testing.expectEqual(counts.acquired, counts.release_calls);
            try std.testing.expectEqual(counts.projected, counts.environment_closed);
            if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
            if (result == .finalization_failed and result.finalization_failed.first == .release_defect and result.finalization_failed.first.release_defect.kind == .out_of_memory) return error.OutOfMemory;
            if (result == .finalization_failed and result.finalization_failed.primary == .defected and result.finalization_failed.primary.defected.kind == .out_of_memory) return error.OutOfMemory;
            if (release_failure) {
                try std.testing.expect(result == .finalization_failed);
                try std.testing.expectEqual(@as(u8, 23), result.finalization_failed.first.release_failure.middleware);
            } else if (project_failure) try std.testing.expectEqual(@as(u8, 21), result.fail.middleware) else if (body_failure) try std.testing.expectEqual(@as(u8, 22), result.fail.domain) else try std.testing.expectEqual(@as(u32, 42), result.success);
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{ false, false, false });
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{ true, false, false });
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{ false, true, false });
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{ false, true, true });
}

test "RPC-03 LIFE-12 destroying the async owner cancels and drains accepted middleware work before releasing configuration environment and service" {
    var live: usize = 0;
    var counts: Counts = .{};
    var gate: Gate = .{ .result = .{ .success = 42 } };
    var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
    defer threads.deinit();
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &counts, .gate = &gate };
    var described = fx.dsl.withRpcMiddleware(&env, .{
        .definition = fx.dsl.declareMetadata(.{ .acquire = Inferred.acquire, .project = Inferred.project, .use = Inferred.pending, .release = Managed.release }),
        .inputs = .{ .owner = try ConfigurationOwner.init(std.testing.allocator, &live), .addend = @as(u32, 1), .acquire_failure = false, .project_failure = false, .body_failure = false, .release_failure = false },
    });
    var pending = fx.runAsync(&env, described);
    described.deinit();
    gate.started.waitUncancelable(env.io);
    try std.testing.expectEqual(@as(usize, 1), live);
    try std.testing.expectEqual(@as(usize, 0), counts.environment_closed);
    try std.testing.expectEqual(@as(usize, 0), counts.service_closed);
    try std.testing.expect(!gate.joined.load(.acquire));
    // No explicit request/join: destruction is the authority under test.
    pending.deinit();
    try std.testing.expect(gate.cancelled.load(.acquire));
    try std.testing.expect(gate.joined.load(.acquire));
    try std.testing.expect(gate.destroyed.load(.acquire));
    try std.testing.expectEqual(@as(usize, 1), counts.release_calls);
    try std.testing.expectEqual(@as(usize, 1), counts.environment_closed);
    try std.testing.expectEqual(@as(usize, 1), counts.service_closed);
    try std.testing.expectEqual(@as(usize, 0), live);
}

const EffectProjector = struct {
    fn project(configuration: anytype, service: *const Service, env: *const Env) fx.Effect(HandlerEnv, u8, Env) {
        const C = @typeInfo(@TypeOf(configuration)).pointer.child;
        const Capture = struct { configuration: *const C, service: *const Service };
        const Unit = fx.Effect(void, u8, Env);
        var seed = if (env.projection_gate == null) Unit.succeed({}) else Unit.fromAsync(struct {
            fn start(current: *const Env) fx.Async(void, u8).Start {
                return current.projection_gate.?.start();
            }
        }.start);
        return seed.flatMapCapturedOwned(env, HandlerEnv, Capture{ .configuration = configuration, .service = service }, struct {
            fn complete(stored: *const Capture, _: void, current: *const Env) fx.Effect(HandlerEnv, u8, Env) {
                return fx.Effect(HandlerEnv, u8, Env).fromOwnedOutcome(current, switch (Inferred.project(stored.configuration, stored.service, current)) {
                    .ok => |value| .{ .success = value },
                    .err => |problem| .{ .fail = problem },
                });
            }
        }.complete);
    }
    fn late(_: anytype, service: *const Service, _: *const Env) fx.Effect(HandlerEnv, u8, Env) {
        return fx.Effect(HandlerEnv, u8, Env).fromCancellableBorrowed(Service, service, struct {
            fn create(stored: *const Service, current: *const Env, token: ?*const fx.Cancellation) fx.Exit(HandlerEnv, u8) {
                const input: u32 = 1;
                const projected = Adapter.project(&input, stored, current);
                @constCast(token.?).request();
                return .{ .success = projected };
            }
        }.create);
    }
};

test "RPC-03 RPC-05 LIFE-12 effectful middleware projection retains service while pending and owner destruction drains before partial rollback" {
    inline for (.{ .success, .fail, .die, .interrupted, .finalization_failed, .destroy }) |mode| {
        var live: usize = 0;
        var counts: Counts = .{};
        var body_gate: Gate = .{ .result = .{ .success = 42 } };
        var projected: ReleaseGate = .{ .result = switch (mode) {
            .success, .destroy => .{ .success = {} },
            .fail => .{ .fail = 25 },
            .die => .{ .die = fx.Defect.init("projection") },
            .interrupted => .interrupted,
            .finalization_failed => .{ .finalization_failed = .{ .primary = .{ .failed = 25 }, .first = .{ .release_failure = 26 } } },
            else => unreachable,
        } };
        var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
        defer threads.deinit();
        const env: Env = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &counts, .gate = &body_gate, .projection_gate = &projected };
        var described = fx.dsl.withRpcMiddleware(&env, .{
            .definition = fx.dsl.declareMetadata(.{ .acquire = Inferred.acquire, .project = EffectProjector.project, .use = Inferred.use, .release = Managed.release }),
            .inputs = .{ .owner = try ConfigurationOwner.init(std.testing.allocator, &live), .addend = @as(u32, 1), .acquire_failure = false, .project_failure = false, .body_failure = false, .release_failure = false },
        });
        var pending = fx.runAsync(&env, described);
        described.deinit();
        projected.started.waitUncancelable(env.io);
        try std.testing.expectEqual(@as(usize, 1), live);
        try std.testing.expectEqual(@as(usize, 0), counts.service_closed);
        try std.testing.expectEqual(@as(usize, 0), counts.projected);
        if (mode == .destroy) {
            pending.deinit();
            try std.testing.expect(projected.cancelled.load(.acquire));
        } else {
            projected.proceed.set(env.io);
            var result = pending.join();
            pending.deinit();
            defer result.deinit();
            switch (mode) {
                .success => try std.testing.expectEqual(@as(u32, 42), result.success),
                .fail => try std.testing.expectEqual(@as(u8, 25), result.fail.middleware),
                .die => try std.testing.expect(result == .die),
                .interrupted => try std.testing.expect(result == .interrupted),
                .finalization_failed => {
                    try std.testing.expectEqual(@as(u8, 25), result.finalization_failed.primary.failed.middleware);
                    try std.testing.expectEqual(@as(u8, 26), result.finalization_failed.first.release_failure.middleware);
                },
                else => unreachable,
            }
        }
        try std.testing.expect(projected.joined.load(.acquire) and projected.destroyed.load(.acquire));
        try std.testing.expectEqual(@as(usize, 1), counts.release_calls);
        try std.testing.expectEqual(@as(usize, 1), counts.service_closed);
        try std.testing.expectEqual(@as(usize, if (mode == .success) 1 else 0), counts.environment_closed);
        try std.testing.expectEqual(@as(usize, 0), live);
    }
}

test "RPC-05 effectful projection late cancellation disposes its owning environment before service release without invoking the handler" {
    var live: usize = 0;
    var counts: Counts = .{};
    var gate: Gate = .{ .result = .{ .success = 42 } };
    var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
    defer threads.deinit();
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &counts, .gate = &gate };
    var described = fx.dsl.withRpcMiddleware(&env, .{
        .definition = fx.dsl.declareMetadata(.{ .acquire = Inferred.acquire, .project = EffectProjector.late, .use = Inferred.use, .release = Managed.release }),
        .inputs = .{ .owner = try ConfigurationOwner.init(std.testing.allocator, &live), .addend = @as(u32, 1), .acquire_failure = false, .project_failure = false, .body_failure = false, .release_failure = false },
    });
    var pending = fx.runAsync(&env, described);
    described.deinit();
    var result = pending.join();
    pending.deinit();
    defer result.deinit();
    try std.testing.expect(result == .interrupted);
    try std.testing.expectEqual(@as(usize, 0), counts.handler_calls);
    try std.testing.expectEqual(@as(usize, 1), counts.environment_closed);
    try std.testing.expectEqual(@as(usize, 1), counts.release_calls);
    try std.testing.expectEqual(@as(usize, 1), counts.service_closed);
    try std.testing.expectEqual(@as(usize, 0), live);
}

test "RPC-05 LIFE-07 effectful middleware projection unwinds every allocation failure before or after environment transfer" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator, project_failure: bool) !void {
            var live: usize = 0;
            defer std.debug.assert(live == 0);
            var counts: Counts = .{};
            var gate: Gate = .{ .result = .{ .success = 42 } };
            var threads: Threads = .{ .allocator = allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
            defer threads.deinit();
            const env: Env = .{ .allocator = allocator, .async_engine = threads.executor(), .counts = &counts, .gate = &gate };
            var described = fx.dsl.withRpcMiddleware(&env, .{
                .definition = fx.dsl.declareMetadata(.{ .acquire = Inferred.acquire, .project = EffectProjector.project, .use = Inferred.use, .release = Managed.release }),
                .inputs = .{ .owner = try ConfigurationOwner.init(allocator, &live), .addend = @as(u32, 1), .acquire_failure = false, .project_failure = project_failure, .body_failure = false, .release_failure = false },
            });
            defer described.deinit();
            var result = fx.run(&env, described);
            defer result.deinit();
            try std.testing.expectEqual(counts.acquired, counts.release_calls);
            try std.testing.expectEqual(counts.acquired, counts.service_closed);
            try std.testing.expectEqual(counts.projected, counts.environment_closed);
            if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
            if (result == .finalization_failed and result.finalization_failed.primary == .defected and result.finalization_failed.primary.defected.kind == .out_of_memory) return error.OutOfMemory;
            if (result == .finalization_failed and result.finalization_failed.first == .release_defect and result.finalization_failed.first.release_defect.kind == .out_of_memory) return error.OutOfMemory;
            if (project_failure) try std.testing.expectEqual(@as(u8, 21), result.fail.middleware) else try std.testing.expectEqual(@as(u32, 42), result.success);
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{false});
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{true});
}
