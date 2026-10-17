const std = @import("std");
const fx = @import("effect");
const R = struct { allocator: std.mem.Allocator, io: std.Io };
fn initialize(_: *const R) u8 {
    return 0;
}
fn step(_: *u8, _: u32, _: std.Io.Duration, _: f64) fx.Schedule.Decision(void) {
    return .{ .stop = {} };
}
pub fn main() void {
    _ = fx.dsl.repeat(fx.Effect(u32, void, R).succeed(1), .{ .initialize = initialize, .step = step, .random = true });
}
