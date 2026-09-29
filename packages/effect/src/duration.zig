//! Explicit nonnegative durations for retained deadlines and local sleeps.
//! Conversion to the native I/O duration is checked rather than truncated.
const std = @import("std");
const outcomes = @import("outcome.zig");
const deadlines = @import("deadline.zig");

/// Duration conversion failure when input is negative or exceeds the representation.
pub const Error = enum { negative, overflow };

/// Validated unsigned nanosecond duration; constructors reject overflow.
pub const Duration = struct {
    nanoseconds: u64,

    /// Adapt the caller-supplied I/O implementation.
    /// Parameters:
    /// - `value: std.Io.Duration`: I/O duration converted to the checked duration representation.
    /// Returns (`outcomes.Result(Duration, Error)`): An ok/err Result; owned payloads must be released according to their type contract.
    pub fn fromIo(value: std.Io.Duration) outcomes.Result(Duration, Error) {
        if (value.nanoseconds < 0) return .{ .err = .negative };
        return .{ .ok = .{ .nanoseconds = std.math.cast(u64, value.nanoseconds) orelse
            return .{ .err = .overflow } } };
    }

    /// Convert this duration to the `std.Io` duration type.
    /// Parameters:
    /// - `self: Duration`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`outcomes.Result(std.Io.Duration, Error)`): An ok/err Result; owned payloads must be released according to their type contract.
    pub fn toIo(self: Duration) outcomes.Result(std.Io.Duration, Error) {
        return .{ .ok = .{ .nanoseconds = std.math.cast(i96, self.nanoseconds) orelse
            return .{ .err = .overflow } } };
    }

    /// Convert this duration to a wall-clock deadline.
    /// Parameters:
    /// - `self: Duration`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`deadlines.Duration`): A validated duration with its declared precision.
    pub fn toDeadline(self: Duration) deadlines.Duration {
        return .{ .nanoseconds = self.nanoseconds };
    }

    /// Add two durations or instants with checked overflow.
    /// Parameters:
    /// - `self: Duration`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// - `other: Duration`: Other duration or typed operand in this operation.
    /// Returns (`outcomes.Result(Duration, Error)`): An ok/err Result; owned payloads must be released according to their type contract.
    pub fn plus(self: Duration, other: Duration) outcomes.Result(Duration, Error) {
        return .{ .ok = .{ .nanoseconds = std.math.add(u64, self.nanoseconds, other.nanoseconds) catch return .{ .err = .overflow } } };
    }

    /// Scale a duration with checked range.
    /// Parameters:
    /// - `self: Duration`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// - `factor: u64`: Multiplier controlling exponential backoff.
    /// Returns (`outcomes.Result(Duration, Error)`): An ok/err Result; owned payloads must be released according to their type contract.
    pub fn multiply(self: Duration, factor: u64) outcomes.Result(Duration, Error) {
        return .{ .ok = .{ .nanoseconds = std.math.mul(u64, self.nanoseconds, factor) catch return .{ .err = .overflow } } };
    }
};
