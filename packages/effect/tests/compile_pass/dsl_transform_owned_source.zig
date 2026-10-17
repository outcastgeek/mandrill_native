const fx = @import("effect");
const Issue = fx.SchemaIssue.Issue;
fn forward(value: []const u8) fx.Result(u64, Issue) {
    return .{ .ok = value.len };
}
fn backward(_: *const u64) fx.Result([]const u8, Issue) {
    return .{ .err = Issue.pure(.encode, .invalid_value) };
}
pub fn main() void {
    _ = fx.dsl.transformCodec(fx.JsonCodec.borrowed_string, .{ .forward = forward, .backward = backward });
}
