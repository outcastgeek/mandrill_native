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

export fn handwritten_validate_u8(value: u8) bool {
    return value < 128;
}
const ValidationCodec = fx.PureCodec(u8, u8).init(struct {
    fn decode(value: *const u8) fx.Result(u8, fx.SchemaIssue.Issue) {
        return .{ .ok = value.* };
    }
}.decode, struct {
    fn encode(value: *const u8) fx.Result(u8, fx.SchemaIssue.Issue) {
        if (value.* >= 128) return .{ .err = fx.SchemaIssue.Issue.pure(.encode, .invalid_value) };
        return .{ .ok = value.* };
    }
}.encode);
export fn pure_validate_u8(value: u8) bool {
    return fx.requirePure(ValidationCodec).validateDirect(&value) == .ok;
}

export fn handwritten_write_boolean(value: bool, writer: *fx.JsonWriter.Writer) bool {
    writer.boolean(value) catch return false;
    return true;
}
export fn pure_write_boolean(value: bool, writer: *fx.JsonWriter.Writer) bool {
    return fx.JsonCodec.boolean.writeJson(&value, writer) == .success;
}
const Tagged = union(enum) { integer: i64, boolean: bool };
const TaggedEnv = struct { allocator: std.mem.Allocator };
const TaggedSchema = fx.VariantSchema.Variant(Tagged, TaggedEnv, "kind", "value", .{ fx.VariantSchema.taggedCase("integer", "integer", fx.JsonCodec.signed_integer), fx.VariantSchema.taggedCase("boolean", "boolean", fx.JsonCodec.boolean) });
export fn handwritten_tagged_integer(value: i64) i64 {
    const tagged: Tagged = .{ .integer = value };
    return tagged.integer;
}
export fn specialized_tagged_integer(value: i64) i64 {
    const tagged = TaggedSchema.make("integer", value);
    return TaggedSchema.project("integer", &tagged).?.*;
}

export fn dsl_role(input: u32) u32 {
    const env: Env = .{ .input = input };
    const described = comptime fx.Dsl(void, Env).sync(.{ .run = read }).map(double);
    return fx.run(&env, described).success;
}
export fn dsl_selector(input: u32) u32 {
    const env: Env = .{ .input = input };
    const described = comptime fx.Dsl(void, Env).field("input").map(struct {
        fn project(value: *const u32) u32 {
            return double(value.*);
        }
    }.project);
    return fx.run(&env, described).success;
}
export fn dsl_pipeline(input: u32) u32 {
    const env: Env = .{ .input = input };
    const p = fx.dsl.pipeline;
    const described = comptime p.pipe(P.sync(read), .{ p.map(double), p.map(double) });
    return fx.run(&env, described).success;
}
export fn dsl_structured(input: u32) u32 {
    const env: Env = .{ .input = input };
    const p = fx.dsl.program;
    const Steps = struct {
        fn twice(value: *const u32, _: *const Env) u32 {
            return double(value.*);
        }
    };
    const described = comptime p.describe(.{ p.native("a", .{}, read), p.native("b", .{"a"}, Steps.twice), p.native("result", .{"b"}, Steps.twice) });
    return fx.run(&env, described).success;
}

export fn handwritten_sixteen(input: u32) u32 {
    return input *% 32768;
}
export fn dsl_structured_sixteen(input: u32) u32 {
    const env: Env = .{ .input = input };
    const p = fx.dsl.program;
    const Steps = struct {
        fn twice(value: *const u32, _: *const Env) u32 {
            return double(value.*);
        }
    };
    const described = comptime p.describe(.{ p.native("s0", .{}, read), p.native("s1", .{"s0"}, Steps.twice), p.native("s2", .{"s1"}, Steps.twice), p.native("s3", .{"s2"}, Steps.twice), p.native("s4", .{"s3"}, Steps.twice), p.native("s5", .{"s4"}, Steps.twice), p.native("s6", .{"s5"}, Steps.twice), p.native("s7", .{"s6"}, Steps.twice), p.native("s8", .{"s7"}, Steps.twice), p.native("s9", .{"s8"}, Steps.twice), p.native("s10", .{"s9"}, Steps.twice), p.native("s11", .{"s10"}, Steps.twice), p.native("s12", .{"s11"}, Steps.twice), p.native("s13", .{"s12"}, Steps.twice), p.native("s14", .{"s13"}, Steps.twice), p.native("s15", .{"s14"}, Steps.twice) });
    return fx.run(&env, described).success;
}
