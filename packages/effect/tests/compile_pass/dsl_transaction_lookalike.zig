const std = @import("std");
const fx = @import("effect");
const E = enum { expected };
const R = struct { allocator: std.mem.Allocator };
const P = fx.Effect(u32, E, R);
const T = fx.TransactionBuilder;
pub fn main() void {
    _ = T.succeed(u32, E, 0).zip(T.succeed(u32, E, 1));
}
