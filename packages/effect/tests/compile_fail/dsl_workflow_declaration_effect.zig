const std = @import("std");
const fx = @import("effect");
const R = struct { allocator: std.mem.Allocator };
const integer = fx.JsonCodec.signed_integer;
const nothing = fx.JsonCodec.null_value;
const description: fx.WorkflowTypes.Definition = .{ .name = "negative.workflow", .version = 1, .payload_schema_version = 1, .success_schema_version = 1, .error_schema_version = 1 };
fn handler(input: *const i64, _: *const R) fx.Effect(i64, void, R) {
    return fx.Effect(i64, void, R).succeed(input.*);
}
pub fn main() void {
    _ = fx.dsl.workflow(.{ .description = description, .payload = integer, .success = integer, .failure = nothing, .handler = handler });
}
