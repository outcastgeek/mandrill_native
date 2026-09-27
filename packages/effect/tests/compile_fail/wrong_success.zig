const fx = @import("effect");
const P = fx.Effect(u16, void, void);
fn expectsText(_: []const u8) P {
    return P.succeed(7);
}
pub fn main() void {
    const program = comptime P.succeed(3).flatMap(expectsText);
    _ = fx.run(&{}, program);
}
