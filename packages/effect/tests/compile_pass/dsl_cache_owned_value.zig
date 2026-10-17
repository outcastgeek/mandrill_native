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
const Payload = fx.dsl.ownership.Capture(struct { token: Owner });
fn configure(_: *const void, _: *const Env) fx.CacheConfig(u32, void) {
    return .{ .keys = .{ .userdata = null, .equal = struct {
        fn equal(_: ?*anyopaque, a: u32, b: u32) bool {
            return a == b;
        }
    }.equal, .retain = struct {
        fn retain(_: ?*anyopaque, key: u32) error{OutOfMemory}!u32 {
            return key;
        }
    }.retain, .release = struct {
        fn release(_: ?*anyopaque, _: *u32) void {}
    }.release }, .time_to_live = null, .capacity = 1, .maximum_inflight = 1, .cache_failure = struct {
        fn cache(_: void) bool {
            return false;
        }
    }.cache };
}
fn create(_: *const Env) fx.Exit(Payload, void) {
    return .{ .success = .{ .value = .{ .token = .{} } } };
}
fn load(_: *const void, _: u32, _: *const Env) fx.Effect(Payload, void, Env) {
    return fx.Effect(Payload, void, Env).fromOutcome(create);
}
pub fn main() void {
    const env: Env = .{ .allocator = std.heap.page_allocator, .io = undefined, .async_engine = undefined };
    var scope = fx.Scope(void).init(&env);
    var described = fx.dsl.makeCache(&env, &scope, @as(void, {}), .{ .configure = configure, .load = load });
    described.deinit();
}
