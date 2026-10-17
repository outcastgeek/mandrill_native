const std = @import("std");
const fx = @import("effect");
const gates = @import("support/dsl_gate.zig");
const E = enum { expected };
const Mode = enum { success, retry_once, fail, die, interrupted };
const Env = struct {
    allocator: std.mem.Allocator,
    io: std.Io = std.testing.io,
    async_engine: fx.Executor = undefined,
    attempts: *usize,
    initialized: *usize,
    live: *usize,
    cancellation: *fx.Cancellation,
    recurrences: usize = 1,
    mode: Mode = .success,
    fail_initialization: bool = false,
    cancel_initialization: bool = false,
    gate: ?*gates.Gate(void, E) = null,
};
const P = fx.Effect(u32, E, Env);
const Leaf = struct {
    fn run(env: *const Env) fx.Exit(u32, E) {
        env.attempts.* += 1;
        return switch (env.mode) {
            .success => .{ .success = @intCast(env.attempts.*) },
            .retry_once => if (env.attempts.* == 1) .{ .fail = .expected } else .{ .success = @intCast(env.attempts.*) },
            .fail => .{ .fail = .expected },
            .die => .{ .die = fx.Defect.init("scheduled body defect") },
            .interrupted => .interrupted,
        };
    }
};
const CounterPolicy = struct {
    fn initialize(env: anytype) usize {
        env.initialized.* += 1;
        return 0;
    }
    fn step(state: *usize, _: anytype, _: std.Io.Duration, _: f64, env: anytype) fx.Schedule.Decision(void) {
        if (state.* == env.recurrences) return .{ .stop = {} };
        state.* += 1;
        return .{ .continue_with = .{ .output = {}, .delay = .zero } };
    }
};
const counter_roles = .{ .initialize = CounterPolicy.initialize, .step = CounterPolicy.step, .random = false };

test "FAC-01 INF-10 named retry and repeat infer selected input channels with fresh native state" {
    const retry = comptime fx.dsl.retry(P.fromOutcome(Leaf.run), counter_roles);
    const repeat = comptime fx.dsl.repeat(P.fromOutcome(Leaf.run), counter_roles);
    comptime if (@TypeOf(retry) != P or @TypeOf(repeat) != P) @compileError("scheduled operations must return exact Effect");
    var attempts: usize = 0;
    var initialized: usize = 0;
    var live: usize = 0;
    var cancellation: fx.Cancellation = .{};
    var env: Env = .{ .allocator = std.testing.allocator, .attempts = &attempts, .initialized = &initialized, .live = &live, .cancellation = &cancellation, .mode = .retry_once };
    try std.testing.expectEqual(@as(usize, 0), initialized);
    for (0..2) |_| {
        attempts = 0;
        try std.testing.expectEqual(@as(u32, 2), fx.run(&env, retry).success);
    }
    env.mode = .success;
    attempts = 0;
    try std.testing.expectEqual(@as(u32, 2), fx.run(&env, repeat).success);
    try std.testing.expectEqual(@as(usize, 3), initialized);
}

const Owned = struct {
    pub const effect_owned_payload = true;
    allocator: std.mem.Allocator,
    bytes: []u8,
    live: *usize,
    pub fn deinit(self: *@This()) void {
        self.allocator.free(self.bytes);
        self.live.* -= 1;
    }
};
// The application state needs no repeated outer ownership marker or destructor.
const State = struct { owned: Owned, index: usize = 0 };
const OwningPolicy = struct {
    fn initialize(env: *const Env) fx.Exit(State, E) {
        env.initialized.* += 1;
        if (env.fail_initialization) return .{ .fail = .expected };
        const bytes = env.allocator.dupe(u8, "policy state") catch return .{ .die = fx.Defect.out_of_memory };
        env.live.* += 1;
        if (env.cancel_initialization) env.cancellation.request();
        return .{ .success = .{ .owned = .{ .allocator = env.allocator, .bytes = bytes, .live = env.live } } };
    }
    fn effectful(_: *const Env) fx.Effect(State, E, Env) {
        return fx.Effect(State, E, Env).fromOutcome(initialize);
    }
    fn pending(_: *const Env) fx.Effect(State, E, Env) {
        return (comptime fx.Effect(void, E, Env).fromAsync(struct {
            fn start(env: *const Env) fx.Async(void, E).Start {
                return env.gate.?.start();
            }
        }.start)).flatMap(struct {
            fn initialized(_: void) fx.Effect(State, E, Env) {
                return fx.Effect(State, E, Env).fromOutcome(initialize);
            }
        }.initialized);
    }
    fn step(state: *State, _: anytype, _: std.Io.Duration, _: f64, env: *const Env) fx.Schedule.Decision(void) {
        if (env.live.* != 1) return .{ .invalid = fx.Defect.init("schedule state ended before its transition") };
        if (!std.mem.eql(u8, state.owned.bytes, "policy state")) return .{ .invalid = fx.Defect.init("schedule state corrupted") };
        if (state.index == env.recurrences) return .{ .stop = {} };
        state.index += 1;
        return .{ .continue_with = .{ .output = {}, .delay = .zero } };
    }
};

