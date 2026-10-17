const std = @import("std");
const fx = @import("effect");
const E = enum { expected };
const R = struct { allocator: std.mem.Allocator, input: u32 };
const W = fx.WorkflowBuilder(E, R);

test "WF-02 INF-06 environment-aware durable bind forwards exact current R" {
    const program = comptime W.bind(W.succeed(@as(u32, 3)), struct {
        fn next(value: u32, env: *const R) W.Step(u64) {
            return W.succeed(@as(u64, value + env.input));
        }
    }.next);
    const env: R = .{ .allocator = std.testing.allocator, .input = 9 };
    try std.testing.expectEqual(@as(u64, 12), fx.run(&env, comptime W.run(program)).success);
}

test "WF-02 WF-09 CAP-11 runtime inputs stay in typed captures and lower to exact Effect" {
    const C = struct {
        fn next(input: anytype, _: *const R) W.Step(u32) {
            return W.succeed(input.number);
        }
    };
    const B = struct {
        fn next(input: anytype, value: u32, env: *const R) W.Step(u64) {
            return W.succeed(@as(u64, value + input.extra + env.input));
        }
    };
    const env: R = .{ .allocator = std.testing.allocator, .input = 7 };
    var first_owner = W.captureOwner(&env, .{ .number = @as(u32, 11) }, C.next);
    defer first_owner.deinit();
    var first = first_owner.step();
    var second_owner = W.bindOwned(&env, &first, .{ .extra = @as(u32, 5) }, B.next);
    defer second_owner.deinit();
    var second = second_owner.step();
    var effect = W.runOwned(&second);
    defer effect.deinit();
    comptime {
        if (@TypeOf(effect) != fx.Effect(u64, E, R)) @compileError("durable runtime boundary must retain exact Effect");
    }
    for (0..2) |_| try std.testing.expectEqual(@as(u64, 23), fx.run(&env, effect).success);
}

test "CAP-10 WF-09 live runtime Step owner checks copied handles after consumption" {
    const C = struct {
        fn next(input: anytype, _: *const R) W.Step(u32) {
            return W.succeed(input.number);
        }
    };
    const env: R = .{ .allocator = std.testing.allocator, .input = 1 };
    var owner = W.captureOwner(&env, .{ .number = @as(u32, 31) }, C.next);
    defer owner.deinit();
    var first = owner.step();
    var alias = first;
    var effect = W.runOwned(&first);
    try std.testing.expectEqual(@as(u32, 31), fx.run(&env, effect).success);
    effect.deinit();
    var second = W.runOwned(&alias);
    defer second.deinit();
    const rejected = fx.run(&env, second);
    try std.testing.expect(rejected == .die and rejected.die.kind == .invalid_configuration);
}

test "COMP-02 CAP-12 scoped captured durable operation returns an ordinary reusable Effect" {
    const C = struct {
        fn next(input: anytype, _: *const R) W.Step(u32) {
            return W.succeed(input.number);
        }
    };
    const env: R = .{ .allocator = std.testing.allocator, .input = 1 };
    var effect = W.captured(&env, .{ .number = @as(u32, 17) }, C.next);
    defer effect.deinit();
    for (0..2) |_| try std.testing.expectEqual(@as(u32, 17), fx.run(&env, effect).success);
}

test "CAP-10 LIFE-07 workflow shared owner retains the program and releases its capture on every allocation failure" {
    const Inputs = struct {
        pub const effect_owned_payload = true;
        drops: *usize,
        pub fn deinit(self: *@This()) void {
            self.drops.* += 1;
        }
        fn next(_: *const @This(), _: *const R) W.Step(u32) {
            return W.succeed(@as(u32, 31));
        }
    };
    const Check = struct {
        fn run(memory: std.mem.Allocator) !void {
            var drops: usize = 0;
            var exhausted = false;
            {
                const env: R = .{ .allocator = memory, .input = 1 };
                var owner = W.captureOwner(&env, Inputs{ .drops = &drops }, Inputs.next);
                var retained = owner.retain();
                owner.deinit();
                defer retained.deinit();
                var step = retained.step();
                var described = W.runOwned(&step);
                defer described.deinit();
                var result = fx.run(&env, described);
                defer result.deinit();
                exhausted = result == .die and result.die.kind == .out_of_memory;
                if (!exhausted) try std.testing.expect(result == .success and result.success == 31);
            }
            try std.testing.expectEqual(@as(usize, 1), drops);
            if (exhausted) return error.OutOfMemory;
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}
