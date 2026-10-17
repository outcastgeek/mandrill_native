const std = @import("std");
const fx = @import("effect");
const E = enum { expected };
const R = struct { allocator: std.mem.Allocator };
const P = fx.Effect(u32, E, R);
const W = fx.WorkflowBuilder(E, R);
pub fn main() void {
    const good = comptime W.succeed(@as(u32, 1));
    _ = W.run(good);
}
