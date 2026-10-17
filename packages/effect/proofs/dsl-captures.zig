const std = @import("std");
const fx = @import("effect");
const E = enum { expected };
const Env = struct {
    allocator: std.mem.Allocator,
    live: *usize,
    cleaned: *usize,
    invalid: *bool,
    cancel_in_factory: bool = false,
    cancellation: *fx.Cancellation,
};
const P = fx.Effect(u16, E, Env);
const Object = struct {
    pub const effect_owned_payload = true;
    live: *usize,
    cleaned: *usize,
    invalid: *bool,
    pub fn deinit(self: *@This()) void {
        self.live.* -= 1;
    }
    pub fn call(self: *const @This(), env: *const Env) P {
        const first = P.deferBorrowed(@This(), self, stageOne);
        if (!env.cancel_in_factory) return first;
        // A description destructor still depends on this capture even though
        // cancellation prevents its execution callback from starting.
        const last = terminal(self, env);
        env.cancellation.request();
        return last;
    }
    fn valid(env: *const Env) bool {
        if (env.live.* == 1) return true;
        env.invalid.* = true;
        return false;
    }
    fn stageOne(self: *const @This(), env: *const Env) P {
        if (!valid(env)) return P.die(fx.Defect.init("capture owner ended before deferred stage"));
        return P.deferBorrowed(@This(), self, stageTwo);
    }
    fn stageTwo(self: *const @This(), env: *const Env) P {
        if (!valid(env)) return P.die(fx.Defect.init("capture owner ended before deferred stage"));
        return terminal(self, env);
    }
    fn terminal(self: *const @This(), env: *const Env) P {
        const Cleanup = struct {
            pub const effect_owned_payload = true;
            borrowed: *const Object,
            live: *usize,
            cleaned: *usize,
            invalid: *bool,
            pub fn deinit(value: *@This()) void {
                if (value.live.* != 1) {
                    value.invalid.* = true;
                    return;
                }
                if (value.borrowed.live != value.live) value.invalid.* = true;
                value.cleaned.* += 1;
            }
            fn run(value: *const @This(), current: *const Env) fx.Exit(u16, E) {
                if (!Object.valid(current)) return .{ .die = fx.Defect.init("capture owner ended before terminal callback") };
                if (value.borrowed.live != current.live) current.invalid.* = true;
                return .{ .success = 37 };
            }
        };
        return P.fromOwned(env, Cleanup{ .borrowed = self, .live = env.live, .cleaned = env.cleaned, .invalid = env.invalid }, Cleanup.run);
    }
};

test "CAP-04 LIFE-07 explicit capture allocation preserves a narrow execution environment and cleans rejected storage" {
    const Narrow = struct { answer: u16, calls: *usize };
    const Out = fx.Effect(u16, E, Narrow);
    const Input = struct {
        pub const effect_owned_payload = true;
        released: *usize,
        fn run(_: *const @This(), env: *const Narrow) fx.Exit(u16, E) {
            env.calls.* += 1;
            return .{ .success = env.answer };
        }
        pub fn deinit(self: *@This()) void {
            self.released.* += 1;
        }
    };
    var released: usize = 0;
    var calls: usize = 0;
    const first: Narrow = .{ .answer = 17, .calls = &calls };
    const second: Narrow = .{ .answer = 29, .calls = &calls };
    var described = Out.fromOwnedAllocated(std.testing.allocator, Input{ .released = &released }, Input.run);
    try std.testing.expectEqual(@as(usize, 0), calls);
    try std.testing.expectEqual(@as(u16, 17), fx.run(&first, described).success);
    try std.testing.expectEqual(@as(u16, 29), fx.run(&second, described).success);
    try std.testing.expectEqual(@as(usize, 0), released);
    described.deinit();
    try std.testing.expectEqual(@as(usize, 1), released);
    var unexecuted = Out.fromOwnedAllocated(std.testing.allocator, Input{ .released = &released }, Input.run);
    unexecuted.deinit();
    try std.testing.expectEqual(@as(usize, 2), released);
    var failing: std.testing.FailingAllocator = .init(std.testing.allocator, .{ .fail_index = 0 });
    var rejected = Out.fromOwnedAllocated(failing.allocator(), Input{ .released = &released }, Input.run);
    defer rejected.deinit();
    const outcome = fx.run(&first, rejected);
    try std.testing.expect(outcome == .die and outcome.die.kind == .out_of_memory);
    try std.testing.expectEqual(@as(usize, 3), released);
    try std.testing.expectEqual(@as(usize, 2), calls);
}

