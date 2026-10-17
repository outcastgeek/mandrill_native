const std = @import("std");
const fx = @import("effect");
const program = fx.dsl.program;
const E = enum { expected };
const Env = struct { io: std.Io = std.testing.io, allocator: std.mem.Allocator, input: u32, releases: *usize };
const A = struct {
    fn first(env: *const Env) fx.Effect(u32, E, Env) {
        return fx.Effect(u32, E, Env).succeed(env.input);
    }
    fn second(value: *const u32, _: *const Env) fx.Effect(u64, E, Env) {
        return fx.Effect(u64, E, Env).succeed(value.* + 1);
    }
    fn last(a: *const u32, b: *const u64, _: *const Env) fx.Effect(u64, E, Env) {
        return fx.Effect(u64, E, Env).succeed(a.* + b.*);
    }
};
const nodes = .{ program.binding("a", .{}, A.first), program.binding("b", .{"a"}, A.second), program.binding("answer", .{ "a", "b" }, A.last) };

test "PROG-01 PROG-05 PROG-08 INF-11 COMP-01 named bindings generate stable per-run slots" {
    var releases: usize = 0;
    var env: Env = .{ .allocator = std.testing.allocator, .input = 7, .releases = &releases };
    const described = comptime program.build(E, Env, nodes);
    comptime {
        if (@TypeOf(described) != fx.Effect(u64, E, Env)) @compileError("program must be exact Effect");
    }
    try std.testing.expectEqual(@as(u64, 15), fx.run(&env, described).success);
    env.input = 11;
    try std.testing.expectEqual(@as(u64, 23), fx.run(&env, described).success);
}

test "LIFE-07 generated frame allocation failures are lazy defects with no leak" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            var releases: usize = 0;
            const env: Env = .{ .allocator = allocator, .input = 7, .releases = &releases };
            var result = fx.run(&env, comptime program.build(E, Env, nodes));
            defer result.deinit();
            if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expectEqual(@as(u64, 15), result.success);
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

const Resource = struct { allocator: std.mem.Allocator, value: u32, releases: *usize };
const ResourceSteps = struct {
    fn acquire(value: *const u32, _: *const Env) fx.Effect(*Resource, E, Env) {
        return fx.Dsl(E, Env).outcomeBorrowed(value, struct {
            fn allocate(input: *const u32, current: *const Env) fx.Exit(*Resource, E) {
                const resource = current.allocator.create(Resource) catch return .{ .die = fx.Defect.out_of_memory };
                resource.* = .{ .allocator = current.allocator, .value = input.* * 2, .releases = current.releases };
                return .{ .success = resource };
            }
        }.allocate);
    }

    fn release(resource: *Resource) fx.Effect(void, E, Env) {
        return fx.Effect(void, E, Env).fromBorrowed(Resource, resource, struct {
            fn finish(input: *const Resource, _: *const Env) fx.Exit(void, E) {
                input.releases.* += 1;
                input.allocator.destroy(@constCast(input));
                return .{ .success = {} };
            }
        }.finish);
    }
    fn predicate(_: *const u32, _: *const u64, _: *const *Resource, env: *const Env) bool {
        return env.input % 2 == 1;
    }
    fn yes(a: *const u32, b: *const u64, resource: *const *Resource, _: *const Env) fx.Effect(u64, E, Env) {
        return fx.Effect(u64, E, Env).succeed(a.* + b.* + resource.*.value);
    }
    fn no(_: *const u32, _: *const u64, _: *const *Resource, _: *const Env) fx.Effect(u64, E, Env) {
        return fx.Effect(u64, E, Env).fail(.expected);
    }
};

test "PROG-01 PROG-02 RES-01 branches retain prior bindings and lexical resource through selected work" {
    const described = comptime program.build(E, Env, .{
        program.binding("a", .{}, A.first),                                                 program.binding("b", .{"a"}, A.second),
        program.resource("resource", .{"a"}, ResourceSteps.acquire, ResourceSteps.release), program.branch("answer", .{ "a", "b", "resource" }, ResourceSteps.predicate, ResourceSteps.yes, ResourceSteps.no),
    });
    var releases: usize = 0;
    var env: Env = .{ .allocator = std.testing.allocator, .input = 7, .releases = &releases };
    try std.testing.expectEqual(@as(usize, 0), releases);
    try std.testing.expectEqual(@as(u64, 29), fx.run(&env, described).success);
    try std.testing.expectEqual(@as(usize, 1), releases);
    env.input = 8;
    try std.testing.expect(fx.run(&env, described) == .fail);
    try std.testing.expectEqual(@as(usize, 2), releases);
}

