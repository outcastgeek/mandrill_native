//! Stable wall-clock deadlines for retained data. Local sleeps still use the
//! injected monotonic clock and never redefine a recovered absolute deadline.
const std = @import("std");
const outcomes = @import("outcome.zig");

pub const Duration = struct {
    nanoseconds: u64,
};

pub const WallInstant = struct {
    /// Signed nanoseconds relative to the Unix epoch, independent of locale
    /// and of the process monotonic clock used for local waiting.
    unix_nanoseconds: i128,

    pub fn plus(self: WallInstant, duration: Duration) outcomes.Result(WallInstant, TimeError) {
        return .{ .ok = .{ .unix_nanoseconds = std.math.add(i128, self.unix_nanoseconds, @as(i128, duration.nanoseconds)) catch return .{ .err = .overflow } } };
    }
};

pub const TimeError = enum { overflow };

pub const Deadline = union(enum) {
    never,
    at: WallInstant,

    pub fn isDue(self: Deadline, now: WallInstant) bool {
        return switch (self) {
            .never => false,
            .at => |instant| now.unix_nanoseconds >= instant.unix_nanoseconds,
        };
    }
};

/// Retained absolute deadlines use an injected wall clock. Local waiting
/// continues to use the caller's monotonic clock/Io; replay never reads an
/// ambient process clock or reconstructs a monotonic timestamp as wall time.
pub const WallClock = struct {
    userdata: ?*anyopaque,
    vtable: *const VTable,

    pub const VTable = struct {
        now: *const fn (?*anyopaque) WallInstant,
    };

    pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) WallClock {
        return .{ .userdata = data, .vtable = vtable };
    }

    pub fn now(self: WallClock) WallInstant {
        return self.vtable.now(self.userdata);
    }
};