test "CAP-08 LIFE-09 generated captures own nested canonical Effect descriptions" {
    const Runtime = struct { allocator: std.mem.Allocator };
    const Out = fx.Effect(u16, E, Runtime);
    const Owned = struct {
        pub const effect_owned_payload = true;
        live: *usize,
        calls: *usize,
        pub fn deinit(self: *@This()) void {
            self.live.* -= 1;
        }
        fn run(self: *const @This(), _: *const Runtime) fx.Exit(u16, E) {
            self.calls.* += 1;
            return .{ .success = 37 };
        }
    };
    const Stored = struct { source: Out };
    const Use = struct {
        fn body(stored: *const Stored, _: *const Runtime) Out {
            return stored.source.borrow();
        }
    };
    inline for (.{ .unexecuted, .reused, .cloned }) |mode| {
        var live: usize = 1;
        var calls: usize = 0;
        const env: Runtime = .{ .allocator = std.testing.allocator };
        const source = Out.fromOwned(&env, Owned{ .live = &live, .calls = &calls }, Owned.run);
        var captured = fx.dsl.owned(&env, Stored{ .source = source }, Use.body);
        for (0..if (mode == .unexecuted) @as(usize, 0) else 2) |_| {
            var result = fx.run(&env, captured);
            defer result.deinit();
            try std.testing.expect(result == .success and result.success == 37);
            try std.testing.expectEqual(@as(usize, 1), live);
        }
        if (mode == .cloned) {
            var copied = try fx.dsl.ownership.cloneValue(Stored{ .source = source });
            captured.deinit();
            try std.testing.expectEqual(@as(usize, 1), live);
            var result = fx.run(&env, copied.source);
            defer result.deinit();
            try std.testing.expect(result == .success and result.success == 37);
            fx.dsl.ownership.discard(&copied);
        } else captured.deinit();
        try std.testing.expectEqual(@as(usize, 0), live);
        try std.testing.expectEqual(@as(usize, if (mode == .unexecuted) 0 else if (mode == .reused) 2 else 3), calls);
    }
}
fn make(env: *const Env) P {
    env.live.* += 1;
    return fx.Dsl(E, Env).object(env, Object{ .live = env.live, .cleaned = env.cleaned, .invalid = env.invalid });
}

test "CAP-04 LIFE-09 returned deferred chain retains solely job-owned capture through description cleanup" {
    var live: usize = 0;
    var cleaned: usize = 0;
    var invalid = false;
    var cancellation: fx.Cancellation = .{};
    const env: Env = .{ .allocator = std.testing.allocator, .live = &live, .cleaned = &cleaned, .invalid = &invalid, .cancellation = &cancellation };
    var unexecuted = make(&env);
    unexecuted.deinit();
    try std.testing.expectEqual(@as(usize, 0), live);
    try std.testing.expectEqual(@as(usize, 0), cleaned);
    const described = comptime P.deferEffect(make);
    try std.testing.expectEqual(@as(u16, 37), fx.run(&env, described).success);
    try std.testing.expectEqual(@as(usize, 0), live);
    try std.testing.expectEqual(@as(usize, 1), cleaned);
    try std.testing.expect(!invalid);
}

test "CAP-04 LIFE-07 late cancellation disposes returned description before capture" {
    var live: usize = 0;
    var cleaned: usize = 0;
    var invalid = false;
    var cancellation: fx.Cancellation = .{};
    const env: Env = .{ .allocator = std.testing.allocator, .live = &live, .cleaned = &cleaned, .invalid = &invalid, .cancellation = &cancellation, .cancel_in_factory = true };
    try std.testing.expect(fx.run(&env, comptime P.deferEffect(make)) == .interrupted);
    try std.testing.expectEqual(@as(usize, 0), live);
    try std.testing.expectEqual(@as(usize, 1), cleaned);
    try std.testing.expect(!invalid);
}

