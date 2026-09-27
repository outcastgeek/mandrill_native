const std = @import("std");
const fx = @import("effect");

const Env = struct {
    allocator: std.mem.Allocator,
    remaining: *usize,
    tracking: ?*std.testing.FailingAllocator = null,
    reclaimed_at_factory_boundary: ?*bool = null,
    reusable: ?*const P = null,
};
const Failure = enum { expected };
const P = fx.Effect(usize, Failure, Env);

fn increment(value: usize) P {
    return P.succeed(value + 1);
}

fn temporaryFactory(env: *const Env) P {
    if (env.tracking) |tracking| {
        if (tracking.allocated_bytes != tracking.freed_bytes)
            env.reclaimed_at_factory_boundary.?.* = false;
    }
    if (env.remaining.* == 0) return P.succeed(42);
    env.remaining.* -= 1;
    var source = P.succeed(env.remaining.*);
    return source.flatMapOwned(env, continueBinding);
}

fn continueBinding(remaining: usize) P {
    return if (remaining == 0) P.succeed(42) else P.deferEffect(temporaryFactory);
}

test "100000 runtime-built bindings execute, rerun, and destroy iteratively" {
    // The implementation plan's dynamic-binding depth gate.
    const required_binding_depth: usize = 100_000;
    var remaining: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .remaining = &remaining };
    var program = P.succeed(7);
    for (0..required_binding_depth) |_| program = program.flatMapOwned(&env, increment);
    defer program.deinit();
    try std.testing.expectEqual(required_binding_depth + 7, fx.run(&env, program).success);
    try std.testing.expectEqual(required_binding_depth + 7, fx.run(&env, program).success);
}

test "description and execution allocation failures retain their allocator domains" {
    const Binding = enum { first, second, third };
    const depth = @typeInfo(Binding).@"enum".fields.len;
    var remaining: usize = 0;
    for (0..depth + 1) |fail_index| {
        var construction = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = fail_index });
        const env: Env = .{ .allocator = construction.allocator(), .remaining = &remaining };
        {
            var program = P.succeed(7);
            for (0..depth) |_| program = program.flatMapOwned(&env, increment);
            defer program.deinit();
            var result = fx.run(&env, program);
            defer result.deinit();
            if (fail_index < depth) {
                try std.testing.expect(result == .die);
                try std.testing.expectEqual(fx.Defect.Kind.out_of_memory, result.die.kind);
            } else {
                // Construction consumed the allocation budget. Execution needs
                // continuation frames, and reports their OOM through the same Exit.
                try std.testing.expect(result == .die);
            }
        }
        try std.testing.expectEqual(construction.allocated_bytes, construction.freed_bytes);
    }

    var construction = std.testing.FailingAllocator.init(std.testing.allocator, .{});
    const construct_env: Env = .{ .allocator = construction.allocator(), .remaining = &remaining };
    {
        var program = P.succeed(7);
        for (0..depth) |_| program = program.flatMapOwned(&construct_env, increment);
        defer program.deinit();
        // The leaf completes inline; each enclosing pending bind requires a frame.
        for (0..depth) |fail_index| {
            var execution = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = fail_index });
            const run_env: Env = .{ .allocator = execution.allocator(), .remaining = &remaining };
            var result = fx.run(&run_env, program);
            defer result.deinit();
            if (fail_index < depth - 1) {
                try std.testing.expect(result == .die);
                try std.testing.expectEqual(fx.Defect.Kind.out_of_memory, result.die.kind);
            } else try std.testing.expectEqual(@as(usize, 7 + depth), result.success);
            try std.testing.expectEqual(execution.allocated_bytes, execution.freed_bytes);
        }
    }
    try std.testing.expectEqual(construction.allocated_bytes, construction.freed_bytes);
}

test "factory-produced owning descriptions are reclaimed during steady recursion" {
    var tracking = std.testing.FailingAllocator.init(std.testing.allocator, .{});
    var remaining: usize = 10_000;
    var reclaimed = true;
    const env: Env = .{ .allocator = tracking.allocator(), .remaining = &remaining, .tracking = &tracking, .reclaimed_at_factory_boundary = &reclaimed };
    const program = comptime P.deferEffect(temporaryFactory);
    try std.testing.expectEqual(@as(usize, 42), fx.run(&env, program).success);
    try std.testing.expectEqual(@as(usize, 0), remaining);
    try std.testing.expect(reclaimed);
    try std.testing.expectEqual(tracking.allocated_bytes, tracking.freed_bytes);
}

test "two static bindings fold without a continuation allocation" {
    var remaining: usize = 0;
    const env: Env = .{ .allocator = std.testing.failing_allocator, .remaining = &remaining };
    const program = comptime P.succeed(7).flatMap(increment).flatMap(increment);
    try std.testing.expectEqual(@as(usize, 9), fx.run(&env, program).success);
}

