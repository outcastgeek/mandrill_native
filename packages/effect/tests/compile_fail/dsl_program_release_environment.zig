const std = @import("std");
const fx = @import("effect");
const p = fx.dsl.program;
const E = enum { expected };
const R = struct { allocator: std.mem.Allocator, io: std.Io, input: u32 };
const Other = struct { allocator: std.mem.Allocator, io: std.Io, input: u32 };
fn EnvOf(comptime T: type) type {
    return @typeInfo(T).pointer.child;
}
fn acquire(env: *const R) fx.Effect(*const u32, E, R) {
    return fx.Effect(*const u32, E, R).succeed(&env.input);
}
fn release(_: *const u32) fx.Effect(void, E, Other) {
    return fx.Effect(void, E, Other).succeed({});
}
fn read(value: anytype, env: anytype) fx.Effect(u32, E, EnvOf(@TypeOf(env))) {
    return fx.Effect(u32, E, EnvOf(@TypeOf(env))).succeed(value.*.*);
}
pub fn main() void {
    _ = p.describe(.{ p.resource("lease", .{}, acquire, release), p.binding("answer", .{"lease"}, read) });
}