test "FAC-01 LIFE-07 owning schedule state initializes natively or effectfully and unwinds every allocation failure" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            inline for (.{ false, true }) |effectful| inline for (.{ false, true }) |repeating| {
                for (std.meta.tags(Mode)) |mode| {
                    var attempts: usize = 0;
                    var initialized: usize = 0;
                    var live: usize = 0;
                    var cancellation: fx.Cancellation = .{};
                    const env: Env = .{ .allocator = allocator, .attempts = &attempts, .initialized = &initialized, .live = &live, .cancellation = &cancellation, .mode = mode };
                    const described = comptime if (repeating)
                        fx.dsl.repeat(P.fromOutcome(Leaf.run), .{ .initialize = if (effectful) OwningPolicy.effectful else OwningPolicy.initialize, .step = OwningPolicy.step, .random = false })
                    else
                        fx.dsl.retry(P.fromOutcome(Leaf.run), .{ .initialize = if (effectful) OwningPolicy.effectful else OwningPolicy.initialize, .step = OwningPolicy.step, .random = false });
                    var result = fx.run(&env, described);
                    defer result.deinit();
                    try std.testing.expectEqual(@as(usize, 0), live);
                    if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
                    switch (mode) {
                        .success => try std.testing.expect(result == .success),
                        .retry_once => try std.testing.expect(if (repeating) result == .fail and result.fail == .expected else result == .success),
                        .fail => try std.testing.expect(result == .fail and result.fail == .expected),
                        .die => try std.testing.expect(result == .die),
                        .interrupted => try std.testing.expect(result == .interrupted),
                    }
                    try std.testing.expectEqual(@as(usize, 1), initialized);
                }
            };
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

test "FAC-01 LIFE-02 initialization failure and late cancellation skip source work and release installed state" {
    inline for (.{ false, true }) |effectful| {
        inline for (.{ false, true }) |cancel| {
            var attempts: usize = 0;
            var initialized: usize = 0;
            var live: usize = 0;
            var cancellation: fx.Cancellation = .{};
            const env: Env = .{ .allocator = std.testing.allocator, .attempts = &attempts, .initialized = &initialized, .live = &live, .cancellation = &cancellation, .fail_initialization = !cancel, .cancel_initialization = cancel };
            var result = fx.run(&env, comptime fx.dsl.retry(P.fromOutcome(Leaf.run), .{ .initialize = if (effectful) OwningPolicy.effectful else OwningPolicy.initialize, .step = OwningPolicy.step, .random = false }));
            defer result.deinit();
            try std.testing.expect(if (cancel) result == .interrupted else result == .fail and result.fail == .expected);
            try std.testing.expectEqual(@as(usize, 0), attempts);
            try std.testing.expectEqual(@as(usize, 0), live);
        }
    }
}

test "FAC-01 suspended initialization resumes or cancels before schedule state and source execution" {
    inline for (.{ false, true }) |cancel| {
        var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = true }, .maximum_owned = 2 };
        defer engine.deinit();
        var attempts: usize = 0;
        var initialized: usize = 0;
        var live: usize = 0;
        var cancellation: fx.Cancellation = .{};
        var gate: gates.Gate(void, E) = .{ .result = .{ .success = {} } };
        const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor(), .attempts = &attempts, .initialized = &initialized, .live = &live, .cancellation = &cancellation, .gate = &gate };
        const described = comptime fx.dsl.retry(P.fromOutcome(Leaf.run), .{ .initialize = OwningPolicy.pending, .step = OwningPolicy.step, .random = false });
        var pending = fx.runAsync(&env, described);
        defer pending.deinit();
        gate.started.waitUncancelable(env.io);
        try std.testing.expectEqual(@as(usize, 0), attempts);
        try std.testing.expectEqual(@as(usize, 0), initialized);
        if (cancel) pending.request() else gate.proceed.set(env.io);
        var result = pending.join();
        defer result.deinit();
        try std.testing.expect(if (cancel) result == .interrupted else result == .success);
        try std.testing.expectEqual(@as(usize, if (cancel) 0 else 1), attempts);
        try std.testing.expectEqual(@as(usize, 0), live);
        try std.testing.expect(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
    }
}