test "full provision composes with a runtime-owned binding using the bound allocator" {
    const Unit = fx.Effect(usize, Failure, void);
    const F = struct {
        fn read(env: *const Env) usize {
            return env.remaining.*;
        }
        fn next(value: usize) Unit {
            return Unit.succeed(value + 1);
        }
    };
    var tracking = std.testing.FailingAllocator.init(std.testing.allocator, .{});
    var remaining: usize = 7;
    const env: Env = .{ .allocator = tracking.allocator(), .remaining = &remaining };
    const source = comptime P.service(F.read);
    {
        var supplied: Unit = source.provide(&env);
        var program: Unit = supplied.flatMapOwned(&{}, F.next);
        defer program.deinit();
        try std.testing.expectEqual(@as(usize, 8), fx.run(&{}, program).success);
        remaining = 19;
        try std.testing.expectEqual(@as(usize, 20), fx.run(&{}, program).success);
    }
    try std.testing.expect(tracking.alloc_index > 0);
    try std.testing.expectEqual(tracking.allocated_bytes, tracking.freed_bytes);
}

test "provided cancellation remains attached after a continuation returns a void environment" {
    const Bound = struct { allocator: std.mem.Allocator, cancellation: *fx.Cancellation };
    const Source = fx.Effect(*fx.Cancellation, Failure, Bound);
    const Supplied = fx.Effect(*fx.Cancellation, Failure, void);
    const Unit = fx.Effect(void, Failure, void);
    const F = struct {
        fn token(env: *const Bound) *fx.Cancellation {
            return env.cancellation;
        }
        fn next(cancellation: *fx.Cancellation) Unit {
            cancellation.request();
            return Unit.succeed({});
        }
    };
    var cancellation: fx.Cancellation = .{};
    const env: Bound = .{ .allocator = std.testing.allocator, .cancellation = &cancellation };
    const source = comptime Source.service(F.token);
    var supplied: Supplied = source.provide(&env);
    var program = supplied.flatMapOwned(&{}, F.next);
    defer program.deinit();
    try std.testing.expect(fx.run(&{}, program) == .interrupted);
}

test "a static continuation can transfer an owning runtime description into the run" {
    const Source = fx.Effect(*const Env, Failure, Env);
    const F = struct {
        fn services(env: *const Env) *const Env {
            return env;
        }
        fn build(env: *const Env) P {
            var source = P.succeed(env.remaining.*);
            return source.flatMapOwned(env, increment);
        }
    };
    var tracking = std.testing.FailingAllocator.init(std.testing.allocator, .{});
    var remaining: usize = 7;
    const env: Env = .{ .allocator = tracking.allocator(), .remaining = &remaining };
    const program = comptime Source.service(F.services).flatMap(F.build);
    try std.testing.expectEqual(@as(usize, 8), fx.run(&env, program).success);
    try std.testing.expectEqual(tracking.allocated_bytes, tracking.freed_bytes);
}

test "resource bodies and finalizers release their factory-owned description storage" {
    const Source = fx.Effect(*const Env, Failure, Env);
    const Release = fx.Effect(void, Failure, Env);
    const F = struct {
        fn acquire(env: *const Env) *const Env {
            return env;
        }
        fn body(env: *const Env) P {
            var source = P.succeed(7);
            return source.flatMapOwned(env, increment);
        }
        fn release(env: *const Env, _: fx.PrimaryOutcome(Failure)) Release {
            var source = Release.sync(recordRelease);
            return source.flatMapOwned(env, releaseFailure);
        }
        fn recordRelease(env: *const Env) void {
            env.remaining.* += 1;
        }
        fn releaseFailure(_: void) Release {
            return Release.fail(.expected);
        }
    };
    var tracking = std.testing.FailingAllocator.init(std.testing.allocator, .{});
    var remaining: usize = 0;
    const env: Env = .{ .allocator = tracking.allocator(), .remaining = &remaining };
    const program = comptime Source.service(F.acquire).acquireRelease(F.body, F.release);
    var result = fx.run(&env, program);
    defer result.deinit();
    try std.testing.expect(result == .finalization_failed);
    try std.testing.expectEqual(Failure.expected, result.finalization_failed.first.release_failure);
    try std.testing.expectEqual(@as(usize, 1), remaining);
    try std.testing.expectEqual(tracking.allocated_bytes, tracking.freed_bytes);
}

