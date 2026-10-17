const std = @import("std");
const fx = @import("effect");
const R = struct { allocator: std.mem.Allocator };
const P = fx.Effect(u32, void, R);
pub fn main() !void {
    const env: R = .{ .allocator = std.heap.page_allocator };
    var source = fx.OwnedSlice(P){ .allocator = env.allocator, .items = try env.allocator.alloc(P, 0) };
    var described = fx.dsl.allOwned(&env, &source);
    described.deinit();
}
