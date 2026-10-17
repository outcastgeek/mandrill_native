const std = @import("std");
const fx = @import("effect");
const p = fx.dsl.program;
const E = enum { expected };
const Counts = struct {
    live: usize = 0,
    created: usize = 0,
    yes: usize = 0,
    no: usize = 0,
    later: usize = 0,
    active: [2]bool = .{ false, false },
    released: [2]u8 = .{ 0, 0 },
    releases: usize = 0,
};
const Env = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, counts: *Counts, choose: bool, input: u32 = 41 };
const Token = struct {
    pub const effect_owned_payload = true;
    counts: *Counts,
    pub fn deinit(self: *@This()) void {
        self.counts.live -= 1;
    }
};
const Value = struct { token: Token, number: u32 };
const Choice = union(enum) { left: Value, right: u32 };
fn make(env: *const Env, number: u32) Value {
    env.counts.live += 1;
    env.counts.created += 1;
    return .{ .token = .{ .counts = env.counts }, .number = number };
}
fn oom(value: anytype) bool {
    if (value == .die) return value.die.kind == .out_of_memory;
    if (value == .finalization_failed) {
        const failed = value.finalization_failed;
        if (failed.primary == .defected and failed.primary.defected.kind == .out_of_memory) return true;
        if (failed.first == .release_defect and failed.first.release_defect.kind == .out_of_memory) return true;
        var next = failed.rest;
        while (next) |node| : (next = node.next) {
            if (node.failure == .release_defect and node.failure.release_defect.kind == .out_of_memory) return true;
        }
    }
    return false;
}
const Branches = struct {
    fn first(_: *const Env) fx.Effect(Value, E, Env) {
        return fx.Effect(Value, E, Env).fromOutcome(struct {
            fn run(current: *const Env) fx.Exit(Value, E) {
                return .{ .success = make(current, current.input) };
            }
        }.run);
    }
    fn predicate(previous: *const Value, env: *const Env) bool {
        std.debug.assert(previous.number == env.input and env.counts.live == 1);
        return env.choose;
    }
    fn yes(previous: *const Value, env: *const Env) fx.Effect(Value, E, Env) {
        env.counts.yes += 1;
        return fx.Effect(Value, E, Env).succeed(make(env, previous.number + 1));
    }
    fn no(previous: *const Value, env: *const Env) fx.Effect(Value, E, Env) {
        env.counts.no += 1;
        return fx.Effect(Value, E, Env).succeed(make(env, previous.number + 2));
    }
    fn taggedYes(previous: *const Value, env: *const Env) fx.Effect(Choice, E, Env) {
        env.counts.yes += 1;
        return fx.Effect(Choice, E, Env).succeed(.{ .left = make(env, previous.number + 3) });
    }
    fn taggedNo(previous: *const Value, env: *const Env) fx.Effect(Choice, E, Env) {
        env.counts.no += 1;
        return fx.Effect(Choice, E, Env).succeed(.{ .right = previous.number + 4 });
    }
    const owning = p.build(E, Env, .{ p.binding("previous", .{}, first), p.branch("answer", .{"previous"}, predicate, yes, no) });
    const tagged = p.build(E, Env, .{ p.binding("previous", .{}, first), p.branch("answer", .{"previous"}, predicate, taggedYes, taggedNo) });
    fn check(allocator: std.mem.Allocator) !void {
        inline for (.{ false, true }) |selected| {
            var counts: Counts = .{};
            defer std.debug.assert(counts.live == 0);
            const env: Env = .{ .allocator = allocator, .counts = &counts, .choose = selected };
            var result = fx.run(&env, owning);
            defer result.deinit();
            if (oom(result)) return error.OutOfMemory;
            try std.testing.expectEqual(@as(u32, if (selected) 42 else 43), result.success.value.number);
            try std.testing.expectEqual(@as(usize, 1), counts.live);
            try std.testing.expectEqual(@as(usize, 2), counts.created);
            try std.testing.expectEqual(@as(usize, @intFromBool(selected)), counts.yes);
            try std.testing.expectEqual(@as(usize, @intFromBool(!selected)), counts.no);
        }
        inline for (.{ false, true }) |selected| {
            var counts: Counts = .{};
            defer std.debug.assert(counts.live == 0);
            const env: Env = .{ .allocator = allocator, .counts = &counts, .choose = selected };
            var result = fx.run(&env, tagged);
            defer result.deinit();
            if (oom(result)) return error.OutOfMemory;
            if (selected) {
                try std.testing.expectEqual(@as(u32, 44), result.success.value.left.number);
            } else try std.testing.expectEqual(@as(u32, 45), result.success.value.right);
            try std.testing.expectEqual(@as(usize, @intFromBool(selected)), counts.live);
            try std.testing.expectEqual(@as(usize, @intFromBool(selected)), counts.yes);
            try std.testing.expectEqual(@as(usize, @intFromBool(!selected)), counts.no);
        }
    }
};
test "PROG-02 CAP-08 selected owning branches and explicit tagged joins transfer one result and release abandoned slots without business Clone" {
    try Branches.check(std.testing.allocator);
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Branches.check, .{});
}

