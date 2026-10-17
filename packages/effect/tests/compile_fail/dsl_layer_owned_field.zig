const std = @import("std");
const fx = @import("effect");
const R = struct { allocator: std.mem.Allocator };
const Owner = struct {
    pub const effect_owned_payload = true;
    pub fn deinit(_: *@This()) void {}
};
const Services = struct { owner: Owner };
fn acquire(_: *const R) fx.Exit(Services, void) {
    return .{ .success = .{ .owner = .{} } };
}
pub fn main() void {
    _ = fx.dsl.layer(.{ .acquire = fx.Effect(Services, void, R).fromOutcome(acquire) });
}
