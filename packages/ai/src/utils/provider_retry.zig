const std = @import("std");
const t = @import("../types.zig");
pub const ProviderRetryOptions = struct {
    maxRetries: f64 = 0,
    maxRetryDelayMs: ?f64 = null,
    noRetryStatuses: []const u16 = &.{},
};
/// Presence of this detail identifies the source helper's provider error
/// shape. A missing status or headers value is still a provider error.
pub const ProviderError = struct { message: []const u8, status: ?u16 = null, headers: ?t.ProviderHeaders = null };
pub fn Request(comptime T: type) type {
    return struct {
        context: ?*anyopaque = null,
        /// Populate failure only for an error carrying provider status/header
        /// fields. Request output and error detail use the supplied allocator.
        call: *const fn (?*anyopaque, std.mem.Allocator, std.Io, *?ProviderError) anyerror!T,
    };
}
/// Each callback invocation is a fresh request. std.Io supplies cancellation
/// for both the request and backoff, replacing the source's AbortSignal.
pub fn retryProviderRequest(comptime T: type, a: std.mem.Allocator, io: std.Io, request: Request(T), options: ProviderRetryOptions, error_message: ?*?[]const u8) !T {
    if (error_message) |slot| slot.* = null;
    var index: f64 = 0;
    while (true) {
        var failure: ?ProviderError = null;
        return request.call(request.context, a, io, &failure) catch |err| {
            io.checkCancel() catch |cancel_err| {
                if (error_message) |slot| slot.* = "Request aborted";
                return cancel_err;
            };
            if (err == error.Canceled) {
                if (error_message) |slot| slot.* = "Request aborted";
                return err;
            }
            if (err == error.OutOfMemory) return err;
            const provider = failure orelse return err;
            const response_headers: t.ProviderHeaders = provider.headers orelse &.{};
            var no_retry = false;
            if (provider.status) |status| for (options.noRetryStatuses) |code| if (status == code) {
                no_retry = true;
                break;
            };
            if (index >= options.maxRetries or !retryable(provider.status, response_headers) or no_retry) {
                if (error_message) |slot| slot.* = try a.dupe(u8, provider.message);
                return err;
            }
            var delay_error: ?[]const u8 = null;
            const milliseconds = delayWithFailure(a, io, response_headers, index, options.maxRetryDelayMs, provider.message, &delay_error) catch |delay_err| {
                if (error_message) |slot| slot.* = delay_error;
                return delay_err;
            };
            index += 1;
            sleep(io, milliseconds) catch |sleep_err| {
                if (sleep_err == error.Canceled) if (error_message) |slot| {
                    slot.* = "Request aborted";
                };
                return sleep_err;
            };
            continue;
        };
    }
}
fn header(headers: t.ProviderHeaders, name: []const u8) ?[]const u8 {
    for (headers) |entry| if (std.ascii.eqlIgnoreCase(entry.name, name)) return entry.value;
    return null;
}
pub fn retryable(status: ?u16, headers: t.ProviderHeaders) bool {
    if (header(headers, "x-should-retry")) |value| {
        if (std.mem.eql(u8, value, "true")) return true;
        if (std.mem.eql(u8, value, "false")) return false;
    }
    const code = status orelse return true;
    return code == 408 or code == 409 or code == 429 or code >= 500;
}
pub fn dateMilliseconds(a: std.mem.Allocator, io: std.Io, bytes: []const u8) !i64 {
    const datetime = @import("datetime.zig");
    return datetime.parseHttpDate(a, io, bytes) catch |err| {
        if (err == error.OutOfMemory or err == error.Canceled) return err;
        return datetime.parseRfc3339(a, bytes);
    };
}
fn serverDelay(a: std.mem.Allocator, io: std.Io, headers: t.ProviderHeaders) !?f64 {
    var requested: ?f64 = null;
    if (header(headers, "retry-after-ms")) |text| {
        const parsed = try @import("numbers.zig").parseFloat(a, io, text);
        if (parsed) |number| {
            if (std.math.isFinite(number)) requested = number;
        }
    }
    if (requested == null) if (header(headers, "retry-after")) |text| {
        if (try @import("numbers.zig").parseFloat(a, io, text)) |seconds| requested = seconds * 1000 else {
            if (dateMilliseconds(a, io, text)) |timestamp| requested = @floatFromInt(timestamp - std.Io.Clock.real.now(io).toMilliseconds()) else |err| {
                if (err == error.OutOfMemory or err == error.Canceled) return err;
            }
        }
    };
    return if (requested) |value| if (std.math.isFinite(value)) value else null else null;
}
const Failure = struct { allocator: std.mem.Allocator, slot: *?[]const u8, message: []const u8 };
pub fn delay(a: std.mem.Allocator, io: std.Io, headers: t.ProviderHeaders, retry_index: f64, cap: ?f64) !u64 {
    return computeDelay(a, io, headers, retry_index, cap, null);
}
pub fn delayWithFailure(a: std.mem.Allocator, io: std.Io, headers: t.ProviderHeaders, retry_index: f64, cap: ?f64, message: []const u8, slot: *?[]const u8) !u64 {
    return computeDelay(a, io, headers, retry_index, cap, .{ .allocator = a, .slot = slot, .message = message });
}
fn computeDelay(a: std.mem.Allocator, io: std.Io, headers: t.ProviderHeaders, retry_index: f64, cap: ?f64, failure: ?Failure) !u64 {
    if (try serverDelay(a, io, headers)) |value| {
        const maximum = cap orelse 60_000;
        if (maximum > 0 and value > maximum) {
            if (failure) |detail| detail.slot.* = try std.fmt.allocPrint(detail.allocator, "Server requested {d:.0}s retry delay (max: {d:.0}s). {s}", .{ @ceil(value / 1000), @ceil(maximum / 1000), detail.message });
            return error.ServerRetryDelayExceedsCap;
        }
        // The pinned timer normalizes finite delays above its signed 32-bit
        // range to one millisecond once the explicit server-delay cap permits it.
        if (value > std.math.maxInt(i32)) return 1;
        return @intFromFloat(@max(0, value));
    }
    var random: [8]u8 = undefined;
    try io.randomSecure(&random);
    const fraction = @as(f64, @floatFromInt(std.mem.readInt(u64, &random, .little) >> 11)) / 9007199254740992.0;
    const exponential = @min(500 * std.math.pow(f64, 2, @min(retry_index, 4)), 8000);
    return @intFromFloat(exponential * (1 - fraction * 0.25));
}
/// Match the timer normalization used by the pinned abortableSleep helper.
/// This is applied after server-delay validation; cap=0 still disables the cap.
pub fn timerMilliseconds(value: f64) u64 {
    return if (!std.math.isFinite(value) or value < 1 or value > std.math.maxInt(i32)) 1 else @intFromFloat(value);
}
pub fn sleep(io: std.Io, milliseconds: anytype) !void {
    const value: f64 = switch (@typeInfo(@TypeOf(milliseconds))) {
        .int, .comptime_int => @floatFromInt(milliseconds),
        else => @floatCast(milliseconds),
    };
    try io.sleep(.fromMilliseconds(@intCast(timerMilliseconds(value))), .awake);
}
