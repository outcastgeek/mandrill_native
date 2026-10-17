const fx = @import("effect");
const E = enum { invalid };
const F = enum { different };
const Env = struct { service: u8 };
fn decode(input: *const u8) fx.Effect(u16, E, Env) {
    return fx.Effect(u16, E, Env).succeed(input.*);
}
fn encode(value: *const u16) fx.Effect(u8, E, Env) {
    return fx.Effect(u8, E, Env).succeed(@intCast(value.*));
}
pub fn main() void {
    _ = fx.dsl.codec(.{ .kind = .contextual, .decode = decode, .encode = encode });
}
