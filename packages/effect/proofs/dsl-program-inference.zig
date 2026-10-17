const std = @import("std");
const fx = @import("effect");
const p = fx.dsl.program;
const E = enum { expected };
const R = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, input: u32, releases: ?*usize = null };
const Steps = struct {
    fn first(env: *const R) fx.Effect(u32, E, R) {
        return fx.Effect(u32, E, R).succeed(env.input);
    }
    fn next(value: *const u32, _: *const R) fx.Effect(u64, E, R) {
        return fx.Effect(u64, E, R).succeed(value.* + 1);
    }
    fn native(env: *const R) u32 {
        return env.input;
    }
    fn afterNative(value: *const u32, _: *const R) u64 {
        return value.* + 2;
    }
    fn genericFirst(env: anytype) fx.Effect(u32, E, @typeInfo(@TypeOf(env)).pointer.child) {
        return fx.Effect(u32, E, @typeInfo(@TypeOf(env)).pointer.child).succeed(env.input);
    }
};

test "INF-11 PROG-01 program description infers result error and environment from binding callbacks without repeated context arguments" {
    const described = comptime p.describe(.{ p.binding("first", .{}, Steps.first), p.binding("answer", .{"first"}, Steps.next) });
    comptime if (@TypeOf(described) != fx.Effect(u64, E, R)) @compileError("program inference must retain exact Effect");
    const env: R = .{ .allocator = std.testing.allocator, .input = 7 };
    var result = fx.run(&env, described);
    defer result.deinit();
    try std.testing.expectEqual(@as(u64, 8), result.success);
}

test "INF-11 PROG-01 native program infers its environment and an empty error channel" {
    const described = comptime p.describe(.{ p.native("first", .{}, Steps.native), p.native("answer", .{"first"}, Steps.afterNative) });
    comptime if (@TypeOf(described) != fx.Effect(u64, void, R)) @compileError("native program must infer only its available channels");
    const env: R = .{ .allocator = std.testing.allocator, .input = 7 };
    var result = fx.run(&env, described);
    defer result.deinit();
    try std.testing.expectEqual(@as(u64, 9), result.success);
}

test "INF-11 INF-13 a later typed binding supplies the generic first binding context and only an ambiguous environment needs an anchor" {
    const paired = comptime p.describe(.{ p.binding("first", .{}, Steps.genericFirst), p.binding("answer", .{"first"}, Steps.next) });
    const anchored = comptime p.describeWith(.{ .environment = R }, .{p.binding("answer", .{}, Steps.genericFirst)});
    comptime {
        if (@TypeOf(paired) != fx.Effect(u64, E, R)) @compileError("later callback evidence must specialize a generic earlier callback");
        if (@TypeOf(anchored) != fx.Effect(u32, E, R)) @compileError("an environment anchor must not require a repeated inferable error channel");
    }
    const env: R = .{ .allocator = std.testing.allocator, .input = 17 };
    var first = fx.run(&env, paired);
    defer first.deinit();
    var second = fx.run(&env, anchored);
    defer second.deinit();
    try std.testing.expectEqual(@as(u64, 18), first.success);
    try std.testing.expectEqual(@as(u32, 17), second.success);
}

test "INF-11 INF-13 native binding data flow specializes a generic Effect continuation and an optional error anchor adds only its selected channel" {
    const Generic = struct {
        fn next(value: anytype, env: anytype) fx.Effect(u64, E, @typeInfo(@TypeOf(env)).pointer.child) {
            return fx.Effect(u64, E, @typeInfo(@TypeOf(env)).pointer.child).succeed(value.* + 3);
        }
    };
    const described = comptime p.describe(.{ p.native("first", .{}, Steps.native), p.binding("answer", .{"first"}, Generic.next) });
    const anchored = comptime p.describeWith(.{ .error_type = E }, .{p.native("answer", .{}, Steps.native)});
    comptime {
        if (@TypeOf(described) != fx.Effect(u64, E, R)) @compileError("binding graph evidence must infer the generic continuation channel");
        if (@TypeOf(anchored) != fx.Effect(u32, E, R)) @compileError("an explicit error anchor must preserve the inferred environment");
    }
    const env: R = .{ .allocator = std.testing.allocator, .input = 17 };
    var result = fx.run(&env, described);
    defer result.deinit();
    try std.testing.expectEqual(@as(u64, 20), result.success);
}

