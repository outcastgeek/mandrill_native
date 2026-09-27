const fx = @import("effect");
const P = fx.Effect(u16, u32, void);
fn body(_: u16) P {
    return P.succeed(3);
}
fn release(_: u16, _: fx.PrimaryOutcome(u32)) fx.Effect(void, u32, void) {
    return .succeed({});
}
pub fn main() void {
    const program = comptime P.succeed(3).acquireRelease(body, release);
    _ = program;
}
