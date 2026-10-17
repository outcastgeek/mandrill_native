const std = @import("std");
const fx = @import("effect");
const R = struct { allocator: std.mem.Allocator, io: std.Io, async_engine: fx.Executor };
const P = fx.Effect(u32, void, R);
const Shared = struct {
    refs: std.atomic.Value(usize) = .init(1),
    fn retain(self: *@This()) *@This() {
        _ = self.refs.fetchAdd(1, .acq_rel);
        return self;
    }
    fn release(self: **@This()) void {
        _ = self.*.refs.fetchSub(1, .acq_rel);
    }
};
pub fn main() void {
    const env: R = .{ .allocator = std.heap.page_allocator, .io = undefined, .async_engine = undefined };
    var shared: Shared = .{};
    const operations = .{ .retain = Shared.retain, .release = Shared.release, .synchronization = fx.dsl.ownership.Synchronization.thread_safe };
    const retained = fx.dsl.ownership.retained(&shared, operations);
    var values = [_]@TypeOf(retained){retained};
    defer fx.dsl.ownership.discard(&values[0]);
    var described = fx.dsl.forEachConcurrentCaptured(&env, &values, @as(void, {}), 1, .{ .body = struct {
        fn run(_: *const void, _: anytype, _: *const R) P {
            return P.succeed(1);
        }
    }.run });
    described.deinit();
}
