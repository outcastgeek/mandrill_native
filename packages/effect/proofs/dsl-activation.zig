const std = @import("std");
const fx = @import("effect");
const gates = @import("support/dsl_gate.zig");
const A = fx.ClusterEntityResourcePlan;
const E = enum { invalid_token, stale, unavailable, acquisition, body, release };
const State = struct {
    acquired: usize = 0,
    released: usize = 0,
    order: [4]usize = @splat(0),
    generation: std.atomic.Value(u64) = .init(1),
    fail_second: bool = false,
    fail_body: bool = false,
    fail_release: bool = false,
    disposed: usize = 0,
    gate: ?*gates.Gate(u32, E) = null,
    late: ?*Late = null,
};
const R = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, async_engine: fx.Executor = undefined, state: *State, provider: *const Service };
const Live = struct { allocator: std.mem.Allocator, number: usize, generation: u64 };
const Service = fx.ClusterEntityResource(*Live, E, R);
const Unit = fx.Effect(void, E, R);
const Number = fx.Effect(u32, E, R);
const Handle = fx.ResourceValue(*Live);
const Pair = struct { first: Handle, second: Handle };
const Inputs = struct {
    key: fx.OwnedSlice(u8),
    owner: struct {
        pub const effect_owned_payload = true;
        state: *State,
        pub fn deinit(self: *@This()) void {
            self.state.disposed += 1;
        }
    },
    generation: u64,
};
fn enter(data: ?*anyopaque, selected_token: A.Token, _: fx.WallInstant) A.Fence.Enter {
    const state: *State = @ptrCast(@alignCast(data.?));
    if (selected_token.generation != state.generation.load(.acquire)) return .stale;
    return .{ .held = .{ .userdata = null, .until = .never, .release_fn = struct {
        fn release(_: ?*anyopaque) void {}
    }.release } };
}
const Callbacks = struct {
    fn select(env: *const R) *const Service {
        return env.provider;
    }
    fn acquisition(_: ?*anyopaque, env: *const R, selected_token: A.Token) Service.AcquireEffect {
        return Service.AcquireEffect.fromOwned(env, selected_token, struct {
            fn allocate(value: *const A.Token, current: *const R) fx.Exit(*Live, E) {
                std.debug.assert(current.state.disposed == 0);
                if (current.state.fail_second and current.state.acquired == 1) return .{ .fail = .acquisition };
                const resource = current.allocator.create(Live) catch return .{ .die = fx.Defect.out_of_memory };
                current.state.acquired += 1;
                resource.* = .{ .allocator = current.allocator, .number = current.state.acquired, .generation = value.generation };
                return .{ .success = resource };
            }
        }.allocate);
    }
    fn release(_: ?*anyopaque, _: *const R, resource: *Live, primary: fx.PrimaryOutcome(E)) Unit {
        _ = primary;
        return Unit.fromBorrowed(Live, resource, struct {
            fn destroy(value: *const Live, current: *const R) fx.Exit(void, E) {
                std.debug.assert(current.state.disposed == 0);
                if (current.state.gate) |gate| std.debug.assert(gate.destroyed.load(.acquire));
                if (current.state.late) |late| std.debug.assert(late.destroyed.load(.acquire));
                current.state.order[current.state.released] = value.number;
                current.state.released += 1;
                value.allocator.destroy(@constCast(value));
                return if (current.state.fail_release) .{ .fail = .release } else .{ .success = {} };
            }
        }.destroy);
    }
    fn use(pair: *const Pair, selected_token: A.Token, env: *const R) Number {
        std.debug.assert(pair.first.value.generation == selected_token.generation and pair.second.value.generation == selected_token.generation);
        if (env.state.fail_body) return Number.fail(.body);
        if (env.state.gate != null) return Number.fromAsync(struct {
            fn start(current: *const R) fx.Async(u32, E).Start {
                return current.state.gate.?.start();
            }
        }.start);
        return Number.fromBorrowed(Pair, pair, struct {
            fn run(values: *const Pair, _: *const R) fx.Exit(u32, E) {
                return .{ .success = @intCast(values.first.value.number + values.second.value.number) };
            }
        }.run);
    }
    fn second(first: Handle, _: *const R) @TypeOf(A.bindCaptured(A.acquire(select), first, third)) {
        return A.bindCaptured(A.acquire(select), first, third);
    }
    fn third(first: *const Handle, second_handle: Handle, _: *const R) @TypeOf(A.use(Pair{ .first = first.*, .second = second_handle }, use)) {
        return A.use(Pair{ .first = first.*, .second = second_handle }, use);
    }
    fn token(inputs: *const Inputs, _: *const R) A.Token {
        return .{ .address = .{ .group = "entities", .entity_key = inputs.key.items }, .owner_id = "owner", .generation = inputs.generation };
    }
    fn fence(_: *const Inputs, env: *const R) A.Fence {
        return .{ .userdata = env.state, .enter = enter };
    }
    fn now(_: *const R) fx.WallInstant {
        return .{ .unix_nanoseconds = 0 };
    }
    fn mapError(err: A.Error) E {
        return switch (err) {
            .invalid_token => .invalid_token,
            .stale => .stale,
            .unavailable => .unavailable,
        };
    }
};
const plan = A.bind(A.acquire(Callbacks.select), Callbacks.second);
fn makeInputs(env: *const R, generation: u64) error{OutOfMemory}!Inputs {
    const key = fx.PrimaryKey.encodeDirect(env, "entity", &.{.{ .text = "one" }});
    if (key == .die and key.die.kind == .out_of_memory) return error.OutOfMemory;
    std.debug.assert(key == .success);
    return .{ .key = key.success, .owner = .{ .state = env.state }, .generation = generation };
}
fn described(env: *const R, values: Inputs) Number {
    return A.run(env, values, plan, .{ .token = Callbacks.token, .fence = Callbacks.fence, .now = Callbacks.now, .map_error = Callbacks.mapError });
}
const vtable: Service.VTable = .{ .acquire = Callbacks.acquisition, .release = Callbacks.release };
fn check(allocator: std.mem.Allocator) !void {
    var state: State = .{};
    const provider = Service.fromProvider(null, &vtable);
    const env: R = .{ .allocator = allocator, .state = &state, .provider = &provider };
    var program = described(&env, try makeInputs(&env, 1));
    var result = fx.run(&env, program);
    program.deinit();
    defer result.deinit();
    try std.testing.expectEqual(@as(usize, 1), state.disposed);
    try std.testing.expectEqual(state.acquired, state.released);
    if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
    try std.testing.expectEqual(@as(u32, 3), result.success);
    try std.testing.expectEqualSlices(usize, &.{ 2, 1 }, state.order[0..2]);
}
test "ENTITY-01 ENTITY-04 COMP-02 activation plans retain one token through nested resources and every allocation failure" {
    try check(std.testing.allocator);
    try std.testing.checkAllAllocationFailures(std.testing.allocator, check, .{});
}
test "ENTITY-04 ENTITY-05 later acquisition and body failures unwind in reverse and retain multiple cleanup failures" {
    inline for (.{ false, true }) |second_fails| {
        var state: State = .{ .fail_second = second_fails, .fail_body = !second_fails, .fail_release = true };
        const provider = Service.fromProvider(null, &vtable);
        const env: R = .{ .allocator = std.testing.allocator, .state = &state, .provider = &provider };
        var program = described(&env, try makeInputs(&env, 1));
        defer program.deinit();
        var result = fx.run(&env, program);
        defer result.deinit();
        try std.testing.expect(result == .finalization_failed);
        try std.testing.expectEqual(if (second_fails) E.acquisition else E.body, result.finalization_failed.primary.failed);
        try std.testing.expectEqual(E.release, result.finalization_failed.first.release_failure);
        if (!second_fails) try std.testing.expect(result.finalization_failed.rest != null);
        try std.testing.expectEqual(state.acquired, state.released);
    }
}
test "ENTITY-03 stale generations reject operations and a fresh activation reacquires resources" {
    var state: State = .{};
    const provider = Service.fromProvider(null, &vtable);
    const env: R = .{ .allocator = std.testing.allocator, .state = &state, .provider = &provider };
    var old = described(&env, try makeInputs(&env, 1));
    state.generation.store(2, .release);
    var stale = fx.run(&env, old);
    old.deinit();
    defer stale.deinit();
    try std.testing.expect(stale == .fail and stale.fail == .stale);
    try std.testing.expectEqual(@as(usize, 0), state.acquired);
    state.disposed = 0;
    var fresh = described(&env, try makeInputs(&env, 2));
    var current = fx.run(&env, fresh);
    fresh.deinit();
    defer current.deinit();
    try std.testing.expect(current == .success);
    try std.testing.expectEqual(@as(usize, 2), state.acquired);
    try std.testing.expectEqual(state.acquired, state.released);
}
test "ENTITY-02 cancellation retains the activation owner through suspended dispatch drain and resource release" {
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = true }, .maximum_owned = 4 };
    defer engine.deinit();
    var gate: gates.Gate(u32, E) = .{ .result = .{ .success = 3 } };
    var state: State = .{ .gate = &gate };
    const provider = Service.fromProvider(null, &vtable);
    const env: R = .{ .allocator = std.testing.allocator, .state = &state, .provider = &provider, .async_engine = engine.executor() };
    var program = described(&env, try makeInputs(&env, 1));
    var pending = fx.runAsync(&env, program);
    defer pending.deinit();
    gate.started.waitUncancelable(env.io);
    try std.testing.expectEqual(@as(usize, 0), state.disposed);
    try std.testing.expectEqual(@as(usize, 0), state.released);
    pending.request();
    var result = pending.join();
    program.deinit();
    defer result.deinit();
    try std.testing.expect(result == .interrupted);
    try std.testing.expect(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
    try std.testing.expectEqual(@as(usize, 2), state.released);
    try std.testing.expectEqual(@as(usize, 1), state.disposed);
}