test "INF-11 PROG-02 PROG-03 RES-01 branch loop and lexical resource descriptions infer their exact shared context" {
    const Nodes = struct {
        fn predicate(env: *const R) bool {
            return env.input == 17;
        }
        fn no(_: *const R) fx.Effect(u32, E, R) {
            return fx.Effect(u32, E, R).succeed(99);
        }
        fn initialize(_: *const R) u32 {
            return 0;
        }
        fn condition(value: *const u32, _: *const R) bool {
            return value.* < 3;
        }
        fn body(view: anytype, _: *const R) fx.Effect(p.LoopControl(u32), E, R) {
            view.state().* += 1;
            return fx.Effect(p.LoopControl(u32), E, R).succeed(.again);
        }
        fn finish(view: anytype, _: *const R) u32 {
            return view.state().*;
        }
        fn acquire(env: *const R) fx.Effect(*const u32, E, R) {
            return fx.Effect(*const u32, E, R).succeed(&env.input);
        }
        fn release(_: *const u32) fx.Effect(void, E, R) {
            return fx.Effect(void, E, R).fromOutcome(struct {
                fn run(env: *const R) fx.Exit(void, E) {
                    env.releases.?.* += 1;
                    return .{ .success = {} };
                }
            }.run);
        }
        fn read(lease: *const *const u32, _: *const R) fx.Effect(u32, E, R) {
            return fx.Effect(u32, E, R).succeed(lease.*.*);
        }
        const branched = p.describe(.{p.branch("answer", .{}, predicate, Steps.first, no)});
        const looped = p.describe(.{p.loop("answer", .{}, initialize, condition, body, finish)});
        const scoped = p.describe(.{ p.resource("lease", .{}, acquire, release), p.binding("answer", .{"lease"}, read) });
    };
    comptime for (.{ Nodes.branched, Nodes.looped, Nodes.scoped }) |plan| {
        if (@TypeOf(plan) != fx.Effect(u32, E, R)) @compileError("every structured descriptor must infer exact Effect context");
    };
    var releases: usize = 0;
    const env: R = .{ .allocator = std.testing.allocator, .input = 17, .releases = &releases };
    var branched = fx.run(&env, Nodes.branched);
    defer branched.deinit();
    try std.testing.expectEqual(@as(u32, 17), branched.success);
    var looped = fx.run(&env, Nodes.looped);
    defer looped.deinit();
    try std.testing.expectEqual(@as(u32, 3), looped.success);
    var scoped = fx.run(&env, Nodes.scoped);
    defer scoped.deinit();
    try std.testing.expectEqual(@as(u32, 17), scoped.success);
    try std.testing.expectEqual(@as(usize, 1), releases);
}

test "INF-11 CAP-08 program context inference preserves generated ownership for noncloneable unmarked composite results" {
    const Token = struct {
        pub const effect_owned_payload = true;
        live: *usize,
        pub fn deinit(self: *@This()) void {
            self.live.* -= 1;
        }
    };
    const State = struct { token: Token, number: u32 };
    const Nodes = struct {
        fn produce(_: *const R) fx.Effect(State, E, R) {
            return fx.Effect(State, E, R).fromOutcome(struct {
                fn run(env: *const R) fx.Exit(State, E) {
                    env.releases.?.* += 1;
                    return .{ .success = .{ .token = .{ .live = env.releases.? }, .number = env.input } };
                }
            }.run);
        }
    };
    const described = comptime p.describe(.{p.binding("answer", .{}, Nodes.produce)});
    comptime if (@TypeOf(described) != fx.Effect(fx.dsl.ownership.Capture(State), E, R)) @compileError("inference must preserve generated payload ownership");
    var live: usize = 0;
    const env: R = .{ .allocator = std.testing.allocator, .input = 17, .releases = &live };
    var result = fx.run(&env, described);
    try std.testing.expectEqual(@as(usize, 1), live);
    try std.testing.expectEqual(@as(u32, 17), result.success.value.number);
    result.deinit();
    try std.testing.expectEqual(@as(usize, 0), live);
}
