const std = @import("std");
const fx = @import("effect");
const p = fx.dsl.program;
const R = struct { allocator: std.mem.Allocator, io: std.Io };
fn value(input: *const u32, _: *const R) u32 {
    return input.*;
}
fn predicate(_: *const u32, _: *const R) bool {
    return true;
}
pub fn main() void {
    const env: R = .{ .allocator = std.heap.page_allocator, .io = undefined };
    var described = p.captured(&env, @as(u32, 1), .{ p.native("value", .{"inputs"}, value), p.returnWhen("stop", .{"value"}, predicate, "inputs"), p.native("answer", .{"value"}, value) });
    described.deinit();
}
