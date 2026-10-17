const std = @import("std");
const fx = @import("effect");
const gates = @import("support/dsl_gate.zig");
const Domain = enum { missing };
const Provider = enum { offline };
const Definition = struct {
    pub const Input = u32;
    pub const Success = u64;
    pub const DomainError = Domain;
};
const Env = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, async_engine: fx.Executor = undefined, state: *State };
const Resolver = fx.RequestResolver(Definition, Env, Provider);
const Out = Resolver.BatchEffect;
const Unit = fx.Effect(void, Resolver.Failure, Env);
const Mode = enum { normal, empty, invalid_key, mismatch, provider_failure, pending };
const State = struct { live: usize = 0, calls: usize = 0, mode: Mode = .normal, gate: ?*gates.Gate(void, Resolver.Failure) = null, legacy_resolver: ?Resolver = null, legacy_jobs: []const Resolver.Job = &.{} };
const Owner = struct {
    pub const effect_owned_payload = true;
    state: *State,
    pub fn deinit(self: *@This()) void {
        if (self.state.gate) |gate| std.debug.assert(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
        self.state.live -= 1;
    }
};
const Inputs = struct { owner: Owner, values: fx.OwnedSlice(u32), even: fx.OwnedSlice(u8), odd: fx.OwnedSlice(u8), jobs: fx.OwnedSlice(Resolver.Job) };
fn inputs(env: *const Env) error{OutOfMemory}!Inputs {
    const values = try env.allocator.alloc(u32, 3);
    errdefer env.allocator.free(values);
    @memcpy(values, &[_]u32{ 2, 1, 4 });
    var even = fx.PrimaryKey.encodeDirect(env, "even-resolver", &.{});
    if (even != .success) return error.OutOfMemory;
    errdefer even.deinit();
    var odd = fx.PrimaryKey.encodeDirect(env, "odd-resolver", &.{});
    if (odd != .success) return error.OutOfMemory;
    errdefer odd.deinit();
    const jobs = try env.allocator.alloc(Resolver.Job, values.len);
    for (jobs, values, 0..) |*job, *value, i| job.* = .{ .key = if (env.state.mode == .invalid_key and i == 0) "malformed" else if (value.* % 2 == 0) even.success.items else odd.success.items, .input = value };
    env.state.live += 1;
    return .{ .owner = .{ .state = env.state }, .values = .{ .items = values, .allocator = env.allocator }, .even = even.success, .odd = odd.success, .jobs = .{ .items = jobs, .allocator = env.allocator } };
}
const Roles = struct {
    fn jobs(capture: anytype, _: *const Env) []Resolver.Job {
        return if (capture.owner.state.mode == .empty) capture.jobs.items[0..0] else capture.jobs.items;
    }
    fn group(_: *const Inputs, job: *const Resolver.Job, _: *const Env) []const u8 {
        return job.key;
    }
    const Selected = struct { inputs: *const Inputs, jobs: []const Resolver.Job };
    fn result(selected: *const Selected, env: *const Env) fx.Exit(Resolver.Results, Resolver.Failure) {
        if (selected.inputs.owner.state.live != 1) return .{ .die = fx.Defect.init("resolver input owner ended before provider") };
        const extra: usize = if (env.state.mode == .mismatch) 1 else 0;
        const values = env.allocator.alloc(Resolver.ItemExit, selected.jobs.len + extra) catch return .{ .die = fx.Defect.out_of_memory };
        for (selected.jobs, values[0..selected.jobs.len]) |job, *slot| slot.* = if (job.input.* % 2 == 0) .{ .success = @as(u64, job.input.*) * 10 } else .{ .fail = .missing };
        if (extra != 0) values[values.len - 1] = .{ .success = 0 };
        return .{ .success = .{ .items = values, .allocator = env.allocator } };
    }
    fn continueResolve(selected: *const Selected, _: void, _: *const Env) Out {
        return Out.fromBorrowed(Selected, selected, result);
    }
    fn resolve(capture: *const Inputs, selected: []const Resolver.Job, env: *const Env) Out {
        env.state.calls += 1;
        if (env.state.mode == .provider_failure) return Out.fail(.{ .provider = .offline });
        if (env.state.mode == .pending) {
            var wait = Unit.fromAsync(struct {
                fn start(current: *const Env) fx.Async(void, Resolver.Failure).Start {
                    return current.state.gate.?.start();
                }
            }.start);
            return wait.flatMapCapturedOwned(env, Resolver.Results, Selected{ .inputs = capture, .jobs = selected }, continueResolve);
        }
        return Out.fromOwned(env, Selected{ .inputs = capture, .jobs = selected }, result);
    }
};
fn make(env: *const Env, comptime grouped: bool) Out {
    const capture = inputs(env) catch return Out.die(fx.Defect.out_of_memory);
    return if (grouped) fx.dsl.resolveGrouped(Resolver, env, capture, .{ .jobs = Roles.jobs, .resolve = Roles.resolve, .group = Roles.group }) else fx.dsl.resolveBatch(Resolver, env, capture, .{ .resolve = Roles.resolve, .jobs = Roles.jobs });
}
fn program(comptime grouped: bool) Out {
    return Out.deferEffect(struct {
        fn create(env: *const Env) Out {
            return make(env, grouped);
        }
    }.create);
}
fn expectOrdered(value: Resolver.Results) !void {
    try std.testing.expectEqual(@as(usize, 3), value.items.len);
    try std.testing.expect(value.items[0] == .success and value.items[0].success == 20);
    try std.testing.expect(value.items[1] == .fail and value.items[1].fail == .missing);
    try std.testing.expect(value.items[2] == .success and value.items[2].success == 40);
}

test "FAC-05 INF-10 inferred resolver roles preserve ordered per-request domain outcomes in batch and grouped modes" {
    inline for (.{ false, true }) |grouped| {
        var state: State = .{};
        const env: Env = .{ .allocator = std.testing.allocator, .state = &state };
        var described = make(&env, grouped);
        comptime if (@TypeOf(described) != fx.Effect(Resolver.Results, Resolver.Failure, Env)) @compileError("resolver must return the exact Effect");
        try std.testing.expectEqual(@as(usize, 0), state.calls);
        var result = fx.run(&env, described);
        described.deinit();
        defer result.deinit();
        try std.testing.expect(result == .success);
        try expectOrdered(result.success);
        try std.testing.expectEqual(@as(usize, if (grouped) 2 else 1), state.calls);
        try std.testing.expectEqual(@as(usize, 0), state.live);
    }
}

test "FAC-05 resolver invalid keys empty batches result counts and provider failures retain distinct channels" {
    inline for (.{ false, true }) |grouped| {
        for ([_]Mode{ .empty, .invalid_key, .mismatch, .provider_failure }) |mode| {
            var state: State = .{ .mode = mode };
            const env: Env = .{ .allocator = std.testing.allocator, .state = &state };
            var result = fx.run(&env, comptime program(grouped));
            defer result.deinit();
            try std.testing.expect(result == .fail);
            if (mode == .provider_failure) {
                try std.testing.expect(result.fail == .provider and result.fail.provider == .offline);
            } else {
                try std.testing.expect(result.fail == .operation);
                const expected: std.meta.Tag(@TypeOf(result.fail.operation)) = switch (mode) {
                    .empty => .empty_batch,
                    .invalid_key => .invalid_key,
                    .mismatch => .response_count_mismatch,
                    else => unreachable,
                };
                try std.testing.expectEqual(expected, std.meta.activeTag(result.fail.operation));
            }
            try std.testing.expectEqual(@as(usize, if (mode == .empty or mode == .invalid_key) 0 else 1), state.calls);
            try std.testing.expectEqual(@as(usize, 0), state.live);
        }
    }
}

test "FAC-05 LIFE-07 all resolver allocation failures release input owners and partial grouped results" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            inline for (.{ false, true }) |grouped| {
                var state: State = .{};
                const env: Env = .{ .allocator = allocator, .state = &state };
                var result = fx.run(&env, comptime program(grouped));
                defer result.deinit();
                try std.testing.expectEqual(@as(usize, 0), state.live);
                if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
                try std.testing.expect(result == .success);
                try expectOrdered(result.success);
            }
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

test "FAC-05 LIFE-11 pending resolver results outlive grouped frames and input-owner teardown" {
    inline for (.{ false, true }) |grouped| inline for (.{ false, true }) |cancel| {
        var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = false, .blocking = false }, .maximum_owned = 4 };
        defer engine.deinit();
        var gate: gates.Gate(void, Resolver.Failure) = .{ .result = .{ .success = {} } };
        var state: State = .{ .mode = .pending, .gate = &gate };
        const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor(), .state = &state };
        var pending = fx.runAsync(&env, comptime program(grouped));
        gate.started.waitUncancelable(env.io);
        try std.testing.expectEqual(@as(usize, 1), state.live);
        if (cancel) pending.request() else gate.proceed.set(env.io);
        var result = pending.join();
        pending.deinit();
        defer result.deinit();
        try std.testing.expectEqual(@as(usize, 0), state.live);
        if (cancel) try std.testing.expect(result == .interrupted) else {
            try std.testing.expect(result == .success);
            try expectOrdered(result.success);
        }
        try std.testing.expect(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
    };
}

