const std = @import("std");
const fx = @import("effect");
const p = fx.dsl.program;
const Threads = @import("support/thread_executor.zig").ThreadExecutor;
const E = enum { body };
const Gate = @import("support/dsl_gate.zig").Gate(void, E);
const Mode = enum { normal, failure, defect, interrupted, pending };
const Counts = struct { acquired: usize = 0, released: usize = 0, active: usize = 0, finished: usize = 0, state_tokens: usize = 0, state_created: usize = 0 };
const Env = struct {
    allocator: std.mem.Allocator,
    io: std.Io = std.testing.io,
    async_engine: fx.Executor,
    counts: *Counts,
    mode: Mode = .normal,
    gate: ?*Gate = null,
};
const Lease = struct { iteration: u32 };
const Control = p.LoopControl(u32);
const Body = fx.Effect(Control, E, Env);
const Unit = fx.Effect(void, E, Env);
const Steps = struct {
    fn initialize(_: *const Env) u32 {
        return 0;
    }
    fn condition(position: *const u32, env: *const Env) bool {
        std.debug.assert(env.counts.active == 0 and env.counts.acquired == env.counts.released);
        return position.* < 3;
    }
    fn acquire(configuration: anytype, _: *const Env) fx.Effect(*Lease, E, Env) {
        const C = @typeInfo(@TypeOf(configuration)).pointer.child;
        return fx.Effect(*Lease, E, Env).fromBorrowed(C, configuration, struct {
            fn run(stored: *const C, env: *const Env) fx.Exit(*Lease, E) {
                std.debug.assert(env.counts.active == 0);
                const lease = env.allocator.create(Lease) catch return .{ .die = fx.Defect.out_of_memory };
                lease.* = .{ .iteration = stored.view.state().* };
                std.debug.assert(lease.iteration == env.counts.acquired);
                env.counts.acquired += 1;
                env.counts.active += 1;
                return .{ .success = lease };
            }
        }.run);
    }
    fn use(configuration: anytype, lease: *Lease, env: *const Env) Body {
        std.debug.assert(lease.iteration == configuration.view.state().* and env.counts.active == 1);
        const C = @typeInfo(@TypeOf(configuration)).pointer.child;
        var pending = if (lease.iteration == 1) switch (env.mode) {
            .normal => Unit.succeed({}),
            .failure => Unit.fail(.body),
            .defect => Unit.die(fx.Defect.init("loop iteration")),
            .interrupted => Unit.interrupted(),
            .pending => Unit.fromAsync(struct {
                fn wait(current: *const Env) fx.Async(void, E).Start {
                    return current.gate.?.start();
                }
            }.wait),
        } else Unit.succeed({});
        return pending.flatMapCapturedOwned(env, Control, configuration, struct {
            fn advance(stored: *const *const C, _: void, current: *const Env) Body {
                std.debug.assert(current.counts.active == 1);
                stored.*.view.state().* += 1;
                return Body.succeed(.again);
            }
        }.advance);
    }
    fn release(_: anytype, lease: *Lease, _: *const fx.PrimaryOutcome(E), _: *const Env) Unit {
        return Unit.fromBorrowed(Lease, lease, struct {
            fn close(current: *const Lease, env: *const Env) fx.Exit(void, E) {
                std.debug.assert(env.counts.active == 1 and current.iteration == env.counts.released);
                if (env.mode == .pending and current.iteration == 1) {
                    std.debug.assert(env.gate.?.joined.load(.acquire) and env.gate.?.destroyed.load(.acquire));
                }
                env.counts.active -= 1;
                env.counts.released += 1;
                env.allocator.destroy(@constCast(current));
                return .{ .success = {} };
            }
        }.close);
    }
    fn iteration(view: anytype, env: *const Env) Body {
        return fx.dsl.resourceCaptured(env, .{ .view = view }, .{ .acquire = acquire, .body = use, .release = release });
    }
    fn finish(view: anytype, env: *const Env) u32 {
        std.debug.assert(env.counts.active == 0);
        env.counts.finished += 1;
        return view.state().*;
    }
};
const description = p.build(E, Env, .{p.loop("iterations", .{}, Steps.initialize, Steps.condition, Steps.iteration, Steps.finish)});

