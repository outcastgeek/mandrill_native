// This bounded TTL cache keeps key values until eviction, expiry or
// invalidation. It supplies no weak-key, GC or ephemeron semantics.
const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator, io: std.Io, async_engine: fx.Executor };
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
    }.release }, .time_to_live = .fromSeconds(30), .capacity = 1, .maximum_inflight = 1, .cache_failure = struct {
        fn cache(_: void) bool {
            return false;
        }
    }.cache };
}
fn load(_: *const void, key: u32, _: *const Env) fx.Effect(u32, void, Env) {
    return fx.Effect(u32, void, Env).succeed(key);
}
pub fn main() void {
    const env: Env = .{ .allocator = std.heap.page_allocator, .io = undefined, .async_engine = undefined };
    var scope = fx.Scope(void).init(&env);
    var described = fx.dsl.makeCache(&env, &scope, @as(void, {}), .{ .configure = configure, .load = load });
    described.deinit();
}
