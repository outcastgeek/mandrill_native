const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator, io: std.Io };
const Owner = struct {
    pub const effect_owned_payload = true;
    pub fn deinit(_: *@This()) void {}
};
pub fn main() void {
    const env: Env = .{ .allocator = std.heap.page_allocator, .io = undefined };
    var reference = fx.Ref(struct { owner: Owner }).init(&env, .{ .owner = .{} });
    _ = fx.Dsl(void, Env).getRef(&reference);
}