const Lease = struct { allocator: std.mem.Allocator, counts: *Counts, id: u8 };
const Regions = struct {
    fn acquire(comptime id: u8) fx.Effect(*Lease, E, Env) {
        return fx.Effect(*Lease, E, Env).fromOutcome(struct {
            fn run(env: *const Env) fx.Exit(*Lease, E) {
                const lease = env.allocator.create(Lease) catch return .{ .die = fx.Defect.out_of_memory };
                std.debug.assert(!env.counts.active[id - 1]);
                if (id == 2) std.debug.assert(env.counts.active[0]);
                lease.* = .{ .allocator = env.allocator, .counts = env.counts, .id = id };
                env.counts.active[id - 1] = true;
                return .{ .success = lease };
            }
        }.run);
    }
    fn outer(_: *const Env) fx.Effect(*Lease, E, Env) {
        return acquire(1);
    }
    fn inner(_: *const Env) fx.Effect(*Lease, E, Env) {
        return acquire(2);
    }
    fn release(lease: *Lease) fx.Effect(void, E, Env) {
        return fx.Effect(void, E, Env).fromBorrowed(Lease, lease, struct {
            fn run(current: *const Lease, _: *const Env) fx.Exit(void, E) {
                const counts = current.counts;
                std.debug.assert(counts.active[current.id - 1]);
                if (current.id == 2) std.debug.assert(counts.active[0]);
                counts.active[current.id - 1] = false;
                counts.released[counts.releases] = current.id;
                counts.releases += 1;
                current.allocator.destroy(@constCast(current));
                return .{ .success = {} };
            }
        }.run);
    }
    fn value(_: *const Env) u32 {
        return 41;
    }
    fn early(_: *const u32, env: *const Env) bool {
        return env.choose;
    }
    fn fallback(_: *const u32, env: *const Env) u32 {
        env.counts.later += 1;
        return 99;
    }
    fn nested(_: *const Env) fx.Effect(u32, E, Env) {
        return inner_plan;
    }
    fn after(lease: *const *Lease, answer: *const u32, env: *const Env) fx.Effect(u32, E, Env) {
        std.debug.assert(lease.*.id == 1 and env.counts.active[0] and !env.counts.active[1]);
        std.debug.assert(env.counts.releases == 1 and env.counts.released[0] == 2);
        return fx.Effect(u32, E, Env).succeed(answer.*);
    }
    const inner_plan = p.build(E, Env, .{ p.resource("inner", .{}, inner, release), p.native("value", .{}, value), p.returnWhen("exit", .{"value"}, early, "value"), p.native("fallback", .{"value"}, fallback) });
    const outer_plan = p.build(E, Env, .{ p.resource("outer", .{}, outer, release), p.binding("nested", .{}, nested), p.binding("answer", .{ "outer", "nested" }, after) });
    fn check(allocator: std.mem.Allocator) !void {
        inline for (.{ false, true }) |selected| {
            var counts: Counts = .{};
            defer std.debug.assert(!counts.active[0] and !counts.active[1]);
            const env: Env = .{ .allocator = allocator, .counts = &counts, .choose = selected };
            var result = fx.run(&env, outer_plan);
            defer result.deinit();
            if (oom(result)) return error.OutOfMemory;
            try std.testing.expectEqual(@as(u32, if (selected) 41 else 99), result.success);
            try std.testing.expectEqualSlices(u8, &.{ 2, 1 }, &counts.released);
            try std.testing.expectEqual(@as(usize, 2), counts.releases);
            try std.testing.expectEqual(@as(usize, @intFromBool(!selected)), counts.later);
        }
    }
};
test "PROG-04 RES-01 inner early return drains its lexical resource before outer work resumes and preserves the outer lease through final cleanup" {
    try Regions.check(std.testing.allocator);
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Regions.check, .{});
}

