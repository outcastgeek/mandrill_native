const std = @import("std");
const fx = @import("effect");
const p = fx.dsl.program;
const E = enum { expected };
const R = struct { allocator: std.mem.Allocator, io: std.Io, input: u32 };
const Other = struct { allocator: std.mem.Allocator, io: std.Io, input: u32 };
fn EnvOf(comptime T: type) type {
    return @typeInfo(T).pointer.child;
}
fn initialize(env: anytype) u32 {
    _ = env;
    return 0;
}
fn condition(value: *const u32, env: *const R) bool {
    return value.* < env.input;
}
fn body(view: anytype, _: *const Other) fx.Effect(p.LoopControl(u32), E, Other) {
    view.state().* += 1;
    return fx.Effect(p.LoopControl(u32), E, Other).succeed(.again);
}
fn finish(view: anytype, _: anytype) u32 {
    return view.state().*;
}
pub fn main() void {
    _ = p.describe(.{p.loop("answer", .{}, initialize, condition, body, finish)});
}
