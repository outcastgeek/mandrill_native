const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator };
pub fn main() void {
    _ = fx.dsl.schemaRecord(Env, .{ .fields = .{ .answer = .{ .codec = fx.JsonCodec.signed_integer, .presence = .required, .default = @as(i64, 7) } }, .policy = .{ .unknown_fields = .reject, .duplicate_fields = .reject, .issues = .first } });
}
