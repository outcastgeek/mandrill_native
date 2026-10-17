const fx = @import("effect");
const E = enum { expected };
const R = struct {};
const P = fx.Effect(u32, E, R);
fn value(_: *const R) u32 {
    return 1;
}
const T = fx.TransactionBuilder;
const Fake = struct {
    pub const Success = u32;
    pub const Error = E;
    pub fn evaluate(_: *const @This(), _: *fx.TransactionDomain.Journal) fx.TransactionAtomicDecision(u32, E) {
        return .{ .commit = 1 };
    }
};
pub fn main() void {
    _ = T.succeed(u32, E, 0).zip(Fake{});
}
