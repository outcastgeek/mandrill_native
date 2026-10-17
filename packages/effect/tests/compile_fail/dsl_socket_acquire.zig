const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator, io: std.Io };
const E = enum { expected };
const limits: fx.WebSocket.Limits = .{ .maximum_message_bytes = 8, .receive_buffer_bytes = 4, .close_timeout = .fromSeconds(1) };
fn acquire(_: *const void, _: *const Env) fx.Effect(fx.WebSocket.NativeSocket, E, Env) {
    return fx.Effect(fx.WebSocket.NativeSocket, E, Env).interrupted();
}
fn body(_: *const void, _: *fx.WebSocket.Connection, _: *const Env) fx.Effect(void, fx.WebSocket.Error, Env) {
    return fx.Effect(void, fx.WebSocket.Error, Env).succeed({});
}
pub fn main() void {
    const env: Env = .{ .allocator = std.heap.page_allocator, .io = undefined };
    var described = fx.dsl.withSocket(&env, @as(void, {}), .{ .limits = limits, .acquire = acquire, .body = body });
    described.deinit();
}
