const std = @import("std");
const fx = @import("effect");
const E = enum { expected };
const R = struct { allocator: std.mem.Allocator };
const P = fx.Effect(u32, E, R);
const T = fx.TransactionBuilder;
fn map(value: u32) u64 {
    return value;
}
pub fn main() void {
    const Source = @TypeOf(T.succeed(u32, E, 1));
    _ = T.Mapped(Source, u64, map){ .source = T.succeed(u32, E, 1) };
}
