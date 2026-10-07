const fx = @import("effect");
pub fn main() void {
    _ = fx.NativeCodec.Record(struct { value: u8 }, struct {
        pub const Representation = u8;
    }, void, void, .{}, true);
}