const Gate = @import("support/dsl_gate.zig").Gate(void, E);
const Threads = @import("support/thread_executor.zig").ThreadExecutor;
const RunEnv = struct {
    allocator: std.mem.Allocator,
    io: std.Io = std.testing.io,
    async_engine: fx.Executor,
    counts: *Counts,
    input: u32,
    gate: *Gate,
};
const Waiting = struct {
    fn first(_: *const RunEnv) fx.Effect(Value, E, RunEnv) {
        return fx.Effect(Value, E, RunEnv).fromOutcome(struct {
            fn run(env: *const RunEnv) fx.Exit(Value, E) {
                env.counts.live += 1;
                env.counts.created += 1;
                return .{ .success = .{ .token = .{ .counts = env.counts }, .number = env.input } };
            }
        }.run);
    }
    fn wait(previous: *const Value, env: *const RunEnv) fx.Effect(u32, E, RunEnv) {
        var pending = fx.Effect(void, E, RunEnv).fromAsync(struct {
            fn start(current: *const RunEnv) fx.Async(void, E).Start {
                return current.gate.start();
            }
        }.start);
        return pending.flatMapCapturedOwned(env, u32, previous, struct {
            fn afterWait(stored: *const *const Value, _: void, current: *const RunEnv) fx.Effect(u32, E, RunEnv) {
                std.debug.assert(stored.*.number == current.input and current.counts.live == 1);
                return fx.Effect(u32, E, RunEnv).succeed(stored.*.number);
            }
        }.afterWait);
    }
    const plan = p.build(E, RunEnv, .{ p.binding("previous", .{}, first), p.binding("answer", .{"previous"}, wait) });
};
test "PROG-05 PROG-08 LIFE-12 one plan keeps owning prior bindings in distinct suspended frames across environments and independently joins cancellation" {
    var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 2, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
    defer threads.deinit();
    var first_gate: Gate = .{ .result = .{ .success = {} } };
    var second_gate: Gate = .{ .result = .{ .success = {} } };
    var first_counts: Counts = .{};
    var second_counts: Counts = .{};
    const first_env: RunEnv = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &first_counts, .input = 17, .gate = &first_gate };
    const second_env: RunEnv = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &second_counts, .input = 29, .gate = &second_gate };
    var first = fx.runAsync(&first_env, Waiting.plan);
    var first_active = true;
    defer if (first_active) first.deinit();
    first_gate.started.waitUncancelable(first_env.io);
    var second = fx.runAsync(&second_env, Waiting.plan);
    var second_active = true;
    defer if (second_active) second.deinit();
    second_gate.started.waitUncancelable(second_env.io);
    try std.testing.expectEqual(@as(usize, 1), first_counts.live);
    try std.testing.expectEqual(@as(usize, 1), second_counts.live);
    second_gate.proceed.set(second_env.io);
    var result = second.join();
    defer result.deinit();
    second.deinit();
    second_active = false;
    try std.testing.expectEqual(@as(u32, 29), result.success);
    try std.testing.expectEqual(@as(usize, 0), second_counts.live);
    try std.testing.expectEqual(@as(usize, 1), first_counts.live);
    try std.testing.expect(!first_gate.joined.load(.acquire));
    first.deinit();
    first_active = false;
    try std.testing.expect(first_gate.cancelled.load(.acquire) and first_gate.joined.load(.acquire) and first_gate.destroyed.load(.acquire));
    try std.testing.expectEqual(@as(usize, 0), first_counts.live);
    try std.testing.expectEqual(@as(usize, 1), first_counts.created);
    try std.testing.expectEqual(@as(usize, 1), second_counts.created);
}

