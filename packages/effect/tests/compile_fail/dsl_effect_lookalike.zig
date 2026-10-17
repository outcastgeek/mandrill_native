const fx = @import("effect");
const E = enum { expected };
const R = struct {};
const P = fx.Effect(u32, E, R);
fn value(_: *const R) u32 {
    return 1;
}
const Fake = struct {
    pub const Success = u32;
    pub const Error = E;
    pub const Environment = R;
};
fn make(_: *const R) Fake {
    return .{};
}
pub fn main() void {
    _ = fx.Dsl(E, R).factory(.{ .run = make });
}
