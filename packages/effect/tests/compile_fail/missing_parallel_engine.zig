const std = @import("std");
const fx = @import("effect");
const R = struct { allocator: std.mem.Allocator, io: std.Io };
fn callback(_: *const R, _: *const fx.Cancellation) fx.Exit(void, void) {
    return .{ .success = {} };
}
pub fn main() void {
    const environment: R = .{ .allocator = std.heap.page_allocator, .io = undefined };
    _ = fx.run(&environment, comptime fx.Effect(void, void, R).onCpu(callback));
}
