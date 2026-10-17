const std = @import("std");
const fx = @import("effect");
const R = struct { owned: fx.OwnedSlice(u8) };
pub fn main() void {
    _ = fx.dsl.selectors.borrow(void, R, "owned");
}
