const std = @import("std");
const fx = @import("effect");
const gates = @import("support/dsl_gate.zig");
const E = enum { expected };
const Change = fx.dsl.RefChange(u32, u32);
const Env = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, async_engine: fx.Executor = undefined, gate: ?*gates.Gate(Change, E) = null };
const Seed = struct { initial: u32 };
const Init = struct {
    fn run(seed: *const Seed, _: *const Env) u32 {
        return seed.initial;
    }
};

test "FAC-02 INF-04 inferred Ref initialization and native modifications reuse scope and synchronized state" {
    const env: Env = .{ .allocator = std.testing.allocator };
    var scope = fx.Scope(E).init(&env);
    var acquire = fx.dsl.makeRef(&env, &scope, Seed{ .initial = 4 }, .{ .initialize = Init.run });
    var acquired = fx.run(&env, acquire);
    acquire.deinit();
    defer acquired.deinit();
    try std.testing.expect(acquired == .success);
    const reference = acquired.success;
    const modified = fx.Dsl(E, Env).modifyRef(reference, .{ .run = struct {
        fn run(value: u32) struct { result: u64, value: u32 } {
            return .{ .result = value, .value = value + 1 };
        }
    }.run });
    comptime if (@TypeOf(modified) != fx.Effect(u64, E, Env)) @compileError("Ref modification must return exact Effect");
    try std.testing.expectEqual(@as(u64, 4), fx.run(&env, modified).success);
    var captured = fx.dsl.modifyRefCaptured(E, &env, reference, @as(u32, 3), .{ .run = struct {
        fn run(delta: *const u32, value: u32, _: *const Env) struct { result: u32, value: u32 } {
            return .{ .result = value, .value = value + delta.* };
        }
    }.run });
    try std.testing.expectEqual(@as(u32, 5), fx.run(&env, captured).success);
    captured.deinit();
    try std.testing.expectEqual(@as(u32, 8), fx.run(&env, fx.Dsl(E, Env).getRef(reference)).success);
    var closed = fx.run(&env, fx.Dsl(E, Env).closeScope(&scope));
    defer closed.deinit();
    try std.testing.expect(closed == .success);
    scope.deinit();
}

test "FAC-02 effectful Ref transition commits replacement only on success and preserves failure" {
    const Operation = struct {
        fn run(should_fail: *const bool, value: u32, _: *const Env) fx.Effect(Change, E, Env) {
            return if (should_fail.*) fx.Effect(Change, E, Env).fail(.expected) else fx.Effect(Change, E, Env).succeed(fx.dsl.refChange(value, value + 1));
        }
    };
    const env: Env = .{ .allocator = std.testing.allocator };
    var reference = fx.Ref(u32).init(&env, 9);
    inline for (.{ true, false }) |fail| {
        var described = fx.dsl.modifyRefEffect(&env, &reference, fail, .{ .run = Operation.run });
        comptime if (@TypeOf(described) != fx.Effect(u32, E, Env)) @compileError("effectful Ref must return exact Effect");
        var result = fx.run(&env, described);
        defer result.deinit();
        described.deinit();
        if (fail) {
            try std.testing.expect(result == .fail and result.fail == .expected);
            try std.testing.expectEqual(@as(u32, 9), reference.get());
        } else {
            try std.testing.expectEqual(@as(u32, 9), result.success);
            try std.testing.expectEqual(@as(u32, 10), reference.get());
        }
    }
}

test "FAC-02 pending Ref transition keeps its lock through resume or cancellation and always releases it" {
    const Operation = struct {
        fn run(_: *const void, _: u32, _: *const Env) fx.Effect(Change, E, Env) {
            return fx.Effect(Change, E, Env).fromAsync(struct {
                fn start(env: *const Env) fx.Async(Change, E).Start {
                    return env.gate.?.start();
                }
            }.start);
        }
    };
    inline for (.{ false, true }) |cancel| {
        var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = true }, .maximum_owned = 2 };
        defer engine.deinit();
        var gate: gates.Gate(Change, E) = .{ .result = .{ .success = fx.dsl.refChange(@as(u32, 9), @as(u32, 10)) } };
        const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor(), .gate = &gate };
        var reference = fx.Ref(u32).init(&env, 9);
        var described = fx.dsl.modifyRefEffect(&env, &reference, @as(void, {}), .{ .run = Operation.run });
        var pending = fx.runAsync(&env, described);
        defer pending.deinit();
        gate.started.waitUncancelable(env.io);
        if (cancel) pending.request() else gate.proceed.set(env.io);
        var result = pending.join();
        defer result.deinit();
        described.deinit();
        try std.testing.expect(if (cancel) result == .interrupted else result == .success);
        try std.testing.expectEqual(@as(u32, if (cancel) 9 else 10), reference.get());
        try std.testing.expect(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
    }
}