test "ENTITY-01 ENTITY-03 activation branches reuse configuration while reacquiring independent live resources" {
    const Selection = struct {
        fn predicate(value: *const bool, _: A.Token, _: *const R) fx.Effect(bool, E, R) {
            return fx.Effect(bool, E, R).succeed(value.*);
        }
        fn unused(_: *const void, _: A.Token, _: *const R) Number {
            return Number.fail(.body);
        }
    };
    const branched = A.branch(A.use(true, Selection.predicate), plan, A.use(@as(void, {}), Selection.unused));
    var state: State = .{};
    const provider = Service.fromProvider(null, &vtable);
    const env: R = .{ .allocator = std.testing.allocator, .state = &state, .provider = &provider };
    var program = A.run(&env, try makeInputs(&env, 1), branched, .{ .token = Callbacks.token, .fence = Callbacks.fence, .now = Callbacks.now, .map_error = Callbacks.mapError });
    for (0..2) |n| {
        var result = fx.run(&env, program);
        defer result.deinit();
        try std.testing.expectEqual(@as(u32, @intCast(3 + n * 4)), result.success);
        try std.testing.expectEqual(@as(usize, 0), state.disposed);
    }
    program.deinit();
    try std.testing.expectEqual(@as(usize, 1), state.disposed);
    try std.testing.expectEqualSlices(usize, &.{ 2, 1, 4, 3 }, &state.order);
}

