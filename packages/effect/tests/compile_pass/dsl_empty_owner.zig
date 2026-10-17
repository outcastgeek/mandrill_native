const std = @import("std");
const fx = @import("effect");
const E = enum { expected };
const R = struct { allocator: std.mem.Allocator };
const P = fx.Effect(u32, E, R);
pub fn main() void {
    var source = P.succeed(1);
    var moved = fx.dsl.pipeline.transfer(&source);
    moved.deinit();
}