test "FAC-01 CAP-11 runtime-owned projected source retains its originating allocator and I/O services" {
    const Roles = struct {
        fn initialize(_: *const void) usize {
            return 0;
        }
        fn step(state: *usize, _: E, _: std.Io.Duration, _: f64) fx.Schedule.Decision(void) {
            if (state.* == 1) return .{ .stop = {} };
            state.* += 1;
            return .{ .continue_with = .{ .output = {}, .delay = .zero } };
        }
    };
    var tracking = std.testing.FailingAllocator.init(std.testing.allocator, .{});
    var attempts: usize = 0;
    var initialized: usize = 0;
    var live: usize = 0;
    var cancellation: fx.Cancellation = .{};
    const env: Env = .{ .allocator = tracking.allocator(), .attempts = &attempts, .initialized = &initialized, .live = &live, .cancellation = &cancellation, .mode = .retry_once };
    var source = (comptime P.fromOutcome(Leaf.run)).provide(&env);
    var described = fx.dsl.retryOwned(&source, &{}, .{ .initialize = Roles.initialize, .step = Roles.step, .random = false });
    var result = fx.run(&{}, described);
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(u32, 2), result.success);
    result.deinit();
    described.deinit();
    try std.testing.expect(tracking.alloc_index > 0);
    try std.testing.expectEqual(tracking.allocated_bytes, tracking.freed_bytes);
}

test "FAC-01 explicit randomness uses only the selected provider and repeat success channel" {
    const Runtime = struct { allocator: std.mem.Allocator, io: std.Io, base: Env, random: fx.Random };
    const Program = fx.Effect(u32, E, Runtime);
    const Roles = struct {
        fn initialize(env: *const Runtime) usize {
            return CounterPolicy.initialize(&env.base);
        }
        fn step(state: *usize, input: u32, elapsed: std.Io.Duration, sample: f64, env: *const Runtime) fx.Schedule.Decision(void) {
            if (sample != 0.25) return .{ .invalid = fx.Defect.init("schedule ignored selected random provider") };
            return CounterPolicy.step(state, input, elapsed, sample, &env.base);
        }
        fn draw(data: ?*anyopaque) f64 {
            const count: *usize = @ptrCast(@alignCast(data.?));
            count.* += 1;
            return 0.25;
        }
    };
    var attempts: usize = 0;
    var initialized: usize = 0;
    var live: usize = 0;
    var cancellation: fx.Cancellation = .{};
    var samples: usize = 0;
    const env: Runtime = .{ .allocator = std.testing.allocator, .io = std.testing.io, .base = .{ .allocator = std.testing.allocator, .attempts = &attempts, .initialized = &initialized, .live = &live, .cancellation = &cancellation }, .random = .{ .userdata = &samples, .sample_fn = Roles.draw } };
    const described = comptime fx.dsl.repeat(Program.fromOutcome(struct {
        fn run(current: *const Runtime) fx.Exit(u32, E) {
            return Leaf.run(&current.base);
        }
    }.run), .{ .initialize = Roles.initialize, .step = Roles.step, .random = true });
    var result = fx.run(&env, described);
    defer result.deinit();
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(u32, 2), result.success);
    try std.testing.expectEqual(@as(usize, 2), samples);
    try std.testing.expectEqual(@as(usize, 1), initialized);
}