const Late = struct {
    const Native = fx.Async(*Live, E);
    io: std.Io = std.testing.io,
    started: std.Io.Event = .unset,
    proceed: std.Io.Event = .unset,
    destroyed: std.atomic.Value(bool) = .init(false),
    joined: std.atomic.Value(bool) = .init(false),
    resource: *Live = undefined,
    fn acquire(_: ?*anyopaque, _: *const R, _: A.Token) Service.AcquireEffect {
        return Service.AcquireEffect.fromAsync(struct {
            fn start(env: *const R) Native.Start {
                const resource = env.allocator.create(Live) catch return .{ .completed = .{ .die = fx.Defect.out_of_memory } };
                env.state.acquired += 1;
                resource.* = .{ .allocator = env.allocator, .number = env.state.acquired, .generation = env.state.generation.load(.acquire) };
                const gate = env.state.late.?;
                gate.resource = resource;
                gate.started.set(env.io);
                return .{ .pending = .{ .userdata = gate, .vtable = &native_vtable } };
            }
        }.start);
    }
    fn request(data: *anyopaque) ?fx.Defect {
        const late: *Late = @ptrCast(@alignCast(data));
        late.proceed.set(late.io);
        return null;
    }
    fn join(data: *anyopaque, _: bool) Native.JoinResult {
        const late: *Late = @ptrCast(@alignCast(data));
        late.proceed.waitUncancelable(late.io);
        late.joined.store(true, .release);
        return .{ .exit = .{ .success = late.resource } };
    }
    fn destroy(data: *anyopaque) void {
        const late: *Late = @ptrCast(@alignCast(data));
        std.debug.assert(late.joined.load(.acquire));
        late.destroyed.store(true, .release);
    }
    const native_vtable: Native.Operation.VTable = .{ .request = request, .join = join, .destroy = destroy };
};
test "ENTITY-04 late successful acquisition after cancellation releases once before the activation owner" {
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = true }, .maximum_owned = 4 };
    defer engine.deinit();
    var late: Late = .{};
    var state: State = .{ .late = &late };
    const late_vtable: Service.VTable = .{ .acquire = Late.acquire, .release = Callbacks.release };
    const provider = Service.fromProvider(null, &late_vtable);
    const env: R = .{ .allocator = std.testing.allocator, .state = &state, .provider = &provider, .async_engine = engine.executor() };
    var program = described(&env, try makeInputs(&env, 1));
    var pending = fx.runAsync(&env, program);
    defer pending.deinit();
    late.started.waitUncancelable(env.io);
    pending.request();
    var result = pending.join();
    program.deinit();
    defer result.deinit();
    try std.testing.expect(result == .interrupted);
    try std.testing.expect(late.joined.load(.acquire) and late.destroyed.load(.acquire));
    try std.testing.expectEqual(@as(usize, 1), state.acquired);
    try std.testing.expectEqual(@as(usize, 1), state.released);
    try std.testing.expectEqual(@as(usize, 1), state.disposed);
}

