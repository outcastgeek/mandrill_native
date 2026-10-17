const fx = @import("effect");
const R = struct {};
const P = fx.Effect(u32, void, R);
fn run(_: *const R) u32 {
    return 1;
}
pub fn main() void {
    _ = fx.Dsl(void, R).sync(.{ .run = run });
}