test "PROG-03 LIFE-10 loop state and iterations use the existing trampoline" {
    const L = struct {
        fn initialize(_: *const Env) u32 {
            return 0;
        }
        fn condition(value: *const u32, _: *const Env) bool {
            return value.* < 10000;
        }
        fn body(view: anytype, _: *const Env) fx.Effect(program.LoopControl(u32), E, Env) {
            view.state().* += 1;
            return fx.Effect(program.LoopControl(u32), E, Env).succeed(.again);
        }
        fn finish(view: anytype, _: *const Env) u32 {
            return view.state().*;
        }
    };
    const described = comptime program.build(E, Env, .{program.loop("loop", .{}, L.initialize, L.condition, L.body, L.finish)});
    var releases: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .input = 0, .releases = &releases };
    try std.testing.expectEqual(@as(u32, 10000), fx.run(&env, described).success);
}

const CancelEnv = struct { allocator: std.mem.Allocator, cancellation: *fx.Cancellation, called: *usize, released: *usize };
const NativeOwned = struct {
    pub const effect_owned_payload = true;
    released: *usize,
    pub fn deinit(self: *@This()) void {
        self.released.* += 1;
    }
};
test "PROG-03 LIFE-01 native binding cancellation skips later work and releases initialized owning slots" {
    const N = struct {
        fn first(env: *const CancelEnv) NativeOwned {
            env.cancellation.request();
            return .{ .released = env.released };
        }
        fn later(_: *const NativeOwned, env: *const CancelEnv) u32 {
            env.called.* += 1;
            return 1;
        }
    };
    const described = comptime program.build(E, CancelEnv, .{ program.native("first", .{}, N.first), program.native("later", .{"first"}, N.later) });
    var cancellation: fx.Cancellation = .{};
    var called: usize = 0;
    var released: usize = 0;
    const env: CancelEnv = .{ .allocator = std.testing.allocator, .cancellation = &cancellation, .called = &called, .released = &released };
    try std.testing.expect(fx.run(&env, described) == .interrupted);
    try std.testing.expectEqual(@as(usize, 0), called);
    try std.testing.expectEqual(@as(usize, 1), released);
}

test "PROG-04 LIFE-06 early return transfers an earlier binding and skips remaining callbacks" {
    const EarlySteps = struct {
        fn first(_: *const Env) u32 {
            return 41;
        }
        fn predicate(_: *const u32, env: *const Env) bool {
            return env.input == 1;
        }
        fn last(_: *const u32, env: *const Env) u32 {
            env.releases.* += 1;
            return 99;
        }
    };
    const described = comptime program.build(E, Env, .{ program.native("first", .{}, EarlySteps.first), program.returnWhen("return", .{"first"}, EarlySteps.predicate, "first"), program.native("last", .{"first"}, EarlySteps.last) });
    var calls: usize = 0;
    var env: Env = .{ .allocator = std.testing.allocator, .input = 1, .releases = &calls };
    try std.testing.expectEqual(@as(u32, 41), fx.run(&env, described).success);
    try std.testing.expectEqual(@as(usize, 0), calls);
    env.input = 0;
    try std.testing.expectEqual(@as(u32, 99), fx.run(&env, described).success);
    try std.testing.expectEqual(@as(usize, 1), calls);
}

const InputOwner = struct {
    pub const effect_owned_payload = true;
    disposed: *usize,
    pub fn deinit(self: *@This()) void {
        self.disposed.* += 1;
    }
};
const CapturedSteps = struct {
    fn first(input: anytype, _: *const Env) fx.Effect(u32, E, Env) {
        return fx.Effect(u32, E, Env).succeed(input.value);
    }
};
const captured_nodes = .{
    program.binding("a", .{"inputs"}, CapturedSteps.first),
    program.binding("b", .{"a"}, A.second),
    program.resource("resource", .{"a"}, ResourceSteps.acquire, ResourceSteps.release),
    program.branch("answer", .{ "a", "b", "resource" }, ResourceSteps.predicate, ResourceSteps.yes, ResourceSteps.no),
};
fn checkCaptured(allocator: std.mem.Allocator) !void {
    var releases: usize = 0;
    var disposed: usize = 0;
    const env: Env = .{ .allocator = allocator, .input = 1, .releases = &releases };
    var described = program.captured(&env, .{ .value = @as(u32, 7), .owner = InputOwner{ .disposed = &disposed } }, captured_nodes);
    comptime if (@TypeOf(described) != fx.Effect(u64, E, Env)) @compileError("captured program must infer exact Effect");
    try std.testing.expectEqual(@as(usize, 0), releases);
    try std.testing.expect(disposed <= 1); // Construction OOM consumes the input.
    var mapped = described.mapOwned(&env, struct {
        fn double(value: u64) u64 {
            return value * 2;
        }
    }.double);
    var result = fx.run(&env, mapped);
    mapped.deinit();
    defer result.deinit();
    try std.testing.expectEqual(@as(usize, 1), disposed);
    try std.testing.expect(releases <= 1);
    if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(u64, 58), result.success);
    try std.testing.expectEqual(@as(usize, 1), releases);
}
test "CAP-02 PROG-01 PROG-02 COMP-05 captured anonymous inputs compose through earlier bindings and resource branches including every OOM" {
    try checkCaptured(std.testing.allocator);
    try std.testing.checkAllAllocationFailures(std.testing.allocator, checkCaptured, .{});
}
test "CAP-02 PROG-05 captured program reuses move-only input storage with independent per-run slots" {
    var releases: usize = 0;
    var disposed: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .input = 1, .releases = &releases };
    var described = program.captured(&env, .{ .value = @as(u32, 7), .owner = InputOwner{ .disposed = &disposed } }, captured_nodes);
    for (0..2) |_| {
        var result = fx.run(&env, described);
        defer result.deinit();
        try std.testing.expectEqual(@as(u64, 29), result.success);
        try std.testing.expectEqual(@as(usize, 0), disposed);
    }
    described.deinit();
    try std.testing.expectEqual(@as(usize, 2), releases);
    try std.testing.expectEqual(@as(usize, 1), disposed);
}

