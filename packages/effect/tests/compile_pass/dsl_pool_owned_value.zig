const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator, io: std.Io, async_engine: fx.Executor };
const Owner = struct {
    pub const effect_owned_payload = true;
    pub fn deinit(_: *@This()) void {}
};
const Payload = fx.dsl.ownership.Capture(struct { token: Owner });
fn configure(_: *const void, _: *const Env) fx.PoolConfig {
    return .{ .capacity = 1, .idle_time_to_live = null };
}
fn initialize(_: *const Env) fx.Exit(Payload, void) {
    return .{ .success = .{ .value = .{ .token = .{} } } };
}
fn create(_: *const void, _: *const Env) fx.Effect(Payload, void, Env) {
    return fx.Effect(Payload, void, Env).fromOutcome(initialize);
}
fn validate(_: *const void, _: Payload, _: *const Env) fx.Effect(bool, void, Env) {
    return fx.Effect(bool, void, Env).succeed(true);
}
fn dispose(_: *const void, _: Payload, _: *const Env) fx.Effect(void, void, Env) {
    return fx.Effect(void, void, Env).succeed({});
}
pub fn main() void {
    const env: Env = .{ .allocator = std.heap.page_allocator, .io = undefined, .async_engine = undefined };
    var scope = fx.Scope(void).init(&env);
    var described = fx.dsl.makePool(&env, &scope, @as(void, {}), .{ .configure = configure, .create = create, .validate = validate, .dispose = dispose });
    described.deinit();
}
