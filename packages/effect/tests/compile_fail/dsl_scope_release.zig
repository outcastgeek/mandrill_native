const std = @import("std");
const fx = @import("effect");
const R = struct { allocator: std.mem.Allocator, io: std.Io };
fn acquire(_: *const u32, _: *const R) fx.Effect(u32, void, R) {
    return fx.Effect(u32, void, R).succeed(3);
}
fn release(_: *const u32, _: u32, _: *const R) fx.Effect(u32, void, R) {
    return fx.Effect(u32, void, R).succeed(3);
}
pub fn main() void {
    const env: R = .{ .allocator = std.heap.page_allocator, .io = undefined };
    var scope = fx.Scope(void).init(&env);
    _ = fx.dsl.scopeAcquire(&env, &scope, @as(u32, 1), .{ .acquire = acquire, .release = release });
}
