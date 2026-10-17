const std = @import("std");
const fx = @import("effect");
const p = fx.dsl.pipeline;
const E = enum { invalid };
const Env = struct { allocator: std.mem.Allocator, input: u16, calls: *usize };
const Large = struct { allocator: std.mem.Allocator, inner: Env };
const S = fx.Effect(u16, E, Env);
fn read(env: *const Env) u16 {
    env.calls.* += 1;
    return env.input;
}
fn widen(value: u16) fx.Effect(u32, E, Env) {
    return if (value == 0) fx.Effect(u32, E, Env).fail(.invalid) else fx.Effect(u32, E, Env).succeed(value);
}
fn compare(value: u32, env: *const Env) fx.Effect(bool, E, Env) {
    return fx.Effect(bool, E, Env).succeed(value == env.input);
}
fn project(env: *const Large) *const Env {
    return &env.inner;
}

test "PIPE-01 PIPE-05 COMP-04 static heterogeneous pipe is lazy reusable and short circuits" {
    const program = comptime p.pipe(S.sync(read), .{ p.bind(widen), p.bind(compare) });
    var calls: usize = 0;
    var env: Env = .{ .allocator = std.testing.allocator, .input = 7, .calls = &calls };
    try std.testing.expectEqual(@as(usize, 0), calls);
    for (0..2) |_| try std.testing.expect(fx.run(&env, program).success);
    env.input = 0;
    try std.testing.expectEqual(E.invalid, fx.run(&env, program).fail);
    try std.testing.expectEqual(@as(usize, 3), calls);
}

test "PIPE-02 PIPE-04 runtime pipeline owns intermediates and captured stage data" {
    const Captured = struct {
        fn next(capture: anytype, value: bool, _: *const Env) fx.Effect(u32, E, Env) {
            return fx.Effect(u32, E, Env).succeed(if (value) capture.answer else 0);
        }
    };
    var calls: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .input = 9, .calls = &calls };
    var source = S.sync(read);
    var program = p.owned(&source, &env, .{ p.bind(widen), p.bind(compare), p.captured(.{ .answer = @as(u32, 42) }, Captured.next) });
    defer program.deinit();
    for (0..2) |_| try std.testing.expectEqual(@as(u32, 42), fx.run(&env, program).success);
}

test "PIPE-03 COMP-08 static pipeline explicitly changes error and environment" {
    const Mapper = struct {
        fn map(_: E) u8 {
            return 19;
        }
    };
    const program = comptime p.pipe(S.sync(read), .{ p.bind(widen), p.mapError(Mapper.map), p.project(project) });
    comptime {
        if (@TypeOf(program) != fx.Effect(u32, u8, Large)) @compileError("exact transformed Effect required");
    }
    var calls: usize = 0;
    const env: Large = .{ .allocator = std.testing.allocator, .inner = .{ .allocator = std.testing.allocator, .input = 0, .calls = &calls } };
    try std.testing.expectEqual(@as(u8, 19), fx.run(&env, program).fail);
}

test "PIPE-07 empty static identity and explicit consuming transfer" {
    const identity = comptime p.pipe(S.succeed(7), .{});
    var calls: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .input = 7, .calls = &calls };
    var source = S.fromOwnedSuccess(&env, 9);
    var moved = p.transfer(&source);
    defer moved.deinit();
    try std.testing.expectEqual(@as(u16, 7), fx.run(&env, identity).success);
    try std.testing.expectEqual(@as(u16, 9), fx.run(&env, moved).success);
}

test "PIPE-03 PIPE-08 COMP-08 owned projection in the middle retains construction allocation origin" {
    const Changed = fx.Effect(u32, u8, Large);
    const Steps = struct {
        fn errorMap(_: E) u8 {
            return 19;
        }
        fn after(value: u32, env: *const Large) Changed {
            return Changed.succeed(value + env.inner.input);
        }
        fn last(value: u32) u64 {
            return value;
        }
        fn check(allocator: std.mem.Allocator) !void {
            var calls: usize = 0;
            const construction: Env = .{ .allocator = allocator, .input = 4, .calls = &calls };
            // Execution has a separate allocator; no construction node may be
            // freed there merely because its environment was projected.
            const execution: Large = .{ .allocator = std.testing.allocator, .inner = .{ .allocator = std.testing.allocator, .input = 4, .calls = &calls } };
            var source = S.sync(read);
            var described = p.owned(&source, &construction, .{ p.bind(widen), p.mapError(errorMap), p.project(project), p.bind(after), p.map(last) });
            defer described.deinit();
            comptime {
                if (@TypeOf(described) != fx.Effect(u64, u8, Large)) @compileError("owned stages must retain exact transformed Effect");
            }
            var result = fx.run(&execution, described);
            defer result.deinit();
            if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expectEqual(@as(u64, 8), result.success);
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Steps.check, .{});
}