test "ENTITY-01 DECL-01 INF-08 entity roles infer the completed native handler and registry identity" {
    const Env = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, request: i64 };
    const Handlers = struct {
        fn body(value: *const i64, _: *const Env) fx.Effect(i64, void, Env) {
            return fx.Effect(i64, void, Env).succeed(value.* + 1);
        }
        fn request(env: *const Env) *const i64 {
            return &env.request;
        }
    };
    const integer = fx.JsonCodec.signed_integer;
    const nothing = fx.JsonCodec.null_value;
    const handler = comptime fx.dsl.entity(.{ .group = "accounts", .version = 3, .request = integer, .success = integer, .failure = nothing, .handler = Handlers.body });
    const previous = fx.ClusterEntityBuilder(void, Env).begin("accounts", 3).request(integer).success(integer).failure(nothing).handledBy(Handlers.body);
    comptime if (@TypeOf(handler) != @TypeOf(previous)) @compileError("named entity must retain native handler identity");
    const Registry = fx.ClusterEntityRegistry(.{handler});
    try std.testing.expectEqual(@as(usize, 0), Registry.resolve("accounts", 3).ok);
    const env: Env = .{ .allocator = std.testing.allocator, .request = 21 };
    const program = comptime handler.invoke(Handlers.request);
    comptime if (@TypeOf(program) != fx.Effect(i64, void, Env)) @compileError("entity invocation must remain exact Effect");
    var result = fx.run(&env, program);
    defer result.deinit();
    try std.testing.expectEqual(@as(i64, 22), result.success);
}
