const fx = @import("effect");
const std = @import("std");
const Env = struct { input: u32 };
const P = fx.Effect(u32, void, Env);

fn read(env: *const Env) u32 {
    return env.input;
}
fn double(value: u32) u32 {
    return value *% 2;
}
fn next(value: u32) P {
    return P.succeed(double(value));
}

fn nextWithEnvironment(value: u32, env: *const Env) P {
    return P.succeed(value +% env.input);
}

export fn effect_environment_bind(input: u32) u32 {
    const env: Env = .{ .input = input };
    const program = comptime P.service(read).flatMap(nextWithEnvironment);
    return switch (fx.run(&env, program)) {
        .success => |value| value,
        else => unreachable,
    };
}

export fn handwritten(input: u32) u32 {
    return double(input);
}

export fn effect_map(input: u32) u32 {
    const env: Env = .{ .input = input };
    const program = comptime P.service(read).map(double);
    return switch (fx.run(&env, program)) {
        .success => |value| value,
        else => unreachable,
    };
}

export fn effect_bind(input: u32) u32 {
    const env: Env = .{ .input = input };
    const program = comptime P.service(read).flatMap(next);
    return switch (fx.run(&env, program)) {
        .success => |value| value,
        else => unreachable,
    };
}

export fn handwritten_two_binds(input: u32) u32 {
    return double(double(input));
}

export fn effect_two_binds(input: u32) u32 {
    const env: Env = .{ .input = input };
    const program = comptime P.service(read).flatMap(next).flatMap(next);
    return switch (fx.run(&env, program)) {
        .success => |value| value,
        else => unreachable,
    };
}

export fn handwritten_eight_binds(input: u32) u32 {
    var value = input;
    for (0..8) |_| value = double(value);
    return value;
}

export fn effect_eight_binds(input: u32) u32 {
    const env: Env = .{ .input = input };
    const program = comptime build: {
        var chain = P.service(read);
        for (0..8) |_| chain = chain.flatMap(next);
        break :build chain;
    };
    return switch (fx.run(&env, program)) {
        .success => |value| value,
        else => unreachable,
    };
}

const ValidationError = enum { odd };
const Validated = fx.Effect(u32, ValidationError, Env);

fn validate(value: u32) Validated {
    return if (value % 2 == 0) Validated.succeed(double(value)) else Validated.fail(.odd);
}

fn recover(_: ValidationError) Validated {
    return Validated.succeed(0);
}

export fn handwritten_recovery(input: u32) u32 {
    return if (input % 2 == 0) double(input) else 0;
}

export fn effect_recovery(input: u32) u32 {
    const env: Env = .{ .input = input };
    const program = comptime Validated.service(read).flatMap(validate).catchAll(recover);
    return switch (fx.run(&env, program)) {
        .success => |value| value,
        else => unreachable,
    };
}

test "static specialization preserves wrapping arithmetic and typed recovery" {
    for ([_]u32{ 0, 1, 2, 7, 16, std.math.maxInt(u32) - 1, std.math.maxInt(u32) }) |input| {
        try std.testing.expectEqual(handwritten(input), effect_map(input));
        try std.testing.expectEqual(handwritten(input), effect_bind(input));
        try std.testing.expectEqual(handwritten(input), effect_environment_bind(input));
        try std.testing.expectEqual(handwritten_two_binds(input), effect_two_binds(input));
        try std.testing.expectEqual(handwritten_eight_binds(input), effect_eight_binds(input));
        try std.testing.expectEqual(handwritten_recovery(input), effect_recovery(input));
    }
}
