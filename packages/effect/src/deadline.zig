//! Stable wall-clock deadlines for retained data. Local sleeps still use the
//! injected monotonic clock and never redefine a recovered absolute deadline.
const std = @import("std");
const outcomes = @import("outcome.zig");

/// Nonnegative nanosecond duration used to calculate deadlines.
pub const Duration = struct {
    nanoseconds: u64,
};

/// Signed Unix nanosecond timestamp supplied by a clock.
pub const WallInstant = struct {
    /// Signed nanoseconds relative to the Unix epoch, independent of locale
    /// and of the process monotonic clock used for local waiting.
    unix_nanoseconds: i128,

    /// Add two durations or instants with checked overflow.
    /// Parameters:
    /// - `self: WallInstant`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// - `duration: Duration`: Caller-supplied time or cursor boundary used for ordering and retention.
    /// Returns (`outcomes.Result(WallInstant, TimeError)`): An ok/err Result; owned payloads must be released according to their type contract.
    pub fn plus(self: WallInstant, duration: Duration) outcomes.Result(WallInstant, TimeError) {
        return .{ .ok = .{ .unix_nanoseconds = std.math.add(i128, self.unix_nanoseconds, @as(i128, duration.nanoseconds)) catch return .{ .err = .overflow } } };
    }
};

/// `TimeError` cases: `overflow`.
pub const TimeError = enum { overflow };

/// A deadline that never expires or becomes due at a specific wall instant.
pub const Deadline = union(enum) {
    never,
    at: WallInstant,

    /// Check whether this deadline has arrived.
    /// Parameters:
    /// - `self: Deadline`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// - `now: WallInstant`: Caller-supplied time or cursor boundary used for ordering and retention.
    /// Returns (`bool`): `false` for `.never`; otherwise whether `now` has reached the stored instant.
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

    /// Typed function-pointer interface for this capability. Implement every
    /// hook with the exact signature below; keep the table valid while the
    /// capability or pending work can invoke it.
    pub const VTable = struct {
        /// Read the current time.
        /// Parameters (callback order):
        /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
        /// Returns (`WallInstant`): the specified typed result.
        now: *const fn (?*anyopaque) WallInstant,
    };

    /// Wrap a caller-supplied provider implementation.
    /// Parameters:
    /// - `data: ?*anyopaque`: Borrowed provider context passed unchanged to every hook.
    ///   Its owner keeps it alive through any pending effect or operation.
    /// - `vtable: *const VTable`: Borrowed table of exactly typed callbacks; keep it
    ///   alive for every use of this capability, including deferred execution.
    /// Returns (`WallClock`): A non-owning adapter referencing `data` and `vtable`; no
    ///   provider state is allocated or transferred by this constructor.
    pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) WallClock {
        return .{ .userdata = data, .vtable = vtable };
    }

    /// Read the current time from this clock capability.
    /// Parameters:
    /// - `self: WallClock`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`WallInstant`): The wall-clock instant returned by the clock.
    pub fn now(self: WallClock) WallInstant {
        return self.vtable.now(self.userdata);
    }
};