const CancelEnv = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, cancellation: *fx.Cancellation, counts: *Counts };
const CancelSteps = struct {
    fn yes(env: *const CancelEnv) fx.Effect(Value, E, CancelEnv) {
        env.counts.live += 1;
        env.counts.created += 1;
        env.cancellation.request();
        return fx.Effect(Value, E, CancelEnv).succeed(.{ .token = .{ .counts = env.counts }, .number = 1 });
    }
    fn no(env: *const CancelEnv) fx.Effect(Value, E, CancelEnv) {
        env.counts.no += 1;
        return yes(env);
    }
    fn choose(_: *const CancelEnv) bool {
        return true;
    }
    fn later(_: *const Value, env: *const CancelEnv) fx.Effect(u32, E, CancelEnv) {
        env.counts.later += 1;
        return fx.Effect(u32, E, CancelEnv).succeed(9);
    }
    fn acquire(env: *const CancelEnv) fx.Effect(*Counts, E, CancelEnv) {
        return fx.Effect(*Counts, E, CancelEnv).succeed(env.counts);
    }
    fn release(counts: *Counts) fx.Effect(void, E, CancelEnv) {
        return fx.Effect(void, E, CancelEnv).fromBorrowed(Counts, counts, struct {
            fn close(current: *const Counts, _: *const CancelEnv) fx.Exit(void, E) {
                std.debug.assert(current.live == 0);
                @constCast(current).releases += 1;
                return .{ .success = {} };
            }
        }.close);
    }
    const branch = p.build(E, CancelEnv, .{ p.branch("produced", .{}, choose, yes, no), p.binding("answer", .{"produced"}, later) });
    const binding = p.build(E, CancelEnv, .{ p.binding("produced", .{}, yes), p.binding("answer", .{"produced"}, later) });
    const final_branch = p.build(E, CancelEnv, .{p.branch("answer", .{}, choose, yes, no)});
    const final_binding = p.build(E, CancelEnv, .{p.binding("answer", .{}, yes)});
    const scoped = p.build(E, CancelEnv, .{ p.resource("lease", .{}, acquire, release), p.binding("produced", .{}, yes), p.binding("answer", .{"produced"}, later) });
    const scoped_branch = p.build(E, CancelEnv, .{ p.resource("lease", .{}, acquire, release), p.branch("produced", .{}, choose, yes, no), p.binding("answer", .{"produced"}, later) });
    fn check(allocator: std.mem.Allocator) !void {
        inline for (.{ final_branch, final_binding, branch, binding, scoped, scoped_branch }, 0..) |plan, index| {
            var counts: Counts = .{};
            defer std.debug.assert(counts.live == 0);
            var cancellation: fx.Cancellation = .{};
            const env: CancelEnv = .{ .allocator = allocator, .cancellation = &cancellation, .counts = &counts };
            var result = fx.run(&env, plan);
            defer result.deinit();
            if (oom(result)) return error.OutOfMemory;
            try std.testing.expect(result == .interrupted);
            try std.testing.expectEqual(@as(usize, 0), counts.live);
            try std.testing.expectEqual(@as(usize, 1), counts.created);
            try std.testing.expectEqual(@as(usize, 0), counts.later);
            try std.testing.expectEqual(@as(usize, 0), counts.no);
            try std.testing.expectEqual(@as(usize, @intFromBool(index >= 4)), counts.releases);
        }
    }
};
test "PROG-02 CAP-08 LIFE-01 cancellation before ordinary child admission discards owning succeed terminals from branch and binding callbacks before lexical release" {
    try CancelSteps.check(std.testing.allocator);
    try std.testing.checkAllAllocationFailures(std.testing.allocator, CancelSteps.check, .{});
}