test "FAC-01 LIFE-07 missing clock capability rejects before constructing owning policy state" {
    const Runtime = struct { allocator: std.mem.Allocator, live: *usize, initialized: *usize };
    const Program = fx.Effect(u32, E, Runtime);
    const Roles = struct {
        fn initialize(env: *const Runtime) fx.Exit(State, E) {
            env.initialized.* += 1;
            const bytes = env.allocator.dupe(u8, "unused") catch return .{ .die = fx.Defect.out_of_memory };
            env.live.* += 1;
            return .{ .success = .{ .owned = .{ .allocator = env.allocator, .bytes = bytes, .live = env.live } } };
        }
        fn step(_: *State, _: E, _: std.Io.Duration, _: f64) fx.Schedule.Decision(void) {
            return .{ .stop = {} };
        }
    };
    var live: usize = 0;
    var initialized: usize = 0;
    const env: Runtime = .{ .allocator = std.testing.allocator, .live = &live, .initialized = &initialized };
    var result = fx.run(&env, comptime fx.dsl.retry(Program.succeed(1), .{ .initialize = Roles.initialize, .step = Roles.step, .random = false }));
    defer result.deinit();
    try std.testing.expect(result == .die and result.die.kind == .provider_unavailable);
    try std.testing.expectEqual(@as(usize, 0), live);
    try std.testing.expectEqual(@as(usize, 0), initialized);
}

test "FAC-01 LIFE-07 schedule composition releases owning state and tuple outputs through every allocation failure" {
    const Roles = struct {
        fn step(state: *State, input: E, elapsed: std.Io.Duration, sample: f64, env: *const Env) fx.Schedule.Decision(Owned) {
            const next = OwningPolicy.step(state, input, elapsed, sample, env);
            if (next == .invalid) return .{ .invalid = next.invalid };
            const bytes = env.allocator.dupe(u8, "policy output") catch return .{ .invalid = fx.Defect.out_of_memory };
            env.live.* += 1;
            const output: Owned = .{ .allocator = env.allocator, .bytes = bytes, .live = env.live };
            return if (next == .stop) .{ .stop = output } else .{ .continue_with = .{ .output = output, .delay = next.continue_with.delay } };
        }
        fn compose(policy: anytype, env: *const Env) @TypeOf(fx.Schedule.intersect(fx.Schedule.upTo(.{ .nanoseconds = std.math.maxInt(i96) }, policy), fx.Schedule.recurs(@TypeOf(policy).Input, env.recurrences))) {
            return fx.Schedule.intersect(fx.Schedule.upTo(.{ .nanoseconds = std.math.maxInt(i96) }, policy), fx.Schedule.recurs(@TypeOf(policy).Input, env.recurrences));
        }
    };
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            var attempts: usize = 0;
            var initialized: usize = 0;
            var live: usize = 0;
            var cancellation: fx.Cancellation = .{};
            const env: Env = .{ .allocator = allocator, .attempts = &attempts, .initialized = &initialized, .live = &live, .cancellation = &cancellation, .mode = .fail };
            var result = fx.run(&env, comptime fx.dsl.retry(P.fromOutcome(Leaf.run), .{ .initialize = OwningPolicy.initialize, .step = Roles.step, .random = false, .compose = Roles.compose }));
            defer result.deinit();
            try std.testing.expectEqual(@as(usize, 0), live);
            if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expect(result == .fail and result.fail == .expected);
            try std.testing.expectEqual(@as(usize, 2), attempts);
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

test "CAP-11 LEAF-07 timeout selects runtime duration once and drains source or timer before publication" {
    const Clock = struct {
        manual: fx.ManualClock,
        duration: std.Io.Duration = .zero,
        sleeps: std.atomic.Value(usize) = .init(0),
        fn service(self: *@This()) fx.Clock {
            return .{ .userdata = self, .vtable = &.{ .now = now, .sleep = sleep } };
        }
        fn now(data: ?*anyopaque) std.Io.Timestamp {
            const self: *@This() = @ptrCast(@alignCast(data.?));
            return self.manual.service().now();
        }
        fn sleep(data: ?*anyopaque, duration: std.Io.Duration) fx.Clock.SleepResult {
            const self: *@This() = @ptrCast(@alignCast(data.?));
            self.duration = duration;
            _ = self.sleeps.fetchAdd(1, .acq_rel);
            return self.manual.service().sleep(duration);
        }
    };
    const Runtime = struct { allocator: std.mem.Allocator, io: std.Io, async_engine: fx.Executor, clock: fx.Clock, selections: *std.atomic.Value(usize), starts: *std.atomic.Value(usize), gate: *gates.Gate(u32, E) };
    const Program = fx.Effect(u32, E, Runtime);
    const Roles = struct {
        fn duration(env: *const Runtime) std.Io.Duration {
            return .fromSeconds(if (env.selections.fetchAdd(1, .acq_rel) == 0) 1 else 99);
        }
        fn timedOut() E {
            return .expected;
        }
        fn source(env: *const Runtime) fx.Async(u32, E).Start {
            _ = env.starts.fetchAdd(1, .acq_rel);
            return env.gate.start();
        }
    };
    const Completion = enum { expires, cancellation, source_wins, provider_rejection };
    for (std.meta.tags(Completion)) |mode| {
        var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = mode != .provider_rejection, .cpu = false, .blocking = false }, .maximum_owned = 4 };
        defer engine.deinit();
        var clock: Clock = .{ .manual = fx.ManualClock.init(.{ .io = std.testing.io }, .{ .nanoseconds = 0 }) };
        defer clock.manual.deinit();
        var selections: std.atomic.Value(usize) = .init(0);
        var starts: std.atomic.Value(usize) = .init(0);
        var gate: gates.Gate(u32, E) = .{ .result = .{ .success = 7 } };
        const env: Runtime = .{ .allocator = std.testing.allocator, .io = std.testing.io, .async_engine = engine.executor(), .clock = clock.service(), .selections = &selections, .starts = &starts, .gate = &gate };
        const described = comptime fx.dsl.timeout(Program.fromAsync(Roles.source), .{ .duration = Roles.duration, .on_timeout = Roles.timedOut });
        comptime if (@TypeOf(described) != Program) @compileError("timeout must return exact Effect");
        var pending = fx.runAsync(&env, described);
        defer pending.deinit();
        if (mode != .provider_rejection) {
            gate.started.waitUncancelable(env.io);
            var admitted = clock.manual.awaitPending(1);
            admitted.deinit();
            try std.testing.expectEqual(@as(usize, 1), selections.load(.acquire));
            try std.testing.expectEqual(std.Io.Duration.fromSeconds(1).nanoseconds, clock.duration.nanoseconds);
            switch (mode) {
                .expires => {
                    var advanced = clock.manual.advance(.fromSeconds(1));
                    advanced.deinit();
                },
                .cancellation => pending.request(),
                .source_wins => gate.proceed.set(env.io),
                else => unreachable,
            }
        }
        var result = pending.join();
        defer result.deinit();
        switch (mode) {
            .expires => try std.testing.expect(result == .fail and result.fail == .expected),
            .cancellation => try std.testing.expect(result == .interrupted),
            .source_wins => {
                try std.testing.expect(result == .success);
                try std.testing.expectEqual(@as(u32, 7), result.success);
            },
            .provider_rejection => {
                try std.testing.expect(result.primary() == .defected);
                try std.testing.expectEqual(fx.Defect.Kind.provider_unavailable, result.primary().defected.kind);
                try std.testing.expectEqual(@as(usize, 0), starts.load(.acquire));
                try std.testing.expectEqual(@as(usize, 0), clock.sleeps.load(.acquire));
            },
        }
        try std.testing.expectEqual(@as(usize, 1), selections.load(.acquire));
        try std.testing.expectEqual(@as(usize, 0), clock.manual.pending());
        if (mode != .provider_rejection) try std.testing.expect(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
    }
}

