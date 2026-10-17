const std = @import("std");
const fx = @import("effect");
const p = fx.dsl.program;
const E = enum { expected };
const R = struct { allocator: std.mem.Allocator, io: std.Io };
fn first(_: *const R) fx.Effect(u32, E, R) {
    return fx.Effect(u32, E, R).succeed(1);
}
fn generic(env: anytype) fx.Effect(u32, E, @typeInfo(@TypeOf(env)).pointer.child) {
    return fx.Effect(u32, E, @typeInfo(@TypeOf(env)).pointer.child).succeed(1);
}
const Other = struct { allocator: std.mem.Allocator, io: std.Io };
pub fn main() void {
    _ = p.describeWith(.{ .environment = Other }, .{p.binding("answer", .{}, first)});
}
