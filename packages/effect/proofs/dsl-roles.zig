const std = @import("std");
const fx = @import("effect");
const E = enum { invalid };
const Env = struct { io: std.Io = std.testing.io, allocator: std.mem.Allocator, input: u16, calls: *usize };
const D = fx.Dsl(E, Env);
fn read(env: *const Env) u16 {
    env.calls.* += 1;
    return env.input;
}

test "ROLE-01 ROLE-03 LEAF-01 COMP-01 named functions and namespaces are lazy exact Effects" {
    var calls: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .input = 7, .calls = &calls };
    const a = comptime D.sync(.{ .run = read });
    const Namespace = struct {
        pub fn run(current: *const Env) u16 {
            return read(current);
        }
    };
    const b = comptime D.sync(Namespace);
    comptime {
        if (@TypeOf(a) != fx.Effect(u16, E, Env) or @TypeOf(b) != @TypeOf(a)) @compileError("exact Effect required");
    }
    try std.testing.expectEqual(@as(usize, 0), calls);
    try std.testing.expectEqual(@as(u16, 7), fx.run(&env, a).success);
    try std.testing.expectEqual(@as(u16, 7), fx.run(&env, b).success);
    try std.testing.expectEqual(@as(usize, 2), calls);
}

test "INF-12 INF-01 COMP-04 generic specialization composes with existing map and bind" {
    const Generic = struct {
        fn widen(value: anytype) u32 {
            return value;
        }
        fn next(value: anytype, env: *const Env) fx.Effect(bool, E, Env) {
            return fx.Effect(bool, E, Env).succeed(value == env.input);
        }
    };
    const program = comptime fx.Effect(u16, E, Env).sync(read).map(Generic.widen).flatMap(Generic.next);
    var calls: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .input = 11, .calls = &calls };
    comptime {
        if (@TypeOf(program) != fx.Effect(bool, E, Env)) @compileError("inferred type mismatch");
    }
    try std.testing.expect(fx.run(&env, program).success);
    try std.testing.expectEqual(@as(usize, 1), calls);
}

test "INF-02 LEAF-02 COMP-06 native error mapping keeps cancellation separate" {
    const Native = struct {
        fn run(env: *const Env) error{ Bad, Canceled }!u16 {
            return switch (env.input) {
                0 => error.Bad,
                1 => error.Canceled,
                else => env.input,
            };
        }
        fn map(_: error{ Bad, Canceled }) E {
            return .invalid;
        }
    };
    const program = comptime D.native(.{ .run = Native.run, .map_error = Native.map });
    var calls: usize = 0;
    var env: Env = .{ .allocator = std.testing.allocator, .input = 2, .calls = &calls };
    try std.testing.expectEqual(@as(u16, 2), fx.run(&env, program).success);
    env.input = 0;
    try std.testing.expectEqual(E.invalid, fx.run(&env, program).fail);
    env.input = 1;
    try std.testing.expect(fx.run(&env, program) == .interrupted);
}

test "SELECT-01 SELECT-03 INF-05 borrowed field is the original current environment storage" {
    var calls: usize = 0;
    const one: Env = .{ .allocator = std.testing.allocator, .input = 3, .calls = &calls };
    const two: Env = .{ .allocator = std.testing.allocator, .input = 9, .calls = &calls };
    const select = comptime D.field("input");
    try std.testing.expect(fx.run(&one, select).success == &one.input);
    try std.testing.expect(fx.run(&two, select).success == &two.input);
}

test "CAP-02 CAP-03 CAP-04 CAP-11 runtime records and objects retain returned deferred borrows" {
    const Object = struct {
        input: u16,
        pub fn call(self: *const @This(), _: *const Env) fx.Effect(u16, E, Env) {
            return fx.Effect(u16, E, Env).deferBorrowed(u16, &self.input, struct {
                fn next(value: *const u16, _: *const Env) fx.Effect(u16, E, Env) {
                    return fx.Effect(u16, E, Env).succeed(value.*);
                }
            }.next);
        }
    };
    var calls: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .input = 23, .calls = &calls };
    var program = D.object(&env, Object{ .input = env.input });
    defer program.deinit();
    for (0..3) |_| try std.testing.expectEqual(@as(u16, 23), fx.run(&env, program).success);
}

test "CAP-08 CAP-09 moved noncloneable fields clean up once without requiring clone evidence" {
    const Payload = struct {
        pub const effect_owned_payload = true;
        count: *usize,
        pub fn deinit(self: *@This()) void {
            self.count.* += 1;
        }
    };
    const Read = struct {
        fn run(capture: anytype, _: *const Env) u16 {
            return capture.number;
        }
    };
    var calls: usize = 0;
    var released: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .input = 4, .calls = &calls };
    var program = D.syncOwned(&env, .{ .number = env.input, .payload = Payload{ .count = &released } }, Read.run);
    try std.testing.expectEqual(@as(u16, 4), fx.run(&env, program).success);
    try std.testing.expectEqual(@as(usize, 0), released);
    program.deinit();
    try std.testing.expectEqual(@as(usize, 1), released);
}

test "ROLE-07 runtime callback pointer is stored as data and selected for each description" {
    var calls: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .input = 13, .calls = &calls };
    const Other = struct {
        fn readOther(current: *const Env) u16 {
            return current.input + 1;
        }
    };
    var pointer: *const fn (*const Env) u16 = read;
    var first = fx.dsl.syncPointer(E, &env, pointer);
    defer first.deinit();
    pointer = Other.readOther;
    var second = fx.dsl.syncPointer(E, &env, pointer);
    defer second.deinit();
    try std.testing.expectEqual(@as(u16, 13), fx.run(&env, first).success);
    try std.testing.expectEqual(@as(u16, 14), fx.run(&env, second).success);
}

test "LEAF-05 CAP-02 captured option factories defer present and missing branches until execution" {
    const Input = struct { value: ?u16 };
    const Roles = struct {
        fn missing() E {
            return .invalid;
        }
        fn start(input: *const Input, current: *const Env) fx.Effect(u16, E, Env) {
            current.calls.* += 1;
            return fx.Effect(u16, E, Env).fromOption(input.value, missing);
        }
    };
    var calls: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .input = 0, .calls = &calls };
    inline for (.{ @as(?u16, 19), @as(?u16, null) }) |value| {
        const before = calls;
        var description = fx.dsl.owned(&env, Input{ .value = value }, Roles.start);
        try std.testing.expectEqual(before, calls);
        var result = fx.run(&env, description);
        description.deinit();
        defer result.deinit();
        if (value) |number| {
            try std.testing.expect(result == .success and result.success == number);
        } else try std.testing.expect(result == .fail and result.fail == .invalid);
        try std.testing.expectEqual(before + 1, calls);
    }
}

test "ROLE-04 sequenced initializer form supplies heterogeneous binding inputs in declared order" {
    const Initializers = struct {
        var order: usize = 0;
        fn first() u16 {
            order = order * 10 + 1;
            return 7;
        }
        fn second() bool {
            order = order * 10 + 2;
            return true;
        }
    };
    Initializers.order = 0;
    const values = fx.dsl.sequence(.{ Initializers.first, Initializers.second });
    try std.testing.expectEqual(@as(usize, 12), Initializers.order);
    try std.testing.expectEqual(@as(u16, 7), values[0]);
    try std.testing.expect(values[1]);
}
