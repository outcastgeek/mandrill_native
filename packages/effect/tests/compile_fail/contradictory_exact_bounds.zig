const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator };
pub fn main() void {
    _ = fx.JsonNumberCodec.exact(Env, .{ .integer = true, .minimum = "2", .exclusive_maximum = "2", .maximum_integer_digits = 8, .maximum_value_bytes = 16 });
}
