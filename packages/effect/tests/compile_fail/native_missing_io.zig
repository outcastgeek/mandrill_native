const std = @import("std");
const fx = @import("effect");
const R = struct { allocator: std.mem.Allocator };
fn callback(_: *const void, _: *const R) fx.Exit(void, void) {
    return .{ .success = {} };
}
pub fn main() void {
    _ = fx.Effect(void, void, R).fromNativeBorrowed(void, &{}, callback);
}
