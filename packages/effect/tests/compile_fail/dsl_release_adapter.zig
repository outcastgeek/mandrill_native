const std = @import("std");
const fx = @import("effect");
const R = struct { allocator: std.mem.Allocator };
const Adapter = struct {
    pub fn release(_: u32, _: fx.PrimaryOutcome(void)) fx.Effect(u32, void, R) {
        return fx.Effect(u32, void, R).succeed(1);
    }
};
pub fn main() void {
    _ = fx.dsl.resourceBinding(.{ .acquire = fx.Effect(u32, void, R).succeed(1), .adapter = Adapter });
}
