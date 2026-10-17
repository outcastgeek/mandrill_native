const fx = @import("effect");
const E = enum { expected };
const R = struct {};
const P = fx.Effect(u32, E, R);
fn value(_: *const R) u32 {
    return 1;
}
pub fn main() void {
    var source = P.succeed(1);
    _ = fx.dsl.pipeline.owned(&source, &R{}, .{});
}
