const std = @import("std");
const fx = @import("effect");
const p = fx.dsl.program;
const E = enum { expected };
const R = struct { allocator: std.mem.Allocator, io: std.Io };
const P = fx.Effect(u32, E, R);
fn first(_: *const R) P {
    return P.succeed(1);
}
fn next(value: *const u32, _: *const R) P {
    return P.succeed(value.* + 1);
}
fn always(_: *const R) bool {
    return true;
}
fn acquire(_: *const R) fx.Effect(*u32, E, R) {
    return fx.Effect(*u32, E, R).interrupted();
}
fn release(_: *u32) fx.Effect(void, E, R) {
    return fx.Effect(void, E, R).succeed({});
}
pub fn main() void {
    _ = p.build(E, R, .{p.resource("lease", .{}, acquire, release)});
}
