const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator, io: std.Io };
fn change(value: u32) struct { result: u32, value: u32 } {
    return .{ .result = value, .value = value + 1 };
}
pub fn main() void {
    const env: Env = .{ .allocator = std.heap.page_allocator, .io = undefined };
    var reference = fx.Ref(u32).init(&env, 4);
    _ = fx.Dsl(void, Env).modifyRef(&reference, .{ .run = change });
}
