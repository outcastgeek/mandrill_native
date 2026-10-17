const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator, io: std.Io };
const Definition = struct {
    pub const Input = u32;
    pub const Success = u32;
    pub const DomainError = void;
};
const Resolver = fx.RequestResolver(Definition, Env, void);
fn jobs(_: *const void, _: *const Env) []const Resolver.Job {
    return &.{};
}
fn resolve(_: *const void, _: []const Resolver.Job, _: *const Env) fx.Effect(u32, Resolver.Failure, Env) {
    return fx.Effect(u32, Resolver.Failure, Env).succeed(7);
}
pub fn main() void {
    const env: Env = .{ .allocator = std.heap.page_allocator, .io = undefined };
    var described = fx.dsl.resolveBatch(Resolver, &env, @as(void, {}), .{ .jobs = jobs, .resolve = resolve });
    described.deinit();
}
