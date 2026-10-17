const std = @import("std");
const fx = @import("effect");
const gates = @import("support/dsl_gate.zig");
const Controlled = @import("support/dsl_admission.zig").Controlled;
const E = enum { expected };
const Env = struct {
    allocator: std.mem.Allocator,
    io: std.Io = std.testing.io,
    async_engine: fx.Executor = undefined,
    calls: *std.atomic.Value(usize),
    live: *std.atomic.Value(usize),
    fail_at: ?u32 = null,
    capture_live: ?*std.atomic.Value(bool) = null,
    pending_gates: ?*[2]gates.Gate(u32, E) = null,
    invalid: ?*std.atomic.Value(bool) = null,
    items_live: ?*std.atomic.Value(bool) = null,
    fail_child: bool = false,
};
const P = fx.Effect(u32, E, Env);
const Payload = struct {
    pub const effect_owned_payload = true;
    allocator: std.mem.Allocator,
    bytes: []u8,
    live: *std.atomic.Value(usize),
    capture_live: ?*std.atomic.Value(bool) = null,
    pub fn deinit(self: *@This()) void {
        if (self.capture_live) |flag| flag.store(false, .release);
        self.allocator.free(self.bytes);
        _ = self.live.fetchSub(1, .acq_rel);
    }
};

test "COLL-01 COMP-03 static tuples and borrowed runtime all descriptions stay lazy and ordered" {
    const leaf = comptime P.sync(struct {
        fn run(env: *const Env) u32 {
            return @intCast(env.calls.fetchAdd(1, .acq_rel) + 1);
        }
    }.run);
    const described = comptime fx.dsl.all(.{ leaf, leaf, leaf });
    comptime if (@TypeOf(described) != fx.Effect(fx.OwnedSlice(u32), E, Env)) @compileError("all must return exact Effect");
    var calls: std.atomic.Value(usize) = .init(0);
    var live: std.atomic.Value(usize) = .init(0);
    const env: Env = .{ .allocator = std.testing.allocator, .calls = &calls, .live = &live };
    try std.testing.expectEqual(@as(usize, 0), calls.load(.acquire));
    var result = fx.run(&env, described);
    defer result.deinit();
    try std.testing.expect(result == .success);
    try std.testing.expectEqualSlices(u32, &.{ 1, 2, 3 }, result.success.items);
    const programs = [_]P{ leaf, leaf };
    var borrowed = fx.run(&env, fx.dsl.allBorrowed(&programs));
    defer borrowed.deinit();
    try std.testing.expectEqualSlices(u32, &.{ 4, 5 }, borrowed.success.items);
    var empty = fx.run(&env, comptime fx.dsl.allOf(P, .{}));
    defer empty.deinit();
    try std.testing.expect(empty == .success and empty.success.items.len == 0);
}

const Inputs = struct { factor: u32 };

test "COLL-02 CAP-04 captured borrowed traversal retains typed rows through deferred bodies and reuse" {
    const Body = struct {
        fn run(inputs: *const Inputs, item: *const u32, env: *const Env) P {
            return fx.dsl.owned(env, .{ .inputs = inputs, .item = item }, struct {
                fn later(value: anytype, current: *const Env) P {
                    _ = current.calls.fetchAdd(1, .acq_rel);
                    return P.succeed(value.item.* * value.inputs.factor);
                }
            }.later);
        }
    };
    var calls: std.atomic.Value(usize) = .init(0);
    var live: std.atomic.Value(usize) = .init(0);
    const env: Env = .{ .allocator = std.testing.allocator, .calls = &calls, .live = &live };
    const values = [_]u32{ 4, 5, 6 };
    var described = fx.dsl.forEachCaptured(&env, &values, Inputs{ .factor = 2 }, .{ .body = Body.run });
    defer described.deinit();
    try std.testing.expectEqual(@as(usize, 0), calls.load(.acquire));
    for (0..2) |_| {
        var result = fx.run(&env, described);
        defer result.deinit();
        try std.testing.expect(result == .success);
        try std.testing.expectEqualSlices(u32, &.{ 8, 10, 12 }, result.success.items);
    }
    try std.testing.expectEqual(@as(usize, 6), calls.load(.acquire));
}

