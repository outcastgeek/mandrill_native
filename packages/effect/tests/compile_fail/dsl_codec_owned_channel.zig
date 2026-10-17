const fx = @import("effect");
const E = enum { invalid };
const F = enum { different };
const Owner = struct {
    pub const effect_owned_payload = true;
    pub fn deinit(_: *@This()) void {}
};
const Value = struct { token: Owner };
fn create(_: *const void) fx.Exit(Value, E) {
    return .{ .success = .{ .token = .{} } };
}
fn decode(_: *const u8) fx.Effect(Value, E, void) {
    return fx.Effect(Value, E, void).fromOutcome(create);
}
fn encode(_: *const Value) fx.Effect(u8, E, void) {
    return fx.Effect(u8, E, void).succeed(7);
}
pub fn main() void {
    _ = fx.dsl.codec(.{ .kind = .contextual, .decode = decode, .encode = encode });
}
