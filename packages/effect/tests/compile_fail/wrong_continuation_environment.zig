const fx = @import("effect");
const Env = struct { value: u32 };
const P = fx.Effect(u32, void, Env);
fn wrong(_: u32, _: *Env) P {
    return P.succeed(7);
}
pub fn main() void {
    const program = comptime P.succeed(3).flatMap(wrong);
    _ = fx.run(&Env{ .value = 1 }, program);
}