const gates = @import("support/dsl_gate.zig");
const Pending = struct {
    const R = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, async_engine: fx.Executor, gate: *gates.Gate(u32, E), disposed: *usize };
    const Number = fx.Effect(u32, E, R);
    fn wait(input: anytype, env: *const R) Number {
        std.debug.assert(env.disposed.* == 0 and std.mem.eql(u8, input.bytes.items, "stable"));
        return Number.fromAsync(struct {
            fn start(current: *const R) fx.Async(u32, E).Start {
                return current.gate.start();
            }
        }.start);
    }
    fn continued(input: anytype, value: *const u32, _: *const R) Number {
        std.debug.assert(std.mem.eql(u8, input.bytes.items, "stable"));
        return Number.succeed(value.* + @as(u32, @intCast(input.bytes.items.len)));
    }
};
test "CAP-01 CAP-02 LIFE-05 PROG-05 captured program inputs remain live through pending resume and cancellation" {
    for ([_]bool{ false, true }) |cancel| {
        var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = true }, .maximum_owned = 4 };
        defer engine.deinit();
        var gate: gates.Gate(u32, E) = .{ .result = .{ .success = 10 } };
        var disposed: usize = 0;
        const env: Pending.R = .{ .allocator = std.testing.allocator, .async_engine = engine.executor(), .gate = &gate, .disposed = &disposed };
        var described = program.captured(&env, .{ .bytes = fx.OwnedSlice(u8){ .items = try env.allocator.dupe(u8, "stable"), .allocator = env.allocator }, .owner = InputOwner{ .disposed = &disposed } }, .{
            program.binding("pending", .{"inputs"}, Pending.wait), program.binding("result", .{ "inputs", "pending" }, Pending.continued),
        });
        var pending = fx.runAsync(&env, described);
        gate.started.waitUncancelable(env.io);
        try std.testing.expectEqual(@as(usize, 0), disposed);
        if (cancel) pending.request() else gate.proceed.set(env.io);
        var result = pending.join();
        defer result.deinit();
        if (cancel) try std.testing.expect(result == .interrupted) else try std.testing.expectEqual(@as(u32, 16), result.success);
        pending.deinit();
        try std.testing.expectEqual(@as(usize, 0), disposed);
        described.deinit();
        try std.testing.expect(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
        try std.testing.expectEqual(@as(usize, 1), disposed);
    }
}

test "CAP-02 PERF-03 captured native bindings allocate the same storage as equivalent direct owning composition" {
    const Native = struct {
        fn next(input: anytype, _: *const Env) u32 {
            return input.value + 1;
        }
        fn direct(input: anytype, _: *const Env) fx.Effect(u32, void, Env) {
            return fx.Effect(u32, void, Env).succeed(input.value + 1);
        }
    };
    var counting = std.testing.FailingAllocator.init(std.testing.allocator, .{});
    var releases: usize = 0;
    const env: Env = .{ .allocator = counting.allocator(), .input = 0, .releases = &releases };
    const before = counting.alloc_index;
    var described = program.captured(&env, .{ .value = @as(u32, 7) }, .{program.native("result", .{"inputs"}, Native.next)});
    const after_construct = counting.alloc_index;
    var result = fx.run(&env, described);
    const generated_allocations = counting.alloc_index - before;
    const generated_runtime = counting.alloc_index - after_construct;
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(u32, 8), result.success);
    result.deinit();
    described.deinit();
    const direct_before = counting.alloc_index;
    var direct = fx.dsl.owned(&env, .{ .value = @as(u32, 7) }, Native.direct);
    const direct_constructed = counting.alloc_index;
    var direct_result = fx.run(&env, direct);
    try std.testing.expectEqual(generated_allocations, counting.alloc_index - direct_before);
    try std.testing.expectEqual(generated_runtime, counting.alloc_index - direct_constructed);
    try std.testing.expectEqual(@as(u32, 8), direct_result.success);
    direct_result.deinit();
    direct.deinit();
    try std.testing.expectEqual(counting.allocated_bytes, counting.freed_bytes);
}
