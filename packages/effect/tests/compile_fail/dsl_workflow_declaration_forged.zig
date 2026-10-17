const std = @import("std");
const fx = @import("effect");
const R = struct { allocator: std.mem.Allocator };
const integer = fx.JsonCodec.signed_integer;
const nothing = fx.JsonCodec.null_value;
const description: fx.WorkflowTypes.Definition = .{ .name = "negative.workflow", .version = 1, .payload_schema_version = 1, .success_schema_version = 1, .error_schema_version = 1 };
const Fake = struct {
    pub const Success = i64;
    pub const Error = void;
    pub const Environment = R;
};
fn handler(_: *const i64, _: *const R) Fake {
    return .{};
}
pub fn main() void {
    _ = fx.dsl.workflow(.{ .description = description, .payload = integer, .success = integer, .failure = nothing, .handler = handler });
}
