const fx = @import("effect");
const std = @import("std");
const R = struct { allocator: std.mem.Allocator, io: std.Io };
const P = fx.Effect(u32, void, R);
fn initialize(_: *const R) u32 {
    return 0;
}
fn finish(value: *u32, _: *const R) u32 {
    return value.*;
}
const source = fx.dsl.streams.yieldEffect(P.succeed(1));
fn step(value: *u32, item: *const u32, _: *const R) fx.SinkBuilder.Control {
    value.* += item.*;
    return .proceed;
}
pub fn main() void {
    _ = fx.SinkBuilder.fold(.{ .initialize = initialize, .step = step, .finish = finish }).run(source);
}
