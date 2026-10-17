const std = @import("std");
const fx = @import("effect");
const E = enum { expected };
const R = struct { allocator: std.mem.Allocator };
const P = fx.Effect(u32, E, R);
fn run(_: *const R) fx.Exit(u32, E) {
    return .{ .success = 1 };
}
pub fn main() void {
    _ = fx.Dsl(E, R).outcome(.{ .run = run });
}
