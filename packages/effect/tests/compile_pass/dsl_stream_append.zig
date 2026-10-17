const std = @import("std");
const fx = @import("effect");
const E = enum { expected };
const R = struct { allocator: std.mem.Allocator, io: std.Io };
const Number = fx.Effect(u32, E, R);
const source = fx.dsl.streams.yieldEffect(Number.succeed(1));
pub fn main() void {
    _ = fx.dsl.streams.append(.{ source, source });
}
