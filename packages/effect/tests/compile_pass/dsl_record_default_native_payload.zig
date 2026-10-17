const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator };
fn default(_: *const Env) i64 {
    return 7;
}
pub fn main() void {
    const Record = fx.dsl.schemaRecord(Env, .{ .fields = .{ .answer = .{ .codec = fx.JsonCodec.signed_integer, .presence = .defaulted, .default = default } }, .policy = .{ .unknown_fields = .reject, .duplicate_fields = .reject, .issues = .first } });
    _ = Record.decode;
}