test "PROG-03 LIFE-10 each structured loop iteration owns a fresh lexical resource and releases it before the next condition or terminal outcome" {
    inline for (.{ Mode.normal, Mode.failure, Mode.defect, Mode.interrupted }) |mode| {
        var counts: Counts = .{};
        var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
        defer threads.deinit();
        const env: Env = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &counts, .mode = mode };
        comptime if (@TypeOf(description) != fx.Effect(u32, E, Env)) @compileError("loop execution must be the exact existing Effect");
        try std.testing.expectEqual(@as(usize, 0), counts.acquired);
        var result = fx.run(&env, description);
        defer result.deinit();
        switch (mode) {
            .normal => try std.testing.expectEqual(@as(u32, 3), result.success),
            .failure => try std.testing.expectEqual(E.body, result.fail),
            .defect => try std.testing.expect(result == .die),
            .interrupted => try std.testing.expect(result == .interrupted),
            else => unreachable,
        }
        try std.testing.expectEqual(@as(usize, if (mode == .normal) 3 else 2), counts.acquired);
        try std.testing.expectEqual(counts.acquired, counts.released);
        try std.testing.expectEqual(@as(usize, 0), counts.active);
        try std.testing.expectEqual(@as(usize, if (mode == .normal) 1 else 0), counts.finished);
    }
}

test "PROG-03 LIFE-12 structured loop async owner destruction joins a suspended iteration before releasing its acquired lexical resource and frame" {
    var counts: Counts = .{};
    var gate: Gate = .{ .result = .{ .success = {} } };
    var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
    defer threads.deinit();
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &counts, .mode = .pending, .gate = &gate };
    var pending = fx.runAsync(&env, description);
    gate.started.waitUncancelable(env.io);
    try std.testing.expectEqual(@as(usize, 2), counts.acquired);
    try std.testing.expectEqual(@as(usize, 1), counts.released);
    try std.testing.expectEqual(@as(usize, 1), counts.active);
    pending.deinit();
    try std.testing.expect(gate.cancelled.load(.acquire) and gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
    try std.testing.expectEqual(@as(usize, 2), counts.released);
    try std.testing.expectEqual(@as(usize, 0), counts.active);
    try std.testing.expectEqual(@as(usize, 0), counts.finished);
}

