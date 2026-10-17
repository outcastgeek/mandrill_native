const fx = @import("effect");
const Issue = fx.SchemaIssue.Issue;
fn forward(value: i64) fx.Result(u16, Issue) {
    return .{ .ok = @intCast(value) };
}
fn backward(_: *const u16) fx.Result(bool, Issue) {
    return .{ .ok = true };
}
pub fn main() void {
    _ = fx.dsl.transformCodec(fx.JsonCodec.signed_integer, .{ .forward = forward, .backward = backward });
}
