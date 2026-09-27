const std = @import("std");
const fx = @import("root.zig");

test "runtime capture is inline or explicitly borrowed and reusable" {
    const P = fx.Effect(u32, u32, void);
    var capture: u32 = 27;
    const program = P.fromBorrowed(u32, &capture, struct {
        fn callback(value: *const u32, _: *const void) fx.Exit(u32, u32) {
            return .{ .success = value.* };
        }
    }.callback);
    try std.testing.expectEqual(@as(u32, 27), fx.run(&{}, program).success);
    capture = 31;
    try std.testing.expectEqual(@as(u32, 31), fx.run(&{}, program).success);
    try std.testing.expectEqual(capture, fx.run(&{}, P.succeed(capture)).success);
}

test "full and partial provision bootstrap from the bound environment only" {
    const Env = struct { allocator: std.mem.Allocator, io: std.Io, number: u32 };
    const Larger = struct { embedded: Env };
    const P = fx.Effect(u32, u32, Env);
    const F = struct {
        fn read(env: *const Env) u32 {
            return env.number;
        }
        fn plus(value: u32) P {
            return P.succeed(value + 1);
        }
        fn project(env: *const Larger) *const Env {
            return &env.embedded;
        }
    };
    var tracking = std.testing.FailingAllocator.init(std.testing.allocator, .{});
    const env: Env = .{ .allocator = tracking.allocator(), .io = std.testing.io, .number = 40 };
    const source = comptime P.service(F.read).flatMap(F.plus);
    const provided: fx.Effect(u32, u32, void) = source.provide(&env);
    try std.testing.expectEqual(@as(u32, 41), fx.run(&{}, provided).success);
    try std.testing.expect(tracking.alloc_index > 0);
    try std.testing.expectEqual(tracking.allocated_bytes, tracking.freed_bytes);
    const projected = comptime source.provideSome(Larger, F.project);
    const larger: Larger = .{ .embedded = env };
    try std.testing.expectEqual(@as(u32, 41), fx.run(&larger, projected).success);
    try std.testing.expectEqual(tracking.allocated_bytes, tracking.freed_bytes);
}

test "native I/O uses the consumer vtable and maps errors inside Exit" {
    const Env = struct { allocator: std.mem.Allocator, io: std.Io };
    const E = enum { native };
    const P = fx.Effect(u32, E, Env);
    const F = struct {
        var calls: usize = 0;
        fn sleep(userdata: ?*anyopaque, timeout: std.Io.Timeout) std.Io.Cancelable!void {
            calls += 1;
            return std.testing.io.vtable.sleep(userdata, timeout);
        }
        fn operation(env: *const Env) std.Io.Cancelable!u32 {
            try env.io.sleep(.fromMilliseconds(0), .awake);
            return 7;
        }
        fn mapError(_: std.Io.Cancelable) E {
            return .native;
        }
    };
    F.calls = 0;
    var vtable = std.testing.io.vtable.*;
    vtable.sleep = F.sleep;
    const env: Env = .{ .allocator = std.testing.failing_allocator, .io = .{ .userdata = std.testing.io.userdata, .vtable = &vtable } };
    const program = comptime P.tryIo(F.operation, F.mapError);
    try std.testing.expectEqual(@as(usize, 0), F.calls);
    try std.testing.expectEqual(@as(u32, 7), fx.run(&env, program).success);
    try std.testing.expectEqual(@as(u32, 7), fx.run(&env, program).success);
    try std.testing.expectEqual(@as(usize, 2), F.calls);
}

test "late acquisition owns and releases before checking cancellation" {
    const Env = struct { allocator: std.mem.Allocator, cancellation: *fx.Cancellation, releases: *usize, primary: *bool };
    const P = fx.Effect(u32, u32, Env);
    const Release = fx.Effect(void, u32, Env);
    const F = struct {
        var body_called: bool = false;
        var release_primary: fx.PrimaryOutcome(u32) = undefined;
        fn acquire(env: *const Env) u32 {
            env.cancellation.request();
            return 7;
        }
        fn body(_: u32) P {
            body_called = true;
            return P.succeed(9);
        }
        fn release(_: u32, primary: fx.PrimaryOutcome(u32)) Release {
            release_primary = primary;
            return Release.sync(recordRelease);
        }
        fn recordRelease(env: *const Env) void {
            env.releases.* += 1;
            env.primary.* = release_primary == .was_interrupted;
        }
    };
    F.body_called = false;
    var token: fx.Cancellation = .{};
    var releases: usize = 0;
    var primary = false;
    const env: Env = .{ .allocator = std.testing.allocator, .cancellation = &token, .releases = &releases, .primary = &primary };
    const program = comptime P.service(F.acquire).acquireRelease(F.body, F.release);
    try std.testing.expect(fx.run(&env, program) == .interrupted);
    try std.testing.expect(!F.body_called);
    try std.testing.expect(primary);
    try std.testing.expectEqual(@as(usize, 1), releases);
}