test "PROG-03 LIFE-07 every loop iteration frame acquisition and body allocation failure releases only initialized lexical resources" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            var counts: Counts = .{};
            defer std.debug.assert(counts.active == 0 and counts.acquired == counts.released);
            var threads: Threads = .{ .allocator = allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
            defer threads.deinit();
            const env: Env = .{ .allocator = allocator, .async_engine = threads.executor(), .counts = &counts };
            var result = fx.run(&env, description);
            defer result.deinit();
            if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
            if (result == .finalization_failed) {
                if (result.finalization_failed.primary == .defected and result.finalization_failed.primary.defected.kind == .out_of_memory) return error.OutOfMemory;
                if (result.finalization_failed.first == .release_defect and result.finalization_failed.first.release_defect.kind == .out_of_memory) return error.OutOfMemory;
                var cursor = result.finalization_failed.rest;
                while (cursor) |node| {
                    if (node.failure == .release_defect and node.failure.release_defect.kind == .out_of_memory) return error.OutOfMemory;
                    cursor = node.next;
                }
            }
            try std.testing.expectEqual(@as(u32, 3), result.success);
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

test "PROG-01 PROG-03 CAP-08 structured programs transfer owning loop native and Effect results through generated payloads and business value projections" {
    const Token = struct {
        pub const effect_owned_payload = true;
        counts: *Counts,
        pub fn deinit(self: *@This()) void {
            self.counts.state_tokens -= 1;
        }
    };
    const State = struct { token: Token, number: u32 };
    const Loop = struct {
        fn initialize(env: *const Env) State {
            env.counts.state_tokens += 1;
            env.counts.state_created += 1;
            return .{ .token = .{ .counts = env.counts }, .number = 41 };
        }
        fn condition(_: *const State, env: *const Env) bool {
            return env.mode != .failure;
        }
        fn body(view: anytype, _: *const Env) fx.Effect(p.LoopControl(State), E, Env) {
            const V = @typeInfo(@TypeOf(view)).pointer.child;
            return fx.Effect(p.LoopControl(State), E, Env).fromBorrowed(V, view, struct {
                fn transfer(current: *const V, _: *const Env) fx.Exit(p.LoopControl(State), E) {
                    const state = @constCast(current).takeState() catch return .{ .die = fx.Defect.init("state already moved") };
                    return .{ .success = .{ .done = state } };
                }
            }.transfer);
        }
        fn finish(view: anytype, _: *const Env) State {
            return view.takeState() catch unreachable;
        }
        fn number(value: *const State, _: *const Env) fx.Effect(u32, E, Env) {
            return fx.Effect(u32, E, Env).succeed(value.number);
        }
        fn plainNumber(value: *const State, _: *const Env) u32 {
            return value.number;
        }
        const native_owned = p.build(E, Env, .{p.native("owned", .{}, initialize)});
        const native_projected = p.build(E, Env, .{ p.native("owned", .{}, initialize), p.native("number", .{"owned"}, plainNumber) });
        const mixed_projected = p.build(E, Env, .{ p.native("owned", .{}, initialize), p.binding("number", .{"owned"}, number) });
        fn effect(env: *const Env) fx.Effect(State, E, Env) {
            _ = env;
            return fx.Effect(State, E, Env).fromOutcome(struct {
                fn create(current: *const Env) fx.Exit(State, E) {
                    return .{ .success = initialize(current) };
                }
            }.create);
        }
        fn always(_: *const Env) bool {
            return true;
        }
        const effect_owned = p.build(E, Env, .{p.binding("owned", .{}, effect)});
        const effect_projected = p.build(E, Env, .{ p.binding("owned", .{}, effect), p.binding("number", .{"owned"}, number) });
        const effect_early = p.build(E, Env, .{ p.binding("owned", .{}, effect), p.returnWhen("early", .{}, always, "owned"), p.binding("fallback", .{}, effect) });
    };
    const described = comptime p.build(E, Env, .{p.loop("owned", .{}, Loop.initialize, Loop.condition, Loop.body, Loop.finish)});
    comptime if (@TypeOf(described) != fx.Effect(fx.dsl.ownership.Capture(State), E, Env)) @compileError("owning loop output must use generated native payload ownership in exact Effect");
    var counts: Counts = .{};
    var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
    defer threads.deinit();
    var env: Env = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &counts };
    var result = fx.run(&env, described);
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(usize, 1), counts.state_tokens);
    result.deinit();
    try std.testing.expectEqual(@as(usize, 0), counts.state_tokens);
    const projected = comptime p.build(E, Env, .{
        p.loop("owned", .{}, Loop.initialize, Loop.condition, Loop.body, Loop.finish),
        p.binding("number", .{"owned"}, Loop.number),
    });
    var projection = fx.run(&env, projected);
    try std.testing.expectEqual(@as(u32, 41), projection.success);
    projection.deinit();
    try std.testing.expectEqual(@as(usize, 0), counts.state_tokens);
    env.mode = .failure; // the condition exits through finish before the body
    var finished = fx.run(&env, described);
    try std.testing.expectEqual(@as(u32, 41), finished.success.value.number);
    finished.deinit();
    try std.testing.expectEqual(@as(usize, 0), counts.state_tokens);
    var effect_result = fx.run(&env, Loop.effect_owned);
    try std.testing.expect(effect_result == .success);
    effect_result.deinit();
    try std.testing.expectEqual(@as(usize, 0), counts.state_tokens);
    var effect_projection = fx.run(&env, Loop.effect_projected);
    try std.testing.expectEqual(@as(u32, 41), effect_projection.success);
    effect_projection.deinit();
    try std.testing.expectEqual(@as(usize, 0), counts.state_tokens);
    const created_before = counts.state_created;
    var early = fx.run(&env, Loop.effect_early);
    try std.testing.expect(early == .success);
    try std.testing.expectEqual(created_before + 1, counts.state_created);
    early.deinit();
    try std.testing.expectEqual(@as(usize, 0), counts.state_tokens);
    var native_result = fx.run(&env, Loop.native_owned);
    try std.testing.expect(native_result == .success);
    native_result.deinit();
    try std.testing.expectEqual(@as(usize, 0), counts.state_tokens);
    var native_projection = fx.run(&env, Loop.native_projected);
    try std.testing.expectEqual(@as(u32, 41), native_projection.success);
    native_projection.deinit();
    try std.testing.expectEqual(@as(usize, 0), counts.state_tokens);
    var mixed_projection = fx.run(&env, Loop.mixed_projected);
    try std.testing.expectEqual(@as(u32, 41), mixed_projection.success);
    mixed_projection.deinit();
    try std.testing.expectEqual(@as(usize, 0), counts.state_tokens);
    inline for (.{ false, true }) |via_finish| {
        inline for (.{ false, true }) |with_projection| {
            const Check = struct {
                fn run(allocator: std.mem.Allocator) !void {
                    var owned: Counts = .{};
                    defer std.debug.assert(owned.state_tokens == 0);
                    var executor: Threads = .{ .allocator = allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
                    defer executor.deinit();
                    const current: Env = .{ .allocator = allocator, .async_engine = executor.executor(), .counts = &owned, .mode = if (via_finish) .failure else .normal };
                    const selected = if (comptime with_projection) projected else described;
                    var exit = fx.run(&current, selected);
                    defer exit.deinit();
                    if (exit == .die and exit.die.kind == .out_of_memory) return error.OutOfMemory;
                    try std.testing.expect(exit == .success);
                    if (comptime with_projection)
                        try std.testing.expectEqual(@as(u32, 41), exit.success)
                    else
                        try std.testing.expectEqual(@as(u32, 41), exit.success.value.number);
                }
            };
            try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
        }
    }
    inline for (.{ Loop.native_owned, Loop.native_projected, Loop.mixed_projected, Loop.effect_owned, Loop.effect_projected, Loop.effect_early }) |selected| {
        const Check = struct {
            fn run(allocator: std.mem.Allocator) !void {
                var owned: Counts = .{};
                defer std.debug.assert(owned.state_tokens == 0);
                var executor: Threads = .{ .allocator = allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
                defer executor.deinit();
                const current: Env = .{ .allocator = allocator, .async_engine = executor.executor(), .counts = &owned };
                var exit = fx.run(&current, selected);
                defer exit.deinit();
                if (exit == .die and exit.die.kind == .out_of_memory) return error.OutOfMemory;
                try std.testing.expect(exit == .success);
                if (comptime @TypeOf(selected).Success == u32)
                    try std.testing.expectEqual(@as(u32, 41), exit.success)
                else
                    try std.testing.expectEqual(@as(u32, 41), exit.success.value.number);
            }
        };
        try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
    }
}

test "PROG-01 PROG-04 LIFE-01 ordinary owning Effect binding and early return cancellation clean initialized slots before later work" {
    const R = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, cancellation: *fx.Cancellation, live: *usize, later: *usize, cancel_in_source: bool };
    const Token = struct {
        pub const effect_owned_payload = true;
        live: *usize,
        pub fn deinit(self: *@This()) void {
            self.live.* -= 1;
        }
    };
    const S = struct { token: Token, number: u32 };
    const Functions = struct {
        fn source(_: *const R) fx.Effect(S, E, R) {
            return fx.Effect(S, E, R).fromOutcome(struct {
                fn create(env: *const R) fx.Exit(S, E) {
                    env.live.* += 1;
                    if (env.cancel_in_source) env.cancellation.request();
                    return .{ .success = .{ .token = .{ .live = env.live }, .number = 41 } };
                }
            }.create);
        }
        fn predicate(env: *const R) bool {
            env.cancellation.request();
            return true;
        }
        fn later(_: *const S, env: *const R) fx.Effect(u32, E, R) {
            env.later.* += 1;
            return fx.Effect(u32, E, R).succeed(99);
        }
        const final = p.build(E, R, .{p.binding("owned", .{}, source)});
        const projected = p.build(E, R, .{ p.binding("owned", .{}, source), p.binding("later", .{"owned"}, later) });
        const early = p.build(E, R, .{ p.binding("owned", .{}, source), p.returnWhen("early", .{}, predicate, "owned"), p.binding("fallback", .{}, source) });
    };
    inline for (.{ .{ .operation = Functions.final, .cancel_in_source = true }, .{ .operation = Functions.projected, .cancel_in_source = true }, .{ .operation = Functions.early, .cancel_in_source = false } }) |case| {
        var live: usize = 0;
        var later: usize = 0;
        var cancellation: fx.Cancellation = .{};
        const env: R = .{ .allocator = std.testing.allocator, .cancellation = &cancellation, .live = &live, .later = &later, .cancel_in_source = case.cancel_in_source };
        var result = fx.run(&env, case.operation);
        try std.testing.expect(result == .interrupted);
        result.deinit();
        try std.testing.expectEqual(@as(usize, 0), live);
        try std.testing.expectEqual(@as(usize, 0), later);
    }
}
