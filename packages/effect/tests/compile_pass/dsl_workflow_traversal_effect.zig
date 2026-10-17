const fx = @import("effect");
const std = @import("std");
const R = struct { allocator: std.mem.Allocator };
const E = enum { invalid };
const W = fx.WorkflowBuilder(E, R);
const Row = struct { id: []const u8 };
const rows = [_]Row{ .{ .id = "one" }, .{ .id = "two" } };
fn execution(_: *const void, _: *const R) fx.WorkflowTypes.ExecutionId {
    return .{ .encoded = "" };
}
fn body(_: *const void, _: *const Row, _: *const W.Iteration, _: *const R) W.Step(u32) {
    return W.succeed(@as(u32, 1));
}
fn map(_: W.TraversalError) E {
    return .invalid;
}
pub fn main() void {
    const env: R = .{ .allocator = undefined };
    _ = W.forEach(&env, {}, .{ .name = "rows", .version = 1, .key_field = "id", .execution = execution, .static_items = rows, .body = body, .map_error = map });
}
