const fx = @import("effect");
pub fn main() void {
    _ = fx.Effect(fx.OwnedSlice(u8), void, void).succeed(.{ .items = undefined, .allocator = undefined });
}