test "COLL-02 CAP-09 owning collection factory produces fresh noncloneable items for each execution" {
    const Items = fx.OwnedSlice(Payload);
    const Collection = struct {
        fn items(_: *const Inputs, _: *const Env) fx.Effect(Items, E, Env) {
            return fx.Effect(Items, E, Env).fromOutcome(struct {
                fn run(env: *const Env) fx.Exit(Items, E) {
                    const values = env.allocator.alloc(Payload, 1) catch return .{ .die = fx.Defect.out_of_memory };
                    const bytes = env.allocator.dupe(u8, "borrow without cloning") catch {
                        env.allocator.free(values);
                        return .{ .die = fx.Defect.out_of_memory };
                    };
                    _ = env.live.fetchAdd(1, .acq_rel);
                    values[0] = .{ .allocator = env.allocator, .bytes = bytes, .live = env.live };
                    return .{ .success = .{ .allocator = env.allocator, .items = values } };
                }
            }.run);
        }
        fn body(_: *const Inputs, item: *const Payload, _: *const Env) P {
            return P.deferBorrowed(Payload, item, struct {
                fn run(value: *const Payload, env: *const Env) P {
                    if (env.live.load(.acquire) != 1) return P.die(fx.Defect.init("input owner ended before deferred item body"));
                    _ = env.calls.fetchAdd(1, .acq_rel);
                    return P.succeed(@intCast(value.bytes.len));
                }
            }.run);
        }
    };
    var calls: std.atomic.Value(usize) = .init(0);
    var live: std.atomic.Value(usize) = .init(0);
    const env: Env = .{ .allocator = std.testing.allocator, .calls = &calls, .live = &live };
    var described = fx.dsl.forEachFactory(&env, Inputs{ .factor = 1 }, .{ .items = Collection.items, .body = Collection.body });
    defer described.deinit();
    try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
    for (0..2) |_| {
        var result = fx.run(&env, described);
        defer result.deinit();
        try std.testing.expect(result == .success);
        try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
        try std.testing.expectEqual(@as(u32, 22), result.success.items[0]);
    }
}

test "COLL-03 inferred bounded captured traversal preserves result order and configured providers" {
    const Body = struct {
        fn run(inputs: *const Inputs, item: *const u32, _: *const Env) P {
            return P.succeed(item.* * inputs.factor);
        }
    };
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = true }, .maximum_owned = 4 };
    defer engine.deinit();
    var calls: std.atomic.Value(usize) = .init(0);
    var live: std.atomic.Value(usize) = .init(0);
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor(), .calls = &calls, .live = &live };
    const values = [_]u32{ 3, 2, 4, 1 };
    var described = fx.dsl.forEachConcurrentCaptured(&env, &values, Inputs{ .factor = 3 }, 2, .{ .body = Body.run });
    defer described.deinit();
    var result = fx.run(&env, described);
    defer result.deinit();
    try std.testing.expect(result == .success);
    try std.testing.expectEqualSlices(u32, &.{ 9, 6, 12, 3 }, result.success.items);
}

