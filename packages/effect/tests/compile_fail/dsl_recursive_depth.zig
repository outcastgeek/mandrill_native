const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator };
const Node = struct { value: i64, next: ?fx.dsl.RecursiveBox(@This()) };
fn schema(comptime Context: type) type {
    return fx.dsl.schemaRecord(Context.Environment, .{ .value = Context.Value, .fields = .{
        .value = .{ .codec = fx.JsonCodec.signed_integer, .presence = .required },
        .next = .{ .codec = Context.Schema, .presence = .optional },
    }, .policy = .{ .unknown_fields = .reject, .duplicate_fields = .reject, .issues = .first } });
}
pub fn main() void {
    _ = fx.dsl.recursiveSchema(Env, .{ .value = Node, .max_depth = 0, .reference = "node-v1", .schema = schema });
}
