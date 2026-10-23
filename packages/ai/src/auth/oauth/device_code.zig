const std = @import("std");
const numbers = @import("../../utils/numbers.zig");
fn remember(slot: ?*?[]const u8, message: []const u8) void {
    if (slot) |out| out.* = message;
}
fn sleep(io: std.Io, milliseconds: f64, slot: ?*?[]const u8) !void {
    @import("../../utils/provider_retry.zig").sleep(io, milliseconds) catch |err| {
        if (err == error.Canceled) remember(slot, "Login cancelled");
        return err;
    };
}
fn now(io: std.Io) f64 {
    return @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds());
}
pub fn PollResult(comptime T: type) type {
    return union(enum) { pending, slow_down: ?f64, failed: []const u8, complete: T };
}
pub fn PollOptions(comptime T: type) type {
    return struct { context: ?*anyopaque, intervalSeconds: ?f64 = null, expiresInSeconds: ?f64 = null, waitBeforeFirstPoll: bool = false, poll: *const fn (?*anyopaque, std.Io) anyerror!PollResult(T), errorMessage: ?*?[]const u8 = null };
}
pub fn pollOAuthDeviceCodeFlow(comptime T: type, io: std.Io, options: PollOptions(T)) !T {
    if (options.errorMessage) |slot| slot.* = null;
    const deadline = if (options.expiresInSeconds) |seconds| now(io) + seconds * 1000 else std.math.inf(f64);
    var interval = numbers.maximum(1000, @floor((options.intervalSeconds orelse 5) * 1000));
    var slowed_down = false;
    if (options.waitBeforeFirstPoll) {
        const remaining = deadline - now(io);
        if (remaining > 0) try sleep(io, numbers.minimum(interval, remaining), options.errorMessage);
    }
    while (now(io) < deadline) {
        io.checkCancel() catch |err| {
            remember(options.errorMessage, "Login cancelled");
            return err;
        };
        const result = options.poll(options.context, io) catch |err| {
            if (err == error.Canceled) remember(options.errorMessage, "Login cancelled");
            return err;
        };
        switch (result) {
            .complete => |value| return value,
            .failed => |message| {
                remember(options.errorMessage, message);
                return error.DeviceAuthorizationFailed;
            },
            .slow_down => |server_interval| {
                slowed_down = true;
                interval = if (server_interval != null and std.math.isFinite(server_interval.?) and server_interval.? > 0) numbers.maximum(1000, @floor(server_interval.? * 1000)) else numbers.maximum(1000, interval + 5000);
            },
            .pending => {},
        }
        const remaining = deadline - now(io);
        if (remaining <= 0) break;
        try sleep(io, numbers.minimum(interval, remaining), options.errorMessage);
    }
    remember(options.errorMessage, if (slowed_down) "Device flow timed out after one or more slow_down responses. This is often caused by clock drift in WSL or VM environments. Please sync or restart the VM clock and try again." else "Device flow timed out");
    return if (slowed_down) error.DeviceFlowTimedOutAfterSlowDown else error.DeviceFlowTimedOut;
}
