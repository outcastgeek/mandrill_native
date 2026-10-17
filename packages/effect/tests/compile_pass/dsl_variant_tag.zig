const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator };
pub fn main() void {
    _ = fx.dsl.variant(Env, .{ .discriminator = "type", .payload = "data", .cases = .{
        .first = .{ .tag = "value-v1", .codec = fx.JsonCodec.signed_integer },
        .second = .{ .tag = "flag-v1", .codec = fx.JsonCodec.boolean },
    } });
}
