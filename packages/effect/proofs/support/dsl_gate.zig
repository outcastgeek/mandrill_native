//! Deterministic pending native operation for DSL lifecycle proofs.
const std = @import("std");
const fx = @import("effect");
pub fn Gate(comptime A: type, comptime E: type) type {
    return struct {
        const Self = @This();
        const Native = fx.Async(A, E);
        io: std.Io = std.testing.io,
        started: std.Io.Event = .unset,
        proceed: std.Io.Event = .unset,
        cancelled: std.atomic.Value(bool) = .init(false),
        joined: std.atomic.Value(bool) = .init(false),
        destroyed: std.atomic.Value(bool) = .init(false),
        result: fx.Exit(A, E),
        pub fn start(self: *Self) Native.Start {
            self.started.set(self.io);
            return .{ .pending = .{ .userdata = self, .vtable = &vtable } };
        }
        fn request(data: *anyopaque) ?fx.Defect {
            const self: *Self = @ptrCast(@alignCast(data));
            self.cancelled.store(true, .release);
            self.proceed.set(self.io);
            return null;
        }
        fn join(data: *anyopaque, _: bool) Native.JoinResult {
            const self: *Self = @ptrCast(@alignCast(data));
            self.proceed.waitUncancelable(self.io);
            self.joined.store(true, .release);
            return .{ .exit = if (self.cancelled.load(.acquire)) .interrupted else self.result };
        }
        fn destroy(data: *anyopaque) void {
            const self: *Self = @ptrCast(@alignCast(data));
            std.debug.assert(self.joined.load(.acquire));
            self.destroyed.store(true, .release);
        }
        const vtable: Native.Operation.VTable = .{ .request = request, .join = join, .destroy = destroy };
    };
}