test "LIFE-07 all allocation failures unwind job-owned capture and dependent descriptions" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            var live: usize = 0;
            var cleaned: usize = 0;
            var invalid = false;
            var cancellation: fx.Cancellation = .{};
            const env: Env = .{ .allocator = allocator, .live = &live, .cleaned = &cleaned, .invalid = &invalid, .cancellation = &cancellation };
            var result = fx.run(&env, comptime P.deferEffect(make));
            defer result.deinit();
            try std.testing.expectEqual(@as(usize, 0), live);
            try std.testing.expect(!invalid);
            if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expectEqual(@as(u16, 37), result.success);
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

test "LIFE-10 generated retained roots release a long job-owned chain iteratively" {
    const CountEnv = struct { allocator: std.mem.Allocator, remaining: *usize, live: *usize };
    const Out = fx.Effect(usize, void, CountEnv);
    const Link = struct {
        pub const effect_owned_payload = true;
        live: *usize,
        pub fn deinit(self: *@This()) void {
            self.live.* -= 1;
        }
        pub fn call(_: *const @This(), _: *const CountEnv) Out {
            return Out.deferEffect(next);
        }
        fn next(env: *const CountEnv) Out {
            if (env.remaining.* == 0) return Out.succeed(17);
            env.remaining.* -= 1;
            env.live.* += 1;
            return fx.Dsl(void, CountEnv).object(env, @This(){ .live = env.live });
        }
    };
    var remaining: usize = 10000;
    var live: usize = 0;
    const env: CountEnv = .{ .allocator = std.testing.allocator, .remaining = &remaining, .live = &live };
    try std.testing.expectEqual(@as(usize, 17), fx.run(&env, comptime Out.deferEffect(Link.next)).success);
    try std.testing.expectEqual(@as(usize, 0), live);
}

test "CAP-06 CAP-07 CAP-10 checked aliases preserve input across provider rejection then consume once" {
    const Input = struct {
        pub const effect_owned_payload = true;
        released: *usize,
        pub fn deinit(self: *@This()) void {
            self.released.* += 1;
        }
    };
    const WorkerEnv = struct { allocator: std.mem.Allocator, io: std.Io, parallel_engine: fx.Executor };
    const Worker = struct {
        fn run(value: Input, _: *const WorkerEnv) fx.Exit(u32, E) {
            var moved = value;
            moved.deinit();
            return .{ .success = 29 };
        }
    };
    var released: usize = 0;
    const owner = try fx.dsl.captures.OneShot(Input).create(std.testing.allocator, .{ .released = &released });
    defer owner.deinit();
    const one = owner.handle();
    const alias = one;
    var provider: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .cpu = true, .independent = true, .blocking = false }, .maximum_owned = 0 };
    defer provider.deinit();
    var env: WorkerEnv = .{ .allocator = std.testing.allocator, .io = std.testing.io, .parallel_engine = provider.executor() };
    const described = one.onCpu(Worker.run);
    comptime {
        if (@TypeOf(described) != fx.Effect(u32, E, WorkerEnv)) @compileError("one-shot operation must be exact inferred Effect");
    }
    try std.testing.expect(fx.run(&env, described) == .die);
    try std.testing.expect(owner.isAvailable());
    try std.testing.expectEqual(@as(usize, 0), released);
    provider.maximum_owned = 1;
    env.parallel_engine = provider.executor();
    try std.testing.expectEqual(@as(u32, 29), fx.run(&env, alias.onCpu(Worker.run)).success);
    try std.testing.expect(!owner.isAvailable());
    const reused = fx.run(&env, one.effect(Worker.run));
    try std.testing.expect(reused == .die and reused.die.kind == .invalid_configuration);
    try std.testing.expectEqual(@as(usize, 1), released);
}

