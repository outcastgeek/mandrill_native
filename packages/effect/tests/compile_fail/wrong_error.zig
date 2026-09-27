const fx = @import("effect");
const P = fx.Effect(u16, u32, void);
fn wrongError(_: u16) fx.Effect(u16, u64, void) {
    return .succeed(7);
}
pub fn main() void {
    const program = comptime P.succeed(3).flatMap(wrongError);
    _ = fx.run(&{}, program);
}
