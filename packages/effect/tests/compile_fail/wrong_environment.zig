const fx = @import("effect");
const P = fx.Effect(u16, void, void);
fn wrongEnvironment(_: u16) fx.Effect(u16, void, struct { port: u16 }) {
    return .succeed(7);
}
pub fn main() void {
    const program = comptime P.succeed(3).flatMap(wrongEnvironment);
    _ = fx.run(&{}, program);
}