test "CAP-08 composite moved retained borrowed optional and variant ownership follows active fields" {
    const Payload = struct {
        pub const effect_owned_payload = true;
        count: *usize,
        pub fn deinit(self: *@This()) void {
            self.count.* += 1;
        }
    };
    const Shared = struct {
        refs: std.atomic.Value(usize) = .init(1),
        fn retain(self: *@This()) *@This() {
            _ = self.refs.fetchAdd(1, .monotonic);
            return self;
        }
        fn release(value: **@This()) void {
            _ = value.*.refs.fetchSub(1, .acq_rel);
        }
    };
    var shared: Shared = .{};
    var released: usize = 0;
    var input: u16 = 7;
    var live: usize = 0;
    var cleaned: usize = 0;
    var invalid = false;
    var cancellation: fx.Cancellation = .{};
    const env: Env = .{ .allocator = std.testing.allocator, .live = &live, .cleaned = &cleaned, .invalid = &invalid, .cancellation = &cancellation };
    const Variant = union(enum) { plain: u8, owned: Payload };
    const Record = struct {
        fn read(value: anytype, _: *const Env) u16 {
            return value.borrowed.*;
        }
    };
    var described = fx.Dsl(E, Env).syncOwned(&env, .{
        .moved = Payload{ .count = &released },
        .shared = fx.dsl.ownership.retained(&shared, .{ .retain = Shared.retain, .release = Shared.release, .synchronization = fx.dsl.ownership.Synchronization.thread_safe }),
        .borrowed = &input,
        .optional = @as(?Payload, .{ .count = &released }),
        .variant = Variant{ .owned = .{ .count = &released } },
    }, Record.read);
    try std.testing.expectEqual(@as(usize, 2), shared.refs.load(.acquire));
    try std.testing.expectEqual(@as(u16, 7), fx.run(&env, described).success);
    described.deinit();
    try std.testing.expectEqual(@as(usize, 3), released);
    try std.testing.expectEqual(@as(usize, 1), shared.refs.load(.acquire));
}

test "ASYNC-05 CAP-05 thread_safe retained captures cross concurrent workers without business cloning" {
    const Shared = struct {
        refs: std.atomic.Value(usize) = .init(1),
        calls: std.atomic.Value(u32) = .init(0),
        fn retain(self: *@This()) *@This() {
            _ = self.refs.fetchAdd(1, .monotonic);
            return self;
        }
        fn release(self: **@This()) void {
            _ = self.*.refs.fetchSub(1, .acq_rel);
        }
    };
    const WorkerEnv = struct { allocator: std.mem.Allocator, io: std.Io, async_engine: fx.Executor, parallel_engine: fx.Executor };
    const Worker = struct {
        fn run(value: anytype, _: *const WorkerEnv) fx.Exit(u32, E) {
            var moved = value;
            const result = moved.value.calls.fetchAdd(1, .acq_rel);
            moved.deinit();
            return .{ .success = result };
        }
    };
    var shared: Shared = .{};
    const operations = .{ .retain = Shared.retain, .release = Shared.release, .synchronization = fx.dsl.ownership.Synchronization.thread_safe };
    const a = fx.dsl.ownership.retained(&shared, operations);
    const b = fx.dsl.ownership.retained(&shared, operations);
    const first = try fx.dsl.captures.OneShot(@TypeOf(a)).create(std.testing.allocator, a);
    defer first.deinit();
    const second = try fx.dsl.captures.OneShot(@TypeOf(b)).create(std.testing.allocator, b);
    defer second.deinit();
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = false }, .maximum_owned = 8 };
    defer engine.deinit();
    const env: WorkerEnv = .{ .allocator = std.testing.allocator, .io = std.testing.io, .async_engine = engine.executor(), .parallel_engine = engine.executor() };
    const children = [_]fx.Effect(u32, E, WorkerEnv){ first.handle().onCpu(Worker.run), second.handle().onCpu(Worker.run) };
    var result = fx.run(&env, fx.Effect(u32, E, WorkerEnv).allConcurrent(2, &children));
    defer result.deinit();
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(u32, 2), shared.calls.load(.acquire));
    try std.testing.expectEqual(@as(usize, 1), shared.refs.load(.acquire));
}
