const fx = @import("effect");
fn body(value: fx.ResourceValue(u32)) fx.Effect(u32, void, void) {
    return fx.Effect(u32, void, void).succeed(value.value);
}
pub fn main() void {
    _ = fx.dsl.using(fx.ResourceValue(u32){ .value = 1 }, .{ .body = body });
}