test "COLL-01 ASYNC-01 sequential captured traversal needs no executor capability" {
    const Runtime = struct { allocator: std.mem.Allocator };
    const Value = fx.Effect(u32, void, Runtime);
    const roles = .{ .body = struct {
        fn run(_: *const void, item: *const u32, _: *const Runtime) Value {
            return Value.succeed(item.* + 1);
        }
    }.run };
    const env: Runtime = .{ .allocator = std.testing.allocator };
    const values = [_]u32{ 1, 2 };
    var described = fx.dsl.forEachCaptured(&env, &values, @as(void, {}), roles);
    defer described.deinit();
    var result = fx.run(&env, described);
    defer result.deinit();
    try std.testing.expect(result == .success);
    try std.testing.expectEqualSlices(u32, &.{ 2, 3 }, result.success.items);
    var plain = fx.dsl.forEach(&env, &values, .{ .body = struct {
        fn run(item: *const u32, _: *const Runtime) Value {
            return Value.succeed(item.* + 1);
        }
    }.run });
    defer plain.deinit();
    var plain_result = fx.run(&env, plain);
    defer plain_result.deinit();
    try std.testing.expect(plain_result == .success);
    try std.testing.expectEqualSlices(u32, result.success.items, plain_result.success.items);
}

test "COLL-05 LIFE-07 traversal failures release generated capture storage and initialized owning results" {
    const Output = fx.Effect(Payload, E, Env);
    const Body = struct {
        fn run(stored: *const Payload, item: *const u32, current: *const Env) Output {
            return fx.dsl.owned(current, .{ .stored = stored, .item = item }, struct {
                fn run(value: anytype, env: *const Env) Output {
                    if (env.fail_at == value.item.*) return Output.fail(.expected);
                    if (!env.capture_live.?.load(.acquire)) return Output.die(fx.Defect.init("capture ended before traversal body"));
                    if (value.stored.bytes.len == 0) return Output.die(fx.Defect.init("capture ended before traversal body"));
                    const bytes = env.allocator.dupe(u8, "result") catch return Output.die(fx.Defect.out_of_memory);
                    _ = env.live.fetchAdd(1, .acq_rel);
                    return Output.fromOwnedSuccess(env, .{ .allocator = env.allocator, .bytes = bytes, .live = env.live });
                }
            }.run);
        }
    };
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            inline for (.{ false, true }) |fail_midway| {
                var calls: std.atomic.Value(usize) = .init(0);
                var live: std.atomic.Value(usize) = .init(0);
                var capture_live: std.atomic.Value(bool) = .init(true);
                const env: Env = .{ .allocator = allocator, .calls = &calls, .live = &live, .fail_at = if (fail_midway) 2 else null, .capture_live = &capture_live };
                const bytes = try allocator.dupe(u8, "capture");
                _ = live.fetchAdd(1, .acq_rel);
                const values = [_]u32{ 1, 2, 3 };
                var described = fx.dsl.forEachCaptured(&env, &values, Payload{ .allocator = allocator, .bytes = bytes, .live = &live, .capture_live = &capture_live }, .{ .body = Body.run });
                var result = fx.run(&env, described);
                described.deinit();
                const oom = result == .die and result.die.kind == .out_of_memory;
                const expected = if (fail_midway) result == .fail and result.fail == .expected else result == .success;
                result.deinit();
                try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
                try std.testing.expect(!capture_live.load(.acquire));
                if (oom) return error.OutOfMemory;
                try std.testing.expect(expected);
            }
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

test "COLL-02 owned all retains and releases each runtime description without shallow owner copying" {
    var calls: std.atomic.Value(usize) = .init(0);
    var live: std.atomic.Value(usize) = .init(0);
    const env: Env = .{ .allocator = std.testing.allocator, .calls = &calls, .live = &live };
    const programs = try env.allocator.alloc(P, 2);
    for (programs) |*program| {
        const bytes = try env.allocator.dupe(u8, "owned leaf");
        _ = live.fetchAdd(1, .acq_rel);
        program.* = fx.Dsl(E, Env).syncOwned(&env, Payload{ .allocator = env.allocator, .bytes = bytes, .live = &live }, struct {
            fn run(value: *const Payload, _: *const Env) u32 {
                return @intCast(value.bytes.len);
            }
        }.run);
    }
    var source = fx.OwnedSlice(P){ .allocator = env.allocator, .items = programs };
    var described = fx.dsl.allOwned(&env, &source);
    for (0..2) |_| {
        var result = fx.run(&env, described);
        defer result.deinit();
        try std.testing.expect(result == .success);
        try std.testing.expectEqualSlices(u32, &.{ 10, 10 }, result.success.items);
        try std.testing.expectEqual(@as(usize, 2), live.load(.acquire));
    }
    described.deinit();
    try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
}

test "COLL-04 invalid runtime concurrency rejects before any item callback or provider admission" {
    const Body = struct {
        fn run(_: *const void, _: *const u32, env: *const Env) P {
            _ = env.calls.fetchAdd(1, .acq_rel);
            return P.succeed(1);
        }
    };
    var calls: std.atomic.Value(usize) = .init(0);
    var live: std.atomic.Value(usize) = .init(0);
    const env: Env = .{ .allocator = std.testing.allocator, .calls = &calls, .live = &live };
    const values = [_]u32{ 1, 2 };
    // No initialized executor is supplied: rejected configuration must never
    // select or submit work to the capability.
    var described = fx.dsl.forEachConcurrentCaptured(&env, &values, @as(void, {}), 0, .{ .body = Body.run });
    defer described.deinit();
    var result = fx.run(&env, described);
    defer result.deinit();
    try std.testing.expect(result == .die and result.die.kind == .invalid_configuration);
    try std.testing.expectEqual(@as(usize, 0), calls.load(.acquire));
}

const DrainMode = enum { cancellation, zero_admission, partial_rejection, child_failure };
fn exerciseDrain(comptime mode: DrainMode) !void {
    const Out = fx.Effect(fx.OwnedSlice(u32), E, Env);
    const InputList = struct {
        pub const effect_owned_payload = true;
        allocator: std.mem.Allocator,
        items: []u32,
        active: *std.atomic.Value(bool),
        pub fn deinit(self: *@This()) void {
            self.active.store(false, .release);
            self.allocator.free(self.items);
        }
    };
    const ItemOwner = struct {
        pub const effect_owned_payload = true;
        payload: *const Payload,
        item: *const u32,
        active: *std.atomic.Value(bool),
        items_active: *std.atomic.Value(bool),
        invalid: *std.atomic.Value(bool),
        completed: *std.atomic.Value(usize),
        pub fn deinit(self: *@This()) void {
            if (!self.active.load(.acquire) or !self.items_active.load(.acquire)) {
                self.invalid.store(true, .release);
            } else if (!std.mem.eql(u8, self.payload.bytes, "pending collection") or self.item.* > 1) self.invalid.store(true, .release);
            _ = self.completed.fetchAdd(1, .acq_rel);
        }
        fn pending(self: *const @This(), env: *const Env) P {
            if (env.fail_child and self.item.* == 1) return P.fail(.expected);
            return P.fromAsyncBorrowed(u32, self.item, struct {
                fn start(value: *const u32, current: *const Env) fx.Async(u32, E).Start {
                    return current.pending_gates.?[value.*].start();
                }
            }.start);
        }
    };
    const Factory = struct {
        fn body(stored: *const Payload, item: *const u32, env: *const Env) P {
            return fx.dsl.owned(env, ItemOwner{ .payload = stored, .item = item, .active = env.capture_live.?, .items_active = env.items_live.?, .invalid = env.invalid.?, .completed = env.calls }, ItemOwner.pending);
        }
        fn make(env: *const Env) Out {
            const bytes = env.allocator.dupe(u8, "pending collection") catch return Out.die(fx.Defect.out_of_memory);
            _ = env.live.fetchAdd(1, .acq_rel);
            const values = env.allocator.dupe(u32, &.{ 0, 1 }) catch {
                var abandoned = Payload{ .allocator = env.allocator, .bytes = bytes, .live = env.live, .capture_live = env.capture_live };
                abandoned.deinit();
                return Out.die(fx.Defect.out_of_memory);
            };
            return fx.dsl.forEachConcurrentOwned(env, InputList{ .allocator = env.allocator, .items = values, .active = env.items_live.? }, Payload{ .allocator = env.allocator, .bytes = bytes, .live = env.live, .capture_live = env.capture_live }, 2, .{ .body = body });
        }
    };
    {
        const controlled = mode != .cancellation;
        var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = true }, .maximum_owned = 4 };
        defer engine.deinit();
        var calls: std.atomic.Value(usize) = .init(0);
        var live: std.atomic.Value(usize) = .init(0);
        var active: std.atomic.Value(bool) = .init(true);
        var items_active: std.atomic.Value(bool) = .init(true);
        var invalid: std.atomic.Value(bool) = .init(false);
        var pending_gates = [_]gates.Gate(u32, E){ .{ .result = .{ .success = 1 } }, .{ .result = .{ .success = 2 } } };
        var provider: Controlled = .{ .base = .{ .allocator = std.testing.allocator, .capacity = 4, .capabilities = .{ .independent = true, .cpu = true, .blocking = true } }, .io = std.testing.io, .first_started = &pending_gates[0].started, .capture_active = &active, .items_active = &items_active, .invalid = &invalid, .reject_second_worker = mode == .partial_rejection, .reject_first_worker = mode == .zero_admission };
        defer provider.deinit();
        const env: Env = .{ .allocator = std.testing.allocator, .async_engine = if (controlled) provider.executor() else engine.executor(), .calls = &calls, .live = &live, .capture_live = &active, .pending_gates = &pending_gates, .invalid = &invalid, .items_live = &items_active, .fail_child = mode == .child_failure };
        var pending = fx.runAsync(&env, comptime Out.deferEffect(Factory.make));
        defer pending.deinit();
        if (mode == .cancellation) {
            for (&pending_gates) |*gate| gate.started.waitUncancelable(env.io);
            try std.testing.expectEqual(@as(usize, 1), live.load(.acquire));
            pending.request();
        }
        var result = pending.join();
        defer result.deinit();
        switch (mode) {
            .cancellation => try std.testing.expect(result == .interrupted),
            .zero_admission => try std.testing.expect(result == .die and result.die.kind == .provider_unavailable),
            .partial_rejection => {
                try std.testing.expect(result == .die and result.die.kind == .provider_unavailable);
                try std.testing.expectEqualStrings("controlled second-worker rejection", result.die.message);
            },
            .child_failure => try std.testing.expect(result == .fail and result.fail == .expected),
        }
        const completed: usize = switch (mode) {
            .zero_admission => 0,
            .partial_rejection => 1,
            else => 2,
        };
        try std.testing.expectEqual(completed, calls.load(.acquire));
        try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
        try std.testing.expect(!active.load(.acquire));
        try std.testing.expect(!items_active.load(.acquire));
        try std.testing.expect(!invalid.load(.acquire));
        if (mode == .cancellation) for (&pending_gates) |*gate| try std.testing.expect(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
        if (controlled) {
            if (mode != .zero_admission) {
                try std.testing.expect(pending_gates[0].cancelled.load(.acquire));
                try std.testing.expect(pending_gates[0].joined.load(.acquire) and pending_gates[0].destroyed.load(.acquire));
                try std.testing.expect(provider.requested.load(.acquire) >= 1);
            }
            try std.testing.expectEqual(completed, provider.accepted.load(.acquire));
            try std.testing.expectEqual(completed, provider.joined.load(.acquire));
            try std.testing.expectEqual(completed, provider.destroyed.load(.acquire));
        }
    }
}