test "a borrowed owning description can be returned by a reusable factory" {
    const F = struct {
        fn factory(env: *const Env) P {
            return env.reusable.?.borrow();
        }
    };
    var tracking = std.testing.FailingAllocator.init(std.testing.allocator, .{});
    var remaining: usize = 7;
    var env: Env = .{ .allocator = tracking.allocator(), .remaining = &remaining };
    {
        var source = P.succeed(remaining);
        var owned = source.flatMapOwned(&env, increment);
        defer owned.deinit();
        env.reusable = &owned;
        const program = comptime P.deferEffect(F.factory);
        const retained_bytes = tracking.allocated_bytes - tracking.freed_bytes;
        try std.testing.expectEqual(@as(usize, 8), fx.run(&env, program).success);
        try std.testing.expectEqual(@as(usize, 8), fx.run(&env, program).success);
        try std.testing.expectEqual(retained_bytes, tracking.allocated_bytes - tracking.freed_bytes);
    }
    try std.testing.expectEqual(tracking.allocated_bytes, tracking.freed_bytes);
}

test "provided native cancellation remains active through an empty environment" {
    const Bound = struct { allocator: std.mem.Allocator, io: std.Io, pending: *bool };
    const Source = fx.Effect(*const Bound, Failure, Bound);
    const Unit = fx.Effect(void, Failure, void);
    const F = struct {
        fn checkCancel(data: ?*anyopaque) std.Io.Cancelable!void {
            const pending: *bool = @ptrCast(@alignCast(data.?));
            if (pending.*) {
                pending.* = false;
                return error.Canceled;
            }
        }
        fn read(env: *const Bound) *const Bound {
            return env;
        }
        fn next(env: *const Bound) Unit {
            env.pending.* = true;
            return Unit.succeed({});
        }
    };
    var pending = false;
    var vtable = std.testing.io.vtable.*;
    vtable.checkCancel = F.checkCancel;
    const env: Bound = .{ .allocator = std.testing.allocator, .io = .{ .userdata = &pending, .vtable = &vtable }, .pending = &pending };
    const source = comptime Source.service(F.read);
    var supplied = source.provide(&env);
    var program = supplied.flatMapOwned(&{}, F.next);
    defer program.deinit();
    try std.testing.expect(fx.run(&{}, program) == .interrupted);
    try std.testing.expect(!pending);
}

test "provided allocation survives deferred work inside another runtime binding" {
    const Unit = fx.Effect(usize, Failure, void);
    const F = struct {
        fn read(env: *const Env) usize {
            return env.remaining.*;
        }
        fn next(_: usize) Unit {
            return Unit.deferEffect(deferred);
        }
        fn deferred(_: *const void) Unit {
            return Unit.succeed(7);
        }
        fn finish(value: usize) Unit {
            return Unit.succeed(value + 1);
        }
    };
    var tracking = std.testing.FailingAllocator.init(std.testing.allocator, .{});
    var remaining: usize = 7;
    const env: Env = .{ .allocator = tracking.allocator(), .remaining = &remaining };
    const source = comptime P.service(F.read);
    {
        var supplied = source.provide(&env);
        var first = supplied.flatMapOwned(&{}, F.next);
        var program = first.flatMapOwned(&{}, F.finish);
        defer program.deinit();
        try std.testing.expectEqual(@as(usize, 8), fx.run(&{}, program).success);
    }
    try std.testing.expectEqual(tracking.allocated_bytes, tracking.freed_bytes);
}

test "a ready reduction checks a newly projected runtime before calling its service" {
    const Bound = struct { allocator: std.mem.Allocator, cancellation: *fx.Cancellation, reads: *usize };
    const Outer = struct { target: *const Bound, continued: *bool };
    const Source = fx.Effect(*const Bound, Failure, Outer);
    const Local = fx.Effect(usize, Failure, Bound);
    const Program = fx.Effect(usize, Failure, Outer);
    const F = struct {
        fn target(env: *const Outer) *const Bound {
            return env.target;
        }
        fn project(env: *const Outer) *const Bound {
            return env.target;
        }
        fn read(env: *const Bound) usize {
            env.reads.* += 1;
            return 7;
        }
        fn selected(_: *const Bound) Program {
            return Local.service(read).provideSome(Outer, project);
        }
        fn continued(_: usize) Program {
            return Program.sync(record);
        }
        fn record(env: *const Outer) usize {
            env.continued.* = true;
            return 8;
        }
    };
    var cancellation: fx.Cancellation = .{};
    cancellation.request();
    var reads: usize = 0;
    var continued = false;
    const bound: Bound = .{ .allocator = std.testing.failing_allocator, .cancellation = &cancellation, .reads = &reads };
    const env: Outer = .{ .target = &bound, .continued = &continued };
    const program = comptime Source.service(F.target).flatMap(F.selected).flatMap(F.continued);
    try std.testing.expect(fx.run(&env, program) == .interrupted);
    try std.testing.expectEqual(@as(usize, 0), reads);
    try std.testing.expect(!continued);
}

