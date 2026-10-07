const fx = @import("effect");
pub fn main() void {
    _ = fx.JsonCodec.requireWriter(fx.PureCodec(u8, u8).init(struct {
        fn f(v: *const u8) fx.Result(u8, fx.SchemaIssue.Issue) {
            return .{ .ok = v.* };
        }
    }.f, struct {
        fn f(v: *const u8) fx.Result(u8, fx.SchemaIssue.Issue) {
            return .{ .ok = v.* };
        }
    }.f));
}
