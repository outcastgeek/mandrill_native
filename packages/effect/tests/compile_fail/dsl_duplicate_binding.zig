const fx = @import("effect");
const E = enum { expected };
const R = struct {};
const P = fx.Effect(u32, E, R);
fn value(_: *const R) u32 {
    return 1;
}
fn first(_: *const R) P {
    return P.succeed(1);
}
pub fn main() void {
    _ = fx.dsl.program.build(E, R, .{ fx.dsl.program.binding("a", .{}, first), fx.dsl.program.binding("a", .{}, first) });
}