test "FAC-02 ASYNC-03 overlapping effectful Ref transitions serialize replacement reads and writes" {
    const Operation = struct {
        fn run(pending: *const bool, value: u32, _: *const Env) fx.Effect(Change, E, Env) {
            if (!pending.*) return fx.Effect(Change, E, Env).succeed(fx.dsl.refChange(value, value + 1));
            return fx.Effect(Change, E, Env).fromAsync(struct {
                fn start(env: *const Env) fx.Async(Change, E).Start {
                    return env.gate.?.start();
                }
            }.start);
        }
    };
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = true }, .maximum_owned = 4 };
    defer engine.deinit();
    var gate: gates.Gate(Change, E) = .{ .result = .{ .success = fx.dsl.refChange(@as(u32, 9), @as(u32, 10)) } };
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor(), .gate = &gate };
    var reference = fx.Ref(u32).init(&env, 9);
    var first_description = fx.dsl.modifyRefEffect(&env, &reference, true, .{ .run = Operation.run });
    defer first_description.deinit();
    var first = fx.runAsync(&env, first_description);
    defer first.deinit();
    gate.started.waitUncancelable(env.io);
    var second_description = fx.dsl.modifyRefEffect(&env, &reference, false, .{ .run = Operation.run });
    defer second_description.deinit();
    var second = fx.runAsync(&env, second_description);
    defer second.deinit();
    gate.proceed.set(env.io);
    var first_result = first.join();
    defer first_result.deinit();
    var second_result = second.join();
    defer second_result.deinit();
    try std.testing.expect(first_result == .success and second_result == .success);
    try std.testing.expectEqual(@as(u32, 9), first_result.success);
    try std.testing.expectEqual(@as(u32, 10), second_result.success);
    try std.testing.expectEqual(@as(u32, 11), reference.get());
}

test "FAC-02 CAP-07 native and effectful Ref results transfer nested noncloneable owners once" {
    const Owner = struct {
        pub const effect_owned_payload = true;
        live: *usize,
        pub fn deinit(self: *@This()) void {
            self.live.* -= 1;
        }
    };
    const Payload = struct { token: Owner };
    const Inputs = struct { owner: Owner, results: *usize };
    const Operation = struct {
        fn native(inputs: *const Inputs, value: u32, _: *const Env) struct { result: Payload, value: u32 } {
            inputs.results.* += 1;
            return .{ .result = .{ .token = .{ .live = inputs.results } }, .value = value + 1 };
        }
        fn effectful(inputs: *const Inputs, _: u32, _: *const Env) fx.Effect(fx.dsl.RefChange(u32, Payload), E, Env) {
            return fx.Effect(fx.dsl.RefChange(u32, Payload), E, Env).fromBorrowed(Inputs, inputs, struct {
                fn run(stored: *const Inputs, _: *const Env) fx.Exit(fx.dsl.RefChange(u32, Payload), E) {
                    stored.results.* += 1;
                    return .{ .success = fx.dsl.refChange(Payload{ .token = .{ .live = stored.results } }, @as(u32, 27)) };
                }
            }.run);
        }
    };
    const env: Env = .{ .allocator = std.testing.allocator };
    inline for (.{ false, true }) |effectful| {
        var captures: usize = 1;
        var results: usize = 0;
        var reference = fx.Ref(u32).init(&env, 9);
        const inputs: Inputs = .{ .owner = .{ .live = &captures }, .results = &results };
        var described = if (effectful) fx.dsl.modifyRefEffect(&env, &reference, inputs, .{ .run = Operation.effectful }) else fx.dsl.modifyRefCaptured(E, &env, &reference, inputs, .{ .run = Operation.native });
        for (0..2) |_| {
            var result = fx.run(&env, described);
            try std.testing.expect(result == .success);
            try std.testing.expectEqual(@as(usize, 1), captures);
            try std.testing.expectEqual(@as(usize, 1), results);
            result.deinit();
            try std.testing.expectEqual(@as(usize, 0), results);
        }
        described.deinit();
        try std.testing.expectEqual(@as(usize, 0), captures);
        try std.testing.expectEqual(@as(u32, if (effectful) 27 else 11), reference.get());
    }
}

