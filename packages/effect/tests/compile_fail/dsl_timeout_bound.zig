const std = @import("std");
const fx = @import("effect");
fn timedOut() void {}
pub fn main() void {
    _ = fx.dsl.timeout(fx.Effect(u32, void, void).succeed(1), .{ .duration = std.Io.Duration{ .nanoseconds = -1 }, .on_timeout = timedOut });
}