test "LIFE-07 CAP-11 timer capture OOM is setup failure before source work even when later allocations can succeed" {
    const FailNext = struct {
        base: std.mem.Allocator,
        armed: std.atomic.Value(bool) = .init(false),
        failures: std.atomic.Value(usize) = .init(0),
        fn allocator(self: *@This()) std.mem.Allocator {
            return .{ .ptr = self, .vtable = &.{ .alloc = alloc, .resize = resize, .remap = remap, .free = free } };
        }
        fn alloc(data: *anyopaque, size: usize, alignment: std.mem.Alignment, address: usize) ?[*]u8 {
            const self: *@This() = @ptrCast(@alignCast(data));
            if (self.armed.swap(false, .acq_rel)) {
                _ = self.failures.fetchAdd(1, .acq_rel);
                return null;
            }
            return self.base.rawAlloc(size, alignment, address);
        }
        fn resize(data: *anyopaque, memory: []u8, alignment: std.mem.Alignment, size: usize, address: usize) bool {
            const self: *@This() = @ptrCast(@alignCast(data));
            return self.base.rawResize(memory, alignment, size, address);
        }
        fn remap(data: *anyopaque, memory: []u8, alignment: std.mem.Alignment, size: usize, address: usize) ?[*]u8 {
            const self: *@This() = @ptrCast(@alignCast(data));
            return self.base.rawRemap(memory, alignment, size, address);
        }
        fn free(data: *anyopaque, memory: []u8, alignment: std.mem.Alignment, address: usize) void {
            const self: *@This() = @ptrCast(@alignCast(data));
            self.base.rawFree(memory, alignment, address);
        }
    };
    const Runtime = struct { allocator: std.mem.Allocator, io: std.Io, async_engine: fx.Executor, allocator_control: *FailNext, calls: *std.atomic.Value(usize), started: *std.Io.Event };
    const Program = fx.Effect(u32, E, Runtime);
    const Roles = struct {
        fn duration(env: *const Runtime) std.Io.Duration {
            env.allocator_control.armed.store(true, .release);
            return .fromSeconds(1);
        }
        fn source(env: *const Runtime) u32 {
            _ = env.calls.fetchAdd(1, .acq_rel);
            env.started.set(env.io);
            return 7;
        }
        fn timedOut() E {
            return .expected;
        }
    };
    var allocator_control: FailNext = .{ .base = std.testing.allocator };
    var calls: std.atomic.Value(usize) = .init(0);
    var started: std.Io.Event = .unset;
    var active: std.atomic.Value(bool) = .init(true);
    var invalid: std.atomic.Value(bool) = .init(false);
    var provider: @import("support/dsl_admission.zig").Controlled = .{ .base = .{ .allocator = std.testing.allocator, .capacity = 4, .capabilities = .{ .independent = true, .cpu = true, .blocking = true } }, .io = std.testing.io, .first_started = &started, .capture_active = &active, .items_active = &active, .invalid = &invalid, .reject_second_worker = false };
    defer provider.deinit();
    const env: Runtime = .{ .allocator = allocator_control.allocator(), .io = std.testing.io, .async_engine = provider.executor(), .allocator_control = &allocator_control, .calls = &calls, .started = &started };
    var result = fx.run(&env, comptime fx.dsl.timeout(Program.sync(Roles.source), .{ .duration = Roles.duration, .on_timeout = Roles.timedOut }));
    defer result.deinit();
    try std.testing.expect(result == .die and result.die.kind == .out_of_memory);
    try std.testing.expectEqual(@as(usize, 1), allocator_control.failures.load(.acquire));
    try std.testing.expectEqual(@as(usize, 0), calls.load(.acquire));
    try std.testing.expectEqual(@as(usize, 0), provider.submissions.load(.acquire));
    // This allocator fails only the selected capture allocation. A later
    // successful allocation cannot hide that failure in another race branch.
    const later = try env.allocator.alloc(u8, 1);
    env.allocator.free(later);
    try std.testing.expectEqual(@as(usize, 1), allocator_control.failures.load(.acquire));
}

