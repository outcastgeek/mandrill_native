const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator };
pub fn main() void {
    _ = fx.dsl.schemaRecord(Env, .{ .value = struct { answer: i64 }, .fields = .{ .answer = .{ .codec = fx.JsonCodec.signed_integer, .presence = .required } }, .policy = .{ .unknown_fields = .reject, .duplicate_fields = .reject, .issues = .first } });
}
