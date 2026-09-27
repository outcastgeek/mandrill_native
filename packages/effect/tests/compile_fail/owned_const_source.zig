const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator };
const P = fx.Effect(u16, u16, Env);

fn next(value: u16) P {
    return P.succeed(value);
}

pub fn main() void {
    const env: Env = .{ .allocator = std.heap.page_allocator };
    const source = P.succeed(7);
    _ = source.flatMapOwned(&env, next);
}
