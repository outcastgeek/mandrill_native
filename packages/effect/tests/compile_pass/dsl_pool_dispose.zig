const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator, io: std.Io, async_engine: fx.Executor };
const Owner = struct {
    pub const effect_owned_payload = true;
    pub fn deinit(_: *@This()) void {}
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return self.*;
    }
};
fn configure(_: *const void, _: *const Env) fx.PoolConfig {
    return .{ .capacity = 1, .idle_time_to_live = null };
}
fn create(_: *const void, _: *const Env) fx.Effect(u32, void, Env) {
    return fx.Effect(u32, void, Env).succeed(7);
}
fn validate(_: *const void, _: u32, _: *const Env) fx.Effect(bool, void, Env) {
    return fx.Effect(bool, void, Env).succeed(true);
}
fn dispose(_: *const void, _: u32, _: *const Env) fx.Effect(void, void, Env) {
    return fx.Effect(void, void, Env).succeed({});
}
pub fn main() void {
    const env: Env = .{ .allocator = std.heap.page_allocator, .io = undefined, .async_engine = undefined };
    var scope = fx.Scope(void).init(&env);
    var described = fx.dsl.makePool(&env, &scope, @as(void, {}), .{ .configure = configure, .create = create, .validate = validate, .dispose = dispose });
    described.deinit();
}
