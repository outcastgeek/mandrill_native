const std = @import("std");
const fx = @import("effect");
const E = enum { expected };
const Env = struct { allocator: std.mem.Allocator, io: std.Io, parallel_engine: fx.Executor };
const Shared = struct {
    refs: usize = 1,
    fn retain(self: *@This()) *@This() {
        self.refs += 1;
        return self;
    }
    fn release(self: **@This()) void {
        self.*.refs -= 1;
    }
};
const Worker = struct {
    fn run(value: anytype, _: *const Env) fx.Exit(u32, E) {
        var moved = value;
        fx.dsl.ownership.discard(&moved);
        return .{ .success = 1 };
    }
};
pub fn main() !void {
    var shared: Shared = .{};
    const value = .{ .retained = fx.dsl.ownership.retained(&shared, .{ .retain = Shared.retain, .release = Shared.release, .synchronization = fx.dsl.ownership.Synchronization.thread_safe }) };
    const owner = try fx.dsl.captures.OneShot(@TypeOf(value)).create(std.heap.page_allocator, value);
    defer owner.deinit();
    _ = owner.handle().onCpu(Worker.run);
}
