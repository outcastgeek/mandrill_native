const std = @import("std");
const fx = @import("effect");
const E = enum { expected };
const R = struct { allocator: std.mem.Allocator, io: std.Io };
const State = struct {
    pub const effect_owned_payload = true;
    pub fn deinit(_: *@This()) void {}
};
fn step(_: *State, _: E, _: std.Io.Duration, _: f64) fx.Schedule.Decision(void) {
    return .{ .stop = {} };
}
pub fn main() void {
    _ = (comptime fx.Effect(u32, E, R).succeed(1)).retry(fx.Schedule.unfold(E, void, State{}, false, step));
}
