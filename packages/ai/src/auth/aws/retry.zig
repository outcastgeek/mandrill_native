const std = @import("std");
const t = @import("../../types.zig");
const m = @import("../../models.zig");
const credentials = @import("credentials.zig");
const headers = @import("../../utils/headers.zig");
const retry = @import("../../utils/provider_retry.zig");
pub const ErrorType = enum { throttling, transient, other };
pub fn classify(status: ?u16, name: ?[]const u8, message: ?[]const u8) ErrorType {
    if (status == 429) return .throttling;
    if (name) |code| {
        for ([_][]const u8{ "BandwidthLimitExceeded", "EC2ThrottledException", "LimitExceededException", "PriorRequestNotComplete", "ProvisionedThroughputExceededException", "RequestLimitExceeded", "RequestThrottled", "RequestThrottledException", "SlowDown", "ThrottledException", "Throttling", "ThrottlingException", "TooManyRequestsException", "TransactionInProgressException" }) |known| if (std.mem.eql(u8, known, code)) return .throttling;
        for ([_][]const u8{ "TimeoutError", "RequestTimeout", "RequestTimeoutException", "ModelNotReadyException" }) |known| if (std.mem.eql(u8, known, code)) return .transient;
        if (std.mem.eql(u8, code, "InvalidSignatureException") and std.mem.find(u8, message orelse "", "Signature expired") != null) return .transient;
    }
    return if (status == 500 or status == 502 or status == 503 or status == 504) .transient else .other;
}
pub fn transportError(err: anyerror) ErrorType {
    return switch (err) {
        error.RequestTimeout, error.ConnectionTimedOut, error.ConnectionResetByPeer, error.ConnectionRefused, error.BrokenPipe, error.NetworkUnreachable, error.HostUnreachable, error.UnknownHostName, error.TemporaryNameServerFailure, error.ReadFailed, error.WriteFailed, error.HttpConnectionClosing, error.EndOfStream, error.ConnectionClosedByPeer => .transient,
        else => .other,
    };
}
fn configuredMaxAttempts(operation: m.OperationContext, options: t.StreamOptions) !f64 {
    const shared = @import("shared_config.zig");
    const numbers = @import("../../utils/numbers.zig");
    if (try shared.sdkEnv(operation, "AWS_MAX_ATTEMPTS")) |text| if (text.len > 0) {
        if (try numbers.parseIntDefault(operation.allocator, operation.io, text)) |value| return value;
    };
    if (try shared.settingValue(operation, options, "max_attempts")) |raw| if (@import("../../json/value.zig").truthy(raw)) {
        const text = @import("../../utils/diagnostics.zig").formatThrownValue(operation.allocator, .{ .value = raw }) catch |err| {
            if (err == error.CannotConvertObjectToPrimitive) return 3;
            return err;
        };
        defer operation.allocator.free(text);
        if (try numbers.parseIntDefault(operation.allocator, operation.io, text)) |value| return value;
    };
    return 3;
}
pub const Policy = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    maxAttempts: f64 = 3,
    retries: u32 = 0,
    capacity: u32 = 500,
    newRetries: bool = false,
    limiter: ?RateLimiter = null,
    invocationId: [36]u8,
    clockOffsetMs: f64 = 0,
    requestSentAt: ?i64 = null,
    disableClockCorrection: bool = false,
    pub fn init(operation: m.OperationContext, options: t.StreamOptions) !@This() {
        const shared = @import("shared_config.zig");
        const attempts = try configuredMaxAttempts(operation, options);
        const mode = (try shared.sdkEnv(operation, "AWS_RETRY_MODE")) orelse try shared.setting(operation, options, "retry_mode");
        const disable_clock = if (try credentials.usesSigV4(operation, options)) try shared.configBoolean(operation, options, "AWS_DISABLE_CLOCK_SKEW_CORRECTION", "disable_clock_skew_correction") else true;
        const modern = std.mem.eql(u8, (try shared.sdkEnv(operation, "SMITHY_NEW_RETRIES_2026")) orelse "", "true");
        return .{ .allocator = operation.allocator, .io = operation.io, .maxAttempts = attempts, .newRetries = modern, .limiter = if (std.mem.eql(u8, mode orelse "standard", "adaptive")) RateLimiter.init(operation.io) else null, .invocationId = try @import("../../utils/uuid.zig").randomV4(operation.io), .disableClockCorrection = disable_clock };
    }
    pub fn requestHeaders(self: *@This(), a: std.mem.Allocator, original: t.ProviderHeaders) !t.ProviderHeaders {
        if (self.limiter) |*limiter| try limiter.acquire(self.io);
        self.requestSentAt = std.Io.Clock.real.now(self.io).toMilliseconds();
        return headers.merge(a, &.{ original, &.{ .{ .name = "amz-sdk-invocation-id", .value = &self.invocationId }, .{ .name = "amz-sdk-request", .value = try std.fmt.allocPrint(a, "attempt={d}; max={s}", .{ self.retries + 1, try @import("../../utils/numbers.zig").toString(a, self.maxAttempts) }) } } });
    }
    pub fn responseClock(self: *@This(), response_headers: t.ProviderHeaders, server_time: ?[]const u8) !bool {
        if (self.disableClockCorrection) return false;
        var time = server_time;
        for (response_headers) |header| {
            if (std.ascii.eqlIgnoreCase(header.name, "age")) return false;
            if (time == null and std.ascii.eqlIgnoreCase(header.name, "date")) time = header.value;
        }
        const text = time orelse return false;
        const timestamp = retry.dateMilliseconds(self.allocator, self.io, text) catch |err| blk: {
            if (err == error.OutOfMemory or err == error.Canceled) return err;
            break :blk @import("../../utils/datetime.zig").parseRfc3339(self.allocator, text) catch |timestamp_error| {
                if (timestamp_error == error.OutOfMemory or timestamp_error == error.Canceled) return timestamp_error;
                return false;
            };
        };
        const received = std.Io.Clock.real.now(self.io).toMilliseconds();
        const sent = self.requestSentAt orelse received;
        if (received - sent > 900_000) return false;
        const candidate = @as(f64, @floatFromInt(timestamp)) - (@as(f64, @floatFromInt(sent)) + @as(f64, @floatFromInt(received))) / 2;
        const corrected = @abs(candidate) >= 240_000 and candidate != self.clockOffsetMs;
        self.clockOffsetMs = candidate;
        return corrected;
    }
    pub fn waitForRetry(self: *@This(), kind: ErrorType, response_headers: t.ProviderHeaders) !bool {
        if (self.limiter) |*limiter| limiter.update(self.io, kind == .throttling);
        const cost: u32 = if (self.newRetries) (if (kind == .throttling) 5 else 14) else if (kind == .transient) 10 else 5;
        if (kind == .other or @as(f64, @floatFromInt(self.retries)) + 1 >= self.maxAttempts or self.capacity < cost) return false;
        const base: u64 = if (kind == .throttling) (if (self.newRetries) 1000 else 500) else if (self.newRetries) 50 else 100;
        var random: [8]u8 = undefined;
        try self.io.randomSecure(&random);
        const fraction = @as(f64, @floatFromInt(std.mem.readInt(u64, &random, .little) >> 11)) / 9007199254740992.0;
        const ceiling = @min(base << @as(u6, @intCast(@min(self.retries, 8))), 20_000);
        var delay: f64 = @floor(fraction * @as(f64, @floatFromInt(ceiling)));
        if (try retryAfter(self.allocator, self.io, response_headers)) |hint| delay = @max(delay, @min(hint, delay + 5000));
        self.capacity -= cost;
        self.retries += 1;
        try retry.sleep(self.io, @max(0, delay));
        return true;
    }
};
fn retryAfter(a: std.mem.Allocator, io: std.Io, response_headers: t.ProviderHeaders) !?f64 {
    for (response_headers) |header| {
        const value = header.value orelse continue;
        if (std.ascii.eqlIgnoreCase(header.name, "x-amz-retry-after")) {
            const number = std.fmt.parseFloat(f64, value) catch return null;
            return if (std.math.isFinite(number)) number else null;
        }
        if (std.ascii.eqlIgnoreCase(header.name, "retry-after")) {
            if (std.fmt.parseFloat(f64, value)) |seconds| {
                if (std.math.isFinite(seconds)) return seconds * 1000;
                return null;
            } else |_| {}
            const timestamp = retry.dateMilliseconds(a, io, value) catch |err| {
                if (err == error.OutOfMemory or err == error.Canceled) return err;
                return null;
            };
            return @floatFromInt(timestamp - std.Io.Clock.real.now(io).toMilliseconds());
        }
    }
    return null;
}
const RateLimiter = struct {
    enabled: bool = false,
    available: f64 = 0,
    lastMaxRate: f64 = 0,
    measuredRate: f64 = 0,
    requestCount: f64 = 0,
    fillRate: f64 = 0.5,
    lastThrottle: f64,
    lastTimestamp: ?f64 = null,
    lastBucket: f64,
    maxCapacity: f64 = 1,
    timeWindow: f64 = 0,
    fn now(io: std.Io) f64 {
        return @as(f64, @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds())) / 1000;
    }
    fn precise(value: f64) f64 {
        return @round(value * 100_000_000) / 100_000_000;
    }
    fn init(io: std.Io) @This() {
        const timestamp = now(io);
        return .{ .lastThrottle = timestamp, .lastBucket = @floor(timestamp) };
    }
    fn refill(self: *@This(), io: std.Io) void {
        const timestamp = now(io);
        if (self.lastTimestamp) |previous| self.available = @min(self.maxCapacity, self.available + (timestamp - previous) * self.fillRate);
        self.lastTimestamp = timestamp;
    }
    fn acquire(self: *@This(), io: std.Io) !void {
        if (!self.enabled) return;
        self.refill(io);
        while (self.available < 1) {
            try retry.sleep(io, @max(0, @ceil((1 - self.available) / self.fillRate * 1000)));
            self.refill(io);
        }
        self.available -= 1;
    }
    fn update(self: *@This(), io: std.Io, throttled: bool) void {
        const timestamp = now(io);
        const bucket = @floor(timestamp * 2) / 2;
        self.requestCount += 1;
        if (bucket > self.lastBucket) {
            self.measuredRate = precise(self.requestCount / (bucket - self.lastBucket) * 0.8 + self.measuredRate * 0.2);
            self.requestCount = 0;
            self.lastBucket = bucket;
        }
        var calculated: f64 = undefined;
        if (throttled) {
            const rate = if (self.enabled) @min(self.measuredRate, self.fillRate) else self.measuredRate;
            self.lastMaxRate = rate;
            self.timeWindow = precise(std.math.pow(f64, rate * 0.3 / 0.4, 1.0 / 3.0));
            self.lastThrottle = timestamp;
            calculated = precise(rate * 0.7);
            self.enabled = true;
        } else {
            self.timeWindow = precise(std.math.pow(f64, self.lastMaxRate * 0.3 / 0.4, 1.0 / 3.0));
            const elapsed = timestamp - self.lastThrottle - self.timeWindow;
            calculated = precise(0.4 * elapsed * elapsed * elapsed + self.lastMaxRate);
        }
        const next_rate = @min(calculated, 2 * self.measuredRate);
        self.refill(io);
        self.fillRate = @max(next_rate, 0.5);
        self.maxCapacity = @max(next_rate, 1);
        self.available = @min(self.available, self.maxCapacity);
    }
};
