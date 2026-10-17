const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator };
const Record = fx.dsl.schemaRecord(Env, .{ .fields = .{ .number = .{ .codec = fx.JsonCodec.signed_integer, .presence = .required } }, .policy = .{ .unknown_fields = .reject, .duplicate_fields = .reject, .issues = .first } });
fn text() []const u8 {
    return "{}";
}
const limits = .{ .max_document_bytes = 0, .max_depth = 4, .max_value_bytes = 32, .max_integer_digits = 8 };
pub fn main() void {
    const env: Env = .{ .allocator = std.heap.page_allocator };
    _ = fx.dsl.parseSchema(&env, @as(void, {}), .{ .schema = Record, .text = text, .limits = limits });
}