test "LEAF-07 timeout static zero needs no providers and runtime negative duration rejects before admission" {
    const Pure = fx.Effect(u32, E, void);
    const mapped = struct {
        fn timedOut() E {
            return .expected;
        }
    }.timedOut;
    var zero = fx.run(&{}, comptime fx.dsl.timeout(Pure.succeed(7), .{ .duration = std.Io.Duration.zero, .on_timeout = mapped }));
    defer zero.deinit();
    try std.testing.expect(zero == .fail and zero.fail == .expected);
    var attempts: usize = 0;
    var initialized: usize = 0;
    var live: usize = 0;
    var cancellation: fx.Cancellation = .{};
    const env: Env = .{ .allocator = std.testing.allocator, .attempts = &attempts, .initialized = &initialized, .live = &live, .cancellation = &cancellation };
    var invalid = fx.run(&env, comptime fx.dsl.timeout(P.fromOutcome(Leaf.run), .{ .duration = struct {
        fn duration(_: *const Env) std.Io.Duration {
            return .{ .nanoseconds = -1 };
        }
    }.duration, .on_timeout = mapped }));
    defer invalid.deinit();
    try std.testing.expect(invalid == .die and invalid.die.kind == .invalid_configuration);
    try std.testing.expectEqual(@as(usize, 0), attempts);
}
