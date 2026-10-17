const fx = @import("effect");
const E = enum { invalid };
const F = enum { different };
fn decode(input: *const u8) fx.Result(u16, E) {
    return .{ .ok = input.* };
}
fn encode(value: *const u16) fx.Result(u8, F) {
    return .{ .ok = @intCast(value.*) };
}
pub fn main() void {
    _ = fx.dsl.codec(.{ .kind = .pure, .decode = decode, .encode = encode });
}
