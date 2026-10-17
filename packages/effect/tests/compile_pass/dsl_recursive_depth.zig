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
    const Recursive = fx.dsl.recursiveSchema(Env, .{ .value = Node, .max_depth = 1, .reference = "node-v1", .schema = schema });
    const env: Env = .{ .allocator = std.heap.page_allocator };
    const input: std.json.Value = .{ .object = .{} };
    var result = fx.run(&env, Recursive.decode(&input));
    result.deinit();
}
