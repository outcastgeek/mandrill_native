const fx = @import("effect");
const R = struct {};
const P = fx.Effect(u32, void, R);
fn key(_: *const R) []const u8 {
    return "";
}
fn read(_: *const R) P {
    return P.succeed(1);
}
const atom = fx.Atom(u32, void, R).readOnly("value", 1, key, &.{}, read);
fn map(value: u32) u64 {
    return value;
}
pub fn main() void {
    _ = fx.AtomProgram.map(fx.AtomProgram.read(atom), map);
}
