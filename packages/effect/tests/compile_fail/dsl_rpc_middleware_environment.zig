const std = @import("std");
const fx = @import("effect");
const R = struct { allocator: std.mem.Allocator, io: std.Io };
const Other = struct { allocator: std.mem.Allocator, io: std.Io, other: bool };
const H = struct { allocator: std.mem.Allocator, io: std.Io, value: u32 };
const S = u32;
fn acquire(_: anytype, _: *const R) fx.Effect(S, u8, Other) {
    return fx.Effect(S, u8, Other).succeed(41);
}
fn project(_: anytype, value: *const S, env: *const R) H {
    return .{ .allocator = env.allocator, .io = env.io, .value = value.* };
}
fn use(config: anytype, _: *const H) fx.Effect(u32, u8, H) {
    return fx.Effect(u32, u8, H).succeed(config.answer);
}
fn release(_: anytype, _: *const S, _: *const fx.PrimaryOutcome(fx.RpcMiddlewareFailure(u8, u8)), _: *const R) fx.Effect(void, u8, R) {
    return fx.Effect(void, u8, R).succeed({});
}
pub fn main() void {
    const env: R = .{ .allocator = std.heap.page_allocator, .io = undefined };
    var described = fx.dsl.withRpcMiddleware(&env, .{
        .definition = fx.dsl.declareMetadata(.{ .acquire = acquire, .project = project, .use = use, .release = release }),
        .inputs = .{ .answer = @as(u32, 42) },
    });
    described.deinit();
}
