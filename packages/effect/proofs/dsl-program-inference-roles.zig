const std = @import("std");
const fx = @import("effect");
const p = fx.dsl.program;
const E = enum { expected };
const R = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, input: u32, releases: *usize, calls: *usize };
fn EnvOf(comptime T: type) type {
    return @typeInfo(T).pointer.child;
}

test "INF-03 INF-12 INF-13 loop condition supplies the only environment evidence to generic initializer body and finish roles" {
    const Roles = struct {
        fn initialize(env: anytype) u32 {
            env.calls.* += 1;
            return 0;
        }
        fn condition(value: *const u32, env: *const R) bool {
            return value.* < env.input;
        }
        fn body(view: anytype, env: anytype) fx.Effect(p.LoopControl(u32), E, EnvOf(@TypeOf(env))) {
            env.calls.* += 1;
            view.state().* += 1;
            return fx.Effect(p.LoopControl(u32), E, EnvOf(@TypeOf(env))).succeed(.again);
        }
        fn finish(view: anytype, env: anytype) u32 {
            env.calls.* += 1;
            return view.state().*;
        }
    };
    const described = comptime p.describe(.{p.loop("answer", .{}, Roles.initialize, Roles.condition, Roles.body, Roles.finish)});
    comptime if (@TypeOf(described) != fx.Effect(u32, E, R)) @compileError("loop condition must infer exact context without an anchor");
    var releases: usize = 0;
    var calls: usize = 0;
    const env: R = .{ .allocator = std.testing.allocator, .input = 4, .releases = &releases, .calls = &calls };
    var result = fx.run(&env, described);
    defer result.deinit();
    try std.testing.expectEqual(@as(u32, 4), result.success);
    try std.testing.expectEqual(@as(usize, 6), calls);
}

test "INF-03 INF-12 INF-13 lexical release return supplies the only environment evidence without interpreting its resource pointer as an environment" {
    const Roles = struct {
        fn acquire(env: anytype) fx.Effect(*const u32, E, EnvOf(@TypeOf(env))) {
            env.calls.* += 1;
            return fx.Effect(*const u32, E, EnvOf(@TypeOf(env))).succeed(&env.input);
        }
        fn release(_: *const u32) fx.Effect(void, E, R) {
            return fx.Effect(void, E, R).fromOutcome(struct {
                fn close(env: *const R) fx.Exit(void, E) {
                    env.releases.* += 1;
                    return .{ .success = {} };
                }
            }.close);
        }
        fn read(value: anytype, env: anytype) fx.Effect(u32, E, EnvOf(@TypeOf(env))) {
            env.calls.* += 1;
            return fx.Effect(u32, E, EnvOf(@TypeOf(env))).succeed(value.*.*);
        }
    };
    const described = comptime p.describe(.{ p.resource("lease", .{}, Roles.acquire, Roles.release), p.binding("answer", .{"lease"}, Roles.read) });
    comptime if (@TypeOf(described) != fx.Effect(u32, E, R)) @compileError("release return must infer exact context without an anchor");
    var releases: usize = 0;
    var calls: usize = 0;
    const env: R = .{ .allocator = std.testing.allocator, .input = 4, .releases = &releases, .calls = &calls };
    var result = fx.run(&env, described);
    defer result.deinit();
    try std.testing.expectEqual(@as(u32, 4), result.success);
    try std.testing.expectEqual(@as(usize, 1), releases);
    try std.testing.expectEqual(@as(usize, 2), calls);
}
