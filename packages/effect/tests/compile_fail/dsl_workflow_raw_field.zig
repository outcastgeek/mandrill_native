const fx = @import("effect");
const E = enum { expected };
const R = struct {};
const P = fx.Effect(u32, E, R);
fn value(_: *const R) u32 {
    return 1;
}
const W = fx.WorkflowBuilder(E, R);
pub fn main() void {
    const Step = @TypeOf(W.succeed(@as(u32, 0)));
    const bad = Step{ .program = P.succeed(1) };
    _ = W.run(bad);
}