test "COLL-03 COLL-05 concurrent cancellation and provider rejection drain before capture teardown" {
    try exerciseDrain(.cancellation);
    try exerciseDrain(.zero_admission);
}

test "COLL-05 ASYNC-03 partial provider rejection drains an already-started sibling before failure" {
    try exerciseDrain(.partial_rejection);
}

test "COLL-05 ASYNC-03 child failure after sibling admission drains workers before failure" {
    try exerciseDrain(.child_failure);
}

test "COLL-02 COLL-03 factory concurrent traversal creates and releases fresh owning slices each run" {
    const Items = struct {
        pub const effect_owned_payload = true;
        allocator: std.mem.Allocator,
        items: []u32,
        live: *std.atomic.Value(usize),
        pub fn deinit(self: *@This()) void {
            self.allocator.free(self.items);
            _ = self.live.fetchSub(1, .acq_rel);
        }
    };
    const Factory = struct {
        fn acquire(_: *const Inputs, _: *const Env) fx.Effect(Items, E, Env) {
            return fx.Effect(Items, E, Env).fromOutcome(struct {
                fn run(env: *const Env) fx.Exit(Items, E) {
                    const values = env.allocator.dupe(u32, &.{ 4, 2, 3, 1 }) catch return .{ .die = fx.Defect.out_of_memory };
                    _ = env.live.fetchAdd(1, .acq_rel);
                    return .{ .success = .{ .allocator = env.allocator, .items = values, .live = env.live } };
                }
            }.run);
        }
        fn body(stored: *const Inputs, item: *const u32, _: *const Env) P {
            return P.succeed(item.* * stored.factor);
        }
    };
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = true }, .maximum_owned = 4 };
    defer engine.deinit();
    var calls: std.atomic.Value(usize) = .init(0);
    var live: std.atomic.Value(usize) = .init(0);
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor(), .calls = &calls, .live = &live };
    var described = fx.dsl.forEachFactoryConcurrent(&env, Inputs{ .factor = 3 }, 2, .{ .items = Factory.acquire, .body = Factory.body });
    defer described.deinit();
    for (0..2) |_| {
        var result = fx.run(&env, described);
        defer result.deinit();
        try std.testing.expect(result == .success);
        try std.testing.expectEqualSlices(u32, &.{ 12, 6, 9, 3 }, result.success.items);
        try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
    }
}

