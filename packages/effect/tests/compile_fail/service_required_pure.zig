const fx = @import("effect");
const S = struct { value: u8 };
const child = fx.DomainCodec.Directed(u8, u8, void, void, S, true, false).init(struct {
    fn decode(v: *const u8) fx.Result(u8, void) {
        return .{ .ok = v.* };
    }
}.decode, struct {
    fn encode(_: *const u8) fx.Effect(u8, void, S) {
        return fx.Effect(u8, void, S).service(struct {
            fn read(s: *const S) u8 {
                return s.value;
            }
        }.read);
    }
}.encode);
pub fn main() void {
    _ = fx.requirePure(fx.DomainCodec.optional(child));
}
