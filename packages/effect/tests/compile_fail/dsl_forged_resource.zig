const fx = @import("effect");
const Fake = struct {
    pub const AlreadyOwnedResource = u32;
    value: u32,
};
fn body(value: Fake) fx.Effect(u32, void, void) {
    return fx.Effect(u32, void, void).succeed(value.value);
}
pub fn main() void {
    _ = fx.dsl.using(Fake{ .value = 1 }, .{ .body = body });
}
