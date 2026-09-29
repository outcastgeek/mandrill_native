const std = @import("std");
const fx = @import("effect");

const Env = struct { allocator: std.mem.Allocator, io: std.Io };
const source = fx.TransactionBuilder.succeed(u8, void, 1);

fn nativeWork(value: u8) fx.Effect(u8, void, Env) {
    return fx.Effect(u8, void, Env).succeed(value);
}

pub fn main() void {
    _ = source.flatMap(fx.Effect(u8, void, Env), nativeWork);
}
