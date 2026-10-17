const std = @import("std");
const fx = @import("effect");
pub fn main() void {
    _ = fx.dsl.permits.semaphoreStatic(.{ .io = @as(std.Io, undefined) }, 1);
}
