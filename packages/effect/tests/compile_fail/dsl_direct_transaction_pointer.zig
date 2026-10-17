const std = @import("std");
const fx = @import("effect");
const E = enum { expected };
const R = struct { allocator: std.mem.Allocator };
const P = fx.Effect(u32, E, R);
const T = fx.TransactionBuilder;
const S = @TypeOf(T.describe(*const u32, E, {}, native));
fn native(_: *const void, _: *fx.TransactionDomain.Journal) fx.TransactionAtomicDecision(*const u32, E) {
    return .{ .commit = &@as(u32, 1) };
}
fn map(_: *const u32) u32 {
    return 1;
}
pub fn main() void {
    const bad = T.Mapped(S, u32, map){ .source = T.describe(*const u32, E, {}, native) };
    _ = bad;
}
