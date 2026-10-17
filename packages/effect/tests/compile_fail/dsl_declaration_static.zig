const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator };
const Record = fx.dsl.schemaRecord(Env, .{ .fields = .{ .number = .{ .codec = fx.JsonCodec.signed_integer, .presence = .required } }, .policy = .{ .unknown_fields = .reject, .duplicate_fields = .reject, .issues = .first } });
const budget: fx.SchemaBuilder.ParseLimits = .{ .max_document_bytes = 64, .max_depth = 4, .max_value_bytes = 32 };
fn text() []const u8 {
    return "{}";
}
const metadata = .{ .schema = Record, .text = text, .limits = budget };
pub fn main() void {
    const env: Env = .{ .allocator = std.heap.page_allocator };
    _ = fx.dsl.parseDeclaration(&env, .{ .definition = metadata, .inputs = @as(void, {}) });
}
