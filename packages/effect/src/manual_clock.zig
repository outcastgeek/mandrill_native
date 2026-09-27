const diagnostics = @import("internal/diagnostics.zig");
const std = @import("std");
const providers = @import("services.zig");
const outcomes = @import("outcome.zig");

/// An explicitly advanced clock for deterministic schedulers and applications.
/// Sleeping native operations remain cancellable through the injected I/O.
pub const ManualClock = struct {
    const Self = @This();
    const Waiter = struct { next: ?*Waiter = null, deadline: std.Io.Timestamp, event: std.Io.Event = .unset };
    io: std.Io,
    mutex: std.Io.Mutex = .init,
    changed: std.Io.Condition = .init,
    time: std.Io.Timestamp,
    waiters: ?*Waiter = null,
    pub fn init(env: anytype, initial: std.Io.Timestamp) Self {
        return .{ .io = env.io, .time = initial };
    }
    pub fn service(self: *Self) providers.Clock {
        return .{ .userdata = self, .vtable = &.{ .now = now, .sleep = sleep } };
    }
    fn now(data: ?*anyopaque) std.Io.Timestamp {
        const self: *Self = @ptrCast(@alignCast(data.?));
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        return self.time;
    }
    fn sleep(data: ?*anyopaque, duration: std.Io.Duration) providers.Clock.SleepResult {
        const self: *Self = @ptrCast(@alignCast(data.?));
        if (duration.nanoseconds < 0) return .{ .defect = .{ .kind = .invalid_configuration, .message = diagnostics.clock_sleep_duration_must_be_non_negative } };
        self.mutex.lockUncancelable(self.io);
        const end = std.math.add(i96, self.time.nanoseconds, duration.nanoseconds) catch {
            self.mutex.unlock(self.io);
            return .{ .defect = .{ .kind = .invalid_configuration, .message = diagnostics.clock_deadline_overflowed } };
        };
        if (duration.nanoseconds == 0) {
            self.mutex.unlock(self.io);
            return .done;
        }
        var waiter: Waiter = .{ .deadline = .{ .nanoseconds = end }, .next = self.waiters };
        self.waiters = &waiter;
        self.changed.broadcast(self.io);
        self.mutex.unlock(self.io);
        var interrupted = false;
        waiter.event.wait(self.io) catch {
            interrupted = true;
        };
        self.mutex.lockUncancelable(self.io);
        var link = &self.waiters;
        while (link.*) |item| {
            if (item == &waiter) {
                link.* = item.next;
                break;
            }
            link = &item.next;
        }
        self.changed.broadcast(self.io);
        self.mutex.unlock(self.io);
        return if (interrupted) .interrupted else .done;
    }
    pub fn advance(self: *Self, duration: std.Io.Duration) outcomes.Exit(void, outcomes.Defect) {
        if (duration.nanoseconds < 0) return .{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.manual_clock_cannot_move_backwards } };
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        self.time.nanoseconds = std.math.add(i96, self.time.nanoseconds, duration.nanoseconds) catch return .{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.manual_clock_timestamp_overflowed } };
        var current = self.waiters;
        while (current) |waiter| {
            if (waiter.deadline.nanoseconds <= self.time.nanoseconds) waiter.event.set(self.io);
            current = waiter.next;
        }
        return .{ .success = {} };
    }
    pub fn pending(self: *Self) usize {
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        var count: usize = 0;
        var current = self.waiters;
        while (current) |waiter| {
            count += 1;
            current = waiter.next;
        }
        return count;
    }
    pub fn awaitPending(self: *Self, count: usize) outcomes.Exit(void, outcomes.Defect) {
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        while (true) {
            var current = self.waiters;
            var found: usize = 0;
            while (current) |waiter| {
                found += 1;
                current = waiter.next;
            }
            if (found >= count) return .{ .success = {} };
            self.changed.wait(self.io, &self.mutex) catch return .interrupted;
        }
    }
    pub fn deinit(self: *Self) void {
        std.debug.assert(self.waiters == null);
        self.* = undefined;
    }
};
