const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator, io: std.Io };
const Owner = struct {
    pub const effect_owned_payload = true;
    pub fn deinit(_: *@This()) void {}
};
const Changed = fx.dsl.RefChange(u32, struct { owner: Owner });
fn create(_: *const Env) fx.Exit(Changed, void) {
    return .{ .success = .{ .result = .{ .owner = .{} }, .value = 4 } };
}
fn run(_: *const void, _: u32, _: *const Env) fx.Effect(Changed, void, Env) {
    return fx.Effect(Changed, void, Env).fromOutcome(create);
}
pub fn main() void {
    const env: Env = .{ .allocator = std.heap.page_allocator, .io = undefined };
    var reference = fx.Ref(u32).init(&env, 3);
    _ = fx.dsl.modifyRefEffect(&env, &reference, @as(void, {}), .{ .run = run });
}
