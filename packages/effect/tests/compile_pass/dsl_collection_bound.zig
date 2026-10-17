const std = @import("std");
const fx = @import("effect");
const R = struct { allocator: std.mem.Allocator, io: std.Io, async_engine: fx.Executor };
pub fn main() void {
    _ = fx.dsl.allConcurrentStatic(.{fx.Effect(u32, void, R).succeed(1)}, 1);
}