test "owning factory descriptions are leak checked across every Exit and allocation failure" {
    const Scenario = std.meta.Tag(fx.Exit(usize, Failure));
    const Runtime = struct { allocator: std.mem.Allocator, scenario: Scenario, open_resources: *usize };
    const Program = fx.Effect(usize, Failure, Runtime);
    const Choice = fx.Effect(Scenario, Failure, Runtime);
    const Source = fx.Effect(*const Runtime, Failure, Runtime);
    const Release = fx.Effect(void, Failure, Runtime);
    const F = struct {
        const bracket = Program.service(acquire).acquireRelease(body, release);
        const finalizer = Release.fromOutcome(close);

        fn environment(env: *const Runtime) *const Runtime {
            return env;
        }
        fn factory(env: *const Runtime) Program {
            var source = Choice.succeed(env.scenario);
            return source.flatMapOwned(env, select);
        }
        fn select(scenario: Scenario) Program {
            return switch (scenario) {
                .success => Program.succeed(7),
                .fail => Program.fail(.expected),
                .die => Program.die(fx.Defect.init("test defect")),
                .interrupted => Program.interrupted(),
                .finalization_failed => bracket,
            };
        }
        fn acquire(env: *const Runtime) usize {
            env.open_resources.* += 1;
            return 7;
        }
        fn body(_: usize) Program {
            return Program.fail(.expected);
        }
        fn release(_: usize, _: fx.PrimaryOutcome(Failure)) Release {
            return finalizer;
        }
        fn close(env: *const Runtime) fx.Exit(void, Failure) {
            env.open_resources.* -= 1;
            return .{ .fail = .expected };
        }
        fn exercise(allocator: std.mem.Allocator, scenario: Scenario) !void {
            var open_resources: usize = 0;
            const env: Runtime = .{ .allocator = allocator, .scenario = scenario, .open_resources = &open_resources };
            const program = comptime Source.service(environment).flatMap(factory);
            // Both runs must reclaim their factory-produced description. The
            // test runner checks the backing allocator after all defers finish.
            for (0..2) |_| {
                var result = fx.run(&env, program);
                defer result.deinit();
                try std.testing.expectEqual(@as(usize, 0), open_resources);
                // Only the test harness translates the typed defect for Zig's
                // exhaustive failure-injection helper; Effect still returns Exit.
                if (result == .die and result.die.kind == .out_of_memory)
                    return error.OutOfMemory;
                try std.testing.expectEqual(scenario, std.meta.activeTag(result));
                switch (result) {
                    .success => |value| try std.testing.expectEqual(@as(usize, 7), value),
                    .fail => |err| try std.testing.expectEqual(Failure.expected, err),
                    .die => |defect| try std.testing.expectEqualStrings("test defect", defect.message),
                    .interrupted => {},
                    .finalization_failed => |details| {
                        try std.testing.expectEqual(Failure.expected, details.primary.failed);
                        try std.testing.expectEqual(Failure.expected, details.first.release_failure);
                    },
                }
            }
        }
    };
    inline for (std.meta.tags(Scenario)) |scenario| {
        try std.testing.checkAllAllocationFailures(std.testing.allocator, F.exercise, .{scenario});
    }
}

test "a run snapshots I/O identity even when a callback replaces the environment field" {
    const Runtime = struct { allocator: std.mem.Allocator, io: std.Io, pending: *bool, mutable: ?*@This() = null };
    const Program = fx.Effect(usize, Failure, Runtime);
    const F = struct {
        fn checkCancel(data: ?*anyopaque) std.Io.Cancelable!void {
            const pending: *bool = @ptrCast(@alignCast(data.?));
            if (pending.*) {
                pending.* = false;
                return error.Canceled;
            }
        }
        fn replace(env: *const Runtime) usize {
            env.mutable.?.io = std.testing.io;
            env.pending.* = true;
            return 7;
        }
    };
    var pending = false;
    var vtable = std.testing.io.vtable.*;
    vtable.checkCancel = F.checkCancel;
    var env: Runtime = .{ .allocator = std.testing.failing_allocator, .io = .{ .userdata = &pending, .vtable = &vtable }, .pending = &pending };
    env.mutable = &env;
    const program = comptime Program.sync(F.replace);
    try std.testing.expect(fx.run(&env, program) == .interrupted);
    try std.testing.expect(!pending);
    // The next independent run selects the replacement provider from R.
    try std.testing.expectEqual(@as(usize, 7), fx.run(&env, program).success);
    try std.testing.expect(pending);
}