test "FAC-02 LIFE-07 late cancellation drops owning transition result and leaves Ref unchanged" {
    const Owner = struct {
        pub const effect_owned_payload = true;
        live: *usize,
        pub fn deinit(self: *@This()) void {
            self.live.* -= 1;
        }
    };
    const Runtime = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, cancellation: *fx.Cancellation, live: *usize };
    const Changed = fx.dsl.RefChange(u32, struct { owner: Owner });
    const Operation = struct {
        fn run(_: *const void, _: u32, _: *const Runtime) fx.Effect(Changed, E, Runtime) {
            return fx.Effect(Changed, E, Runtime).fromOutcome(struct {
                fn complete(env: *const Runtime) fx.Exit(Changed, E) {
                    env.live.* += 1;
                    env.cancellation.request();
                    return .{ .success = .{ .result = .{ .owner = .{ .live = env.live } }, .value = 99 } };
                }
            }.complete);
        }
    };
    var cancellation: fx.Cancellation = .{};
    var live: usize = 0;
    const env: Runtime = .{ .allocator = std.testing.allocator, .cancellation = &cancellation, .live = &live };
    var reference = fx.Ref(u32).init(&env, 9);
    var described = fx.dsl.modifyRefEffect(&env, &reference, @as(void, {}), .{ .run = Operation.run });
    defer described.deinit();
    var result = fx.run(&env, described);
    defer result.deinit();
    try std.testing.expect(result == .interrupted);
    try std.testing.expectEqual(@as(usize, 0), live);
    try std.testing.expectEqual(@as(u32, 9), reference.get());
}

test "FAC-02 CAP-07 LIFE-01 canonical owning RefChange composes before late cancellation without leaking" {
    const Owner = struct {
        pub const effect_owned_payload = true;
        live: *usize,
        released: *usize,
        pub fn deinit(self: *@This()) void {
            self.live.* -= 1;
            self.released.* += 1;
        }
    };
    const Runtime = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, cancellation: *fx.Cancellation, live: *usize, released: *usize };
    const Changed = fx.dsl.RefChange(u32, struct { owner: Owner });
    const Operation = struct {
        fn create(env: *const Runtime) fx.Exit(Changed, E) {
            env.live.* += 1;
            env.cancellation.request();
            return .{ .success = .{ .result = .{ .owner = .{ .live = env.live, .released = env.released } }, .value = 99 } };
        }
        fn identity(value: Changed) Changed {
            return value;
        }
        fn run(_: *const void, _: u32, _: *const Runtime) fx.Effect(Changed, E, Runtime) {
            return comptime fx.Effect(Changed, E, Runtime).fromOutcome(create).map(identity);
        }
    };
    var cancellation: fx.Cancellation = .{};
    var live: usize = 0;
    var released: usize = 0;
    const env: Runtime = .{ .allocator = std.testing.allocator, .cancellation = &cancellation, .live = &live, .released = &released };
    var reference = fx.Ref(u32).init(&env, 9);
    var described = fx.dsl.modifyRefEffect(&env, &reference, @as(void, {}), .{ .run = Operation.run });
    defer described.deinit();
    var result = fx.run(&env, described);
    defer result.deinit();
    try std.testing.expect(result == .interrupted);
    try std.testing.expectEqual(@as(u32, 9), reference.get());
    try std.testing.expectEqual(@as(usize, 0), live);
    try std.testing.expectEqual(@as(usize, 1), released);
}