const Nested = struct {
    const Env = struct { allocator: std.mem.Allocator, remaining: *usize, acquired: *usize, released: *usize };
    const E = enum { release_failed };
    const P = fx.Effect(usize, E, Env);
    const Release = fx.Effect(void, E, Env);
    const program = P.service(acquire).acquireRelease(body, release);
    fn acquire(env: *const Env) usize {
        env.acquired.* += 1;
        env.remaining.* -= 1;
        return env.remaining.*;
    }
    fn body(remaining: usize) P {
        return if (remaining == 0) P.succeed(remaining) else program;
    }
    fn release(_: usize, _: fx.PrimaryOutcome(E)) Release {
        return Release.sync(recordRelease).flatMap(releaseFailure);
    }
    fn recordRelease(env: *const Env) void {
        env.released.* += 1;
    }
    fn releaseFailure(_: void) Release {
        return Release.fail(.release_failed);
    }
};

test "10000 nested brackets retain every release failure and destroy iteratively" {
    // The implementation plan's nested-bracket stress gate.
    const required_bracket_depth: usize = 10_000;
    var remaining: usize = required_bracket_depth;
    var acquired: usize = 0;
    var released: usize = 0;
    const env: Nested.Env = .{ .allocator = std.testing.allocator, .remaining = &remaining, .acquired = &acquired, .released = &released };
    const program = Nested.program;
    var result = fx.run(&env, program);
    defer result.deinit();
    try std.testing.expect(result == .finalization_failed);
    try std.testing.expect(result.finalization_failed.primary == .succeeded);
    var iterator = result.finalization_failed.iterator();
    var failures: usize = 0;
    while (iterator.nextFailure()) |failure| {
        try std.testing.expectEqual(Nested.E.release_failed, failure.release_failure);
        failures += 1;
    }
    try std.testing.expectEqual(required_bracket_depth, failures);
    try std.testing.expectEqual(acquired, released);
}

test "OOM at resource admission never loses acquired resources or cleanup errors" {
    const program = Nested.program;
    // Outer admission failure, inner admission failure with an owned outer
    // resource, and successful admission of both resources are distinct paths.
    const ResourceRole = enum { outer, inner };
    const bracket_depth = @typeInfo(ResourceRole).@"enum".fields.len;
    for (0..bracket_depth + 1) |fail_index| {
        var failing = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = fail_index });
        var remaining: usize = bracket_depth;
        var acquired: usize = 0;
        var released: usize = 0;
        const env: Nested.Env = .{ .allocator = failing.allocator(), .remaining = &remaining, .acquired = &acquired, .released = &released };
        var result = fx.run(&env, program);
        defer result.deinit();
        try std.testing.expectEqual(acquired, released);
        if (acquired == 0) {
            try std.testing.expect(result == .die);
            try std.testing.expect(result.die.kind == .out_of_memory);
        } else {
            try std.testing.expect(result == .finalization_failed);
            if (fail_index < bracket_depth) try std.testing.expect(result.finalization_failed.primary == .defected);
            var iterator = result.finalization_failed.iterator();
            var failures: usize = 0;
            while (iterator.nextFailure()) |_| failures += 1;
            try std.testing.expectEqual(acquired, failures);
        }
    }
}

test "comptime service/map composition requires no runtime allocation" {
    const Env = struct { allocator: std.mem.Allocator, value: u32 };
    const P = fx.Effect(u32, void, Env);
    const F = struct {
        fn read(env: *const Env) u32 {
            return env.value;
        }
        fn double(value: u32) u32 {
            return value * 2;
        }
    };
    const program = comptime P.service(F.read).map(F.double);
    const env: Env = .{ .allocator = std.testing.failing_allocator, .value = 21 };
    try std.testing.expectEqual(@as(u32, 42), fx.run(&env, program).success);
}
