//! Explicit nonnegative durations for retained deadlines and local sleeps.
//! Conversion to the native I/O duration is checked rather than truncated.
const std = @import("std");
const outcomes = @import("outcome.zig");
const deadlines = @import("deadline.zig");

pub const Error = enum { negative, overflow };

pub const Duration = struct {
    nanoseconds: u64,

    pub fn fromIo(value: std.Io.Duration) outcomes.Result(Duration, Error) {
        if (value.nanoseconds < 0) return .{ .err = .negative };
        return .{ .ok = .{ .nanoseconds = std.math.cast(u64, value.nanoseconds) orelse
            return .{ .err = .overflow } } };
    }

    pub fn toIo(self: Duration) outcomes.Result(std.Io.Duration, Error) {
        return .{ .ok = .{ .nanoseconds = std.math.cast(i96, self.nanoseconds) orelse
            return .{ .err = .overflow } } };
    }

    pub fn toDeadline(self: Duration) deadlines.Duration {
        return .{ .nanoseconds = self.nanoseconds };
    }

    pub fn plus(self: Duration, other: Duration) outcomes.Result(Duration, Error) {
        return .{ .ok = .{ .nanoseconds = std.math.add(u64, self.nanoseconds, other.nanoseconds) catch return .{ .err = .overflow } } };
    }

    pub fn multiply(self: Duration, factor: u64) outcomes.Result(Duration, Error) {
        return .{ .ok = .{ .nanoseconds = std.math.mul(u64, self.nanoseconds, factor) catch return .{ .err = .overflow } } };
    }
};
