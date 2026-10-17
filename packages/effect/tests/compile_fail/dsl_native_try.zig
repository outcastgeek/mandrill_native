const fx = @import("effect");
const R = struct {};
const P = fx.Effect(u32, void, R);
pub fn main() !void {
    _ = try P.succeed(1);
}
