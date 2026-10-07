//! Unbuffered std.Io.Writer bridge for schema sinks. The target retains output
//! ownership and enforces its byte budget; this bridge preserves its exact error.
const std = @import("std");

/// Adapt a checked schema sink without buffering, allocating, or erasing its error.
pub fn Sink(comptime Target: type, comptime Failure: type, comptime append: fn (*Target, []const u8) Failure!void) type {
    return struct {
        const Self = @This();
        target: *Target,
        failure: ?Failure = null,
        writer: std.Io.Writer = .{ .buffer = &.{}, .vtable = &.{ .drain = drain } },

        /// Recover the target's error from std's single WriteFailed channel.
        pub fn check(self: *const Self, result: std.Io.Writer.Error!void) Failure!void {
            result catch return self.failure.?;
        }

        fn drain(writer: *std.Io.Writer, data: []const []const u8, splat: usize) std.Io.Writer.Error!usize {
            const self: *Self = @fieldParentPtr("writer", writer);
            var count: usize = 0;
            for (data[0 .. data.len - 1]) |bytes| {
                append(self.target, bytes) catch |problem| {
                    self.failure = problem;
                    return error.WriteFailed;
                };
                count += bytes.len;
            }
            for (0..splat) |_| {
                const bytes = data[data.len - 1];
                append(self.target, bytes) catch |problem| {
                    self.failure = problem;
                    return error.WriteFailed;
                };
                count += bytes.len;
            }
            return count;
        }
    };
}