test "FAC-05 legacy batch and grouped entry points share the checked resolver implementation" {
    const Legacy = struct {
        fn resolver(env: *const Env) *const Resolver {
            return &env.state.legacy_resolver.?;
        }
        fn jobs(env: *const Env) []const Resolver.Job {
            return env.state.legacy_jobs;
        }
        fn group(job: *const Resolver.Job) []const u8 {
            return job.key;
        }
        fn resolve(data: ?*anyopaque, env: *const Env, selected: []const Resolver.Job) Out {
            const capture: *const Inputs = @ptrCast(@alignCast(data.?));
            return Roles.resolve(capture, selected, env);
        }
    };
    inline for (.{ false, true }) |grouped| {
        var state: State = .{};
        const env: Env = .{ .allocator = std.testing.allocator, .state = &state };
        var capture = try inputs(&env);
        state.legacy_resolver = Resolver.fromProvider(&capture, &.{ .resolve_batch = Legacy.resolve });
        state.legacy_jobs = capture.jobs.items;
        const described = comptime if (grouped) Resolver.resolveGrouped(Legacy.resolver, Legacy.jobs, Legacy.group) else Resolver.resolveBatch(Legacy.resolver, Legacy.jobs);
        var result = fx.run(&env, described);
        defer result.deinit();
        try std.testing.expect(result == .success);
        try expectOrdered(result.success);
        fx.dsl.ownership.discard(&capture);
        try std.testing.expectEqual(@as(usize, 0), state.live);
    }
}
