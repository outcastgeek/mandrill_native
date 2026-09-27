const std = @import("std");
const fx = @import("effect");

const Failure = enum { expected };
const Env = struct { calls: *usize };
const P = fx.Effect(void, Failure, Env);

fn record(env: *const Env) void {
    env.calls.* += 1;
}

fn reject(_: void) P {
    return P.fail(.expected);
}

test "void-valued ordinary binding preserves typed failure" {
    var calls: usize = 0;
    const env: Env = .{ .calls = &calls };
    const program = comptime P.sync(record).flatMap(reject);
    const result = fx.run(&env, program);
    try std.testing.expectEqual(Failure.expected, result.fail);
    try std.testing.expectEqual(@as(usize, 1), calls);
}