test "ASYNC-05 erased Effect descriptions preserve retained synchronization evidence before concurrent admission" {
    const Shared = struct {
        references: std.atomic.Value(usize) = .init(1),
        fn retain(self: *@This()) *@This() {
            _ = self.references.fetchAdd(1, .monotonic);
            return self;
        }
        fn release(self: **@This()) void {
            _ = self.*.references.fetchSub(1, .acq_rel);
        }
    };
    inline for (.{ fx.dsl.ownership.Synchronization.single_thread, fx.dsl.ownership.Synchronization.thread_safe }) |synchronization| {
        var shared: Shared = .{};
        var calls: std.atomic.Value(usize) = .init(0);
        var live: std.atomic.Value(usize) = .init(0);
        var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = false, .blocking = false }, .maximum_owned = 2 };
        defer engine.deinit();
        const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor(), .calls = &calls, .live = &live };
        const retained = fx.dsl.ownership.retained(&shared, .{ .retain = Shared.retain, .release = Shared.release, .synchronization = synchronization });
        var values = [_]P{P.fromOwned(&env, retained, struct {
            fn value(_: *const @TypeOf(retained), current: *const Env) fx.Exit(u32, E) {
                _ = current.calls.fetchAdd(1, .monotonic);
                return .{ .success = 17 };
            }
        }.value)};
        var description = fx.dsl.forEachConcurrent(&env, &values, 1, .{ .body = struct {
            fn invoke(value: *const P, _: *const Env) P {
                return value.borrow();
            }
        }.invoke });
        var result = fx.run(&env, description);
        description.deinit();
        defer result.deinit();
        if (comptime synchronization == .single_thread) {
            try std.testing.expect(result == .die and result.die.kind == .invalid_configuration);
            try std.testing.expectEqual(@as(usize, 0), calls.load(.acquire));
        } else {
            try std.testing.expect(result == .success);
            try std.testing.expectEqual(@as(u32, 17), result.success.items[0]);
        }
        values[0].deinit();
        try std.testing.expectEqual(@as(usize, 1), shared.references.load(.acquire));
    }
}
