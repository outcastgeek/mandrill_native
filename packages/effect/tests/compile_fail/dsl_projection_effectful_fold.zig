const support = @import("support/dsl_projection.zig");
const fx = support.fx;
fn impure(_: *i64, _: *const i64) fx.Effect(void, void, support.R) {
    return fx.Effect(void, void, support.R).succeed({});
}
pub fn main() void {
    _ = fx.dsl.projection(.{ .state = fx.JsonCodec.signed_integer, .cases = .{fx.dsl.eventCase(support.event, impure)}, .limits = support.limits });
}
