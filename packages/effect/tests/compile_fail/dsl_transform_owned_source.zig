const std = @import("std");
const fx = @import("effect");
const Issue = fx.SchemaIssue.Issue;
const Owned = fx.OwnedSlice(u8);
const source = fx.PureCodec(Owned, std.json.Value).init(struct {
    fn decode(_: *const std.json.Value) fx.Result(Owned, Issue) {
        return .{ .err = Issue.pure(.decode, .invalid_value) };
    }
}.decode, struct {
    fn encode(value: *const Owned) fx.Result(std.json.Value, Issue) {
        return .{ .ok = .{ .string = value.items } };
    }
}.encode);
fn forward(value: Owned) fx.Result(u64, Issue) {
    var owned = value;
    const length = owned.items.len;
    owned.deinit();
    return .{ .ok = length };
}
fn backward(_: *const u64) fx.Result(Owned, Issue) {
    return .{ .err = Issue.pure(.encode, .invalid_value) };
}
pub fn main() void {
    _ = fx.dsl.transformCodec(source, .{ .forward = forward, .backward = backward });
}
