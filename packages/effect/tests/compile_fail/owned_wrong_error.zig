const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator };
const P = fx.Effect(u16, u16, Env);

fn wrong(_: u16) fx.Effect(u16, u32, Env) {
    return fx.Effect(u16, u32, Env).succeed(7);
}

pub fn main() void {
    const env: Env = .{ .allocator = std.heap.page_allocator };
    var source = P.succeed(7);
    _ = source.flatMapOwned(&env, wrong);
}
