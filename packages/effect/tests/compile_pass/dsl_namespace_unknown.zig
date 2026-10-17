const std = @import("std");
const fx = @import("effect");
const E = enum { expected };
const R = struct { allocator: std.mem.Allocator };
const P = fx.Effect(u32, E, R);
const Roles = struct {
    pub fn run(_: *const R) u32 {
        return 1;
    }
};
pub fn main() void {
    _ = fx.Dsl(E, R).sync(Roles);
}
