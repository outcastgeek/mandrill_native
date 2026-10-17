const std = @import("std");
const fx = @import("effect");
const E = enum { expected };
const R = struct { allocator: std.mem.Allocator };
const P = fx.Effect(u32, E, R);
fn work(_: *const R) P {
    return P.succeed(1);
}
pub fn main() void {
    _ = fx.Dsl(E, R).factory(.{ .run = work });
}
