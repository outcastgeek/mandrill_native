const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator };
const P = fx.Effect(u16, u32, Env);
fn body(_: u16) P {
    return P.succeed(3);
}
fn invalidRelease(_: u16) fx.Effect(void, u32, Env) {
    return .succeed({});
}
pub fn main() void {
    const program = comptime P.succeed(3).acquireRelease(body, invalidRelease);
    _ = program;
}