test "FAC-02 RES-03 Ref initialization is lazy fresh scoped and rolls back rejected registrations under every allocation failure" {
    const State = struct { live: usize = 1, calls: usize = 0 };
    const Inputs = struct {
        pub const effect_owned_payload = true;
        state: *State,
        pub fn deinit(self: *@This()) void {
            self.state.live -= 1;
        }
    };
    const Operation = struct {
        fn initialize(inputs: *const Inputs, _: *const Env) fx.Exit(u32, E) {
            inputs.state.calls += 1;
            return .{ .success = 16 + @as(u32, @intCast(inputs.state.calls)) };
        }
        fn effectful(inputs: *const Inputs, _: *const Env) fx.Effect(u32, E, Env) {
            return fx.Effect(u32, E, Env).fromBorrowed(Inputs, inputs, initialize);
        }
    };
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            inline for (.{ false, true }) |effectful| inline for (.{ false, true }) |closed| {
                const env: Env = .{ .allocator = allocator };
                var scope = fx.Scope(E).init(&env);
                if (closed) {
                    var result = fx.run(&env, fx.Dsl(E, Env).closeScope(&scope));
                    result.deinit();
                }
                var state: State = .{};
                var described = fx.dsl.makeRef(&env, &scope, Inputs{ .state = &state }, .{ .initialize = if (effectful) Operation.effectful else Operation.initialize });
                try std.testing.expectEqual(@as(usize, 0), state.calls);
                var result = fx.run(&env, described);
                var repeated: ?fx.Exit(*fx.Ref(u32), E) = null;
                defer if (repeated) |*value| value.deinit();
                if (result == .success) {
                    try std.testing.expect(!closed);
                    try std.testing.expectEqual(@as(u32, 17), result.success.get());
                    try std.testing.expectEqual(@as(usize, 1), state.live);
                    repeated = fx.run(&env, described);
                    if (repeated.? == .success) {
                        try std.testing.expect(result.success != repeated.?.success);
                        try std.testing.expectEqual(@as(u32, 18), repeated.?.success.get());
                    }
                }
                described.deinit();
                var cleanup = fx.run(&env, fx.Dsl(E, Env).closeScope(&scope));
                cleanup.deinit();
                scope.deinit();
                try std.testing.expectEqual(@as(usize, 0), state.live);
                defer result.deinit();
                if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
                if (repeated) |value| if (value == .die and value.die.kind == .out_of_memory) return error.OutOfMemory;
                try std.testing.expectEqual(@as(usize, if (closed) 1 else 2), state.calls);
                if (closed) try std.testing.expect(result == .die and result.die.kind == .invalid_configuration) else try std.testing.expect(result == .success);
            };
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

test "FAC-02 native and effectful Ref initializers preserve failure defect interruption and late cancellation" {
    const Mode = enum { failure, defect, interrupted, late_cancellation };
    const Runtime = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, cancellation: *fx.Cancellation, calls: *usize };
    const Operation = struct {
        fn initialize(mode: *const Mode, env: *const Runtime) fx.Exit(u32, E) {
            env.calls.* += 1;
            return switch (mode.*) {
                .failure => .{ .fail = .expected },
                .defect => .{ .die = fx.Defect.init("initializer defect") },
                .interrupted => .interrupted,
                .late_cancellation => blk: {
                    env.cancellation.request();
                    break :blk .{ .success = 17 };
                },
            };
        }
        fn effectful(mode: *const Mode, _: *const Runtime) fx.Effect(u32, E, Runtime) {
            return fx.Effect(u32, E, Runtime).fromBorrowed(Mode, mode, initialize);
        }
    };
    inline for (.{ false, true }) |effectful| for (std.meta.tags(Mode)) |mode| {
        var cancellation: fx.Cancellation = .{};
        var calls: usize = 0;
        const env: Runtime = .{ .allocator = std.testing.allocator, .cancellation = &cancellation, .calls = &calls };
        var scope = fx.Scope(E).init(&env);
        var described = fx.dsl.makeRef(&env, &scope, mode, .{ .initialize = if (effectful) Operation.effectful else Operation.initialize });
        var result = fx.run(&env, described);
        described.deinit();
        defer result.deinit();
        try std.testing.expectEqual(@as(usize, 1), calls);
        switch (mode) {
            .failure => try std.testing.expect(result == .fail and result.fail == .expected),
            .defect => try std.testing.expect(result == .die and std.mem.eql(u8, result.die.message, "initializer defect")),
            .interrupted, .late_cancellation => try std.testing.expect(result == .interrupted),
        }
        cancellation = .{};
        var cleanup = fx.run(&env, fx.Dsl(E, Runtime).closeScope(&scope));
        defer cleanup.deinit();
        try std.testing.expect(cleanup == .success);
        scope.deinit();
    };
}

test "FAC-02 LIFE-07 effectful Ref setup allocation failures release captures and preserve usable state" {
    const Owner = struct {
        pub const effect_owned_payload = true;
        live: *usize,
        pub fn deinit(self: *@This()) void {
            self.live.* -= 1;
        }
    };
    const Inputs = struct { owner: Owner, calls: *usize };
    const Operation = struct {
        fn run(inputs: *const Inputs, value: u32, _: *const Env) fx.Effect(Change, E, Env) {
            inputs.calls.* += 1;
            return fx.Effect(Change, E, Env).succeed(fx.dsl.refChange(value, value + 1));
        }
    };
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            var live: usize = 1;
            var calls: usize = 0;
            const env: Env = .{ .allocator = allocator };
            var reference = fx.Ref(u32).init(&env, 9);
            var described = fx.dsl.modifyRefEffect(&env, &reference, Inputs{ .owner = .{ .live = &live }, .calls = &calls }, .{ .run = Operation.run });
            var result = fx.run(&env, described);
            described.deinit();
            defer result.deinit();
            try std.testing.expectEqual(@as(usize, 0), live);
            if (result == .die and result.die.kind == .out_of_memory) {
                try std.testing.expectEqual(@as(usize, 0), calls);
                try std.testing.expectEqual(@as(u32, 9), reference.get());
                return error.OutOfMemory;
            }
            try std.testing.expect(result == .success);
            try std.testing.expectEqual(@as(usize, 1), calls);
            try std.testing.expectEqual(@as(u32, 10), reference.get());
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}
