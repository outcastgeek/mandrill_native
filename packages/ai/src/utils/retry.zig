const std = @import("std");
const t = @import("../types.zig");
const patterns = @import("patterns.zig");
pub const limitPatterns: []const []const u8 = &.{ "GoUsageLimitError", "FreeUsageLimitError", "Monthly usage limit reached", "available balance", "insufficient_quota", "out of budget", "quota exceeded", "billing", "subscription_sharing_usage_limit_exceeded" };
pub const retryPatterns: []const []const u8 = &.{ "overloaded", "server_busy", "servers are currently busy", "currently experiencing high demand", "model is at capacity", "rate.?limit", "too many requests", "429", "500", "502", "503", "504", "520", "524", "service.?(?:is.?)?(?:temporarily.?)?unavailable", "server.?error", "internal.?error", "provider.?returned.?(?:an.?)?error", "exceeded request buffer limit while retrying upstream", "unable to forward upstream", "upstream.?request.?failed", "upstream_stream_read_error", "stream was interrupted", "http/?2 stream failed", "network.?error", "connection.?error", "connection.?refused", "connection.?lost", "other side closed", "fetch failed", "request to \\S+ failed, reason:", "getaddrinfo", "ENOTFOUND", "EAI_AGAIN", "upstream.?connect", "reset before headers", "socket hang up", "socket connection was closed", "timed? out", "timeout", "terminated", "websocket.?closed", "websocket.?error", "ended without", "stream ended before message_stop", "stream ended before a terminal response event", "http2 request did not get a response", "pending stream has been canceled", "retry delay", "you can retry your request", "try your request again", "please retry your request", "ResourceExhausted", "subscription_sharing_usage_unavailable", "subscription_sharing_user_unavailable" };
pub const DEFAULT_MAX_AGENT_RETRY_DELAY_MS: f64 = 60_000;
pub const RetryPolicy = struct { enabled: bool, maxRetries: f64, baseDelayMs: f64, maxAgentDelayMs: ?f64 = null };
pub fn retryDelayMs(policy: RetryPolicy, attempt: f64) f64 {
    const delay = policy.baseDelayMs * std.math.pow(f64, 2, @import("numbers.zig").maximum(0, attempt - 1));
    const safe_delay = if (std.math.isFinite(delay) and @trunc(delay) == delay and @abs(delay) <= 9007199254740991) delay else 9007199254740991;
    return @import("numbers.zig").minimum(safe_delay, policy.maxAgentDelayMs orelse DEFAULT_MAX_AGENT_RETRY_DELAY_MS);
}
pub fn isRetryableAssistantError(a: std.mem.Allocator, message: t.AssistantMessage) !bool {
    if (message.stopReason != .@"error" or message.errorMessage == null or message.errorMessage.?.len == 0) return false;
    if (try patterns.terminalLimit(a, null, message.errorMessage.?)) return false;
    return patterns.retryable(a, null, message.errorMessage.?);
}
pub const RetryCallbacks = struct {
    context: ?*anyopaque = null,
    onRetryScheduled: ?*const fn (?*anyopaque, std.Io, f64, f64, f64, []const u8) anyerror!void = null,
    onRetryAttemptStart: ?*const fn (?*anyopaque, std.Io) anyerror!void = null,
    onRetryFinished: ?*const fn (?*anyopaque, std.Io, bool, f64, ?[]const u8) anyerror!void = null,
};
pub const Producer = struct { context: ?*anyopaque, call: *const fn (?*anyopaque, std.mem.Allocator, std.Io) anyerror!t.AssistantMessage };
pub fn retryAssistantCall(a: std.mem.Allocator, io: std.Io, producer: Producer, policy: ?RetryPolicy, callbacks: RetryCallbacks) !t.AssistantMessage {
    const maximum = if (policy != null and policy.?.enabled) policy.?.maxRetries else 0;
    var attempt: f64 = 0;
    while (true) {
        var response = try producer.call(producer.context, a, io);
        if (response.stopReason == .aborted) {
            if (attempt > 0) if (callbacks.onRetryFinished) |call| try call(callbacks.context, io, false, attempt, null);
            return response;
        }
        if (response.stopReason != .@"error") {
            if (attempt > 0) if (callbacks.onRetryFinished) |call| try call(callbacks.context, io, true, attempt, null);
            return response;
        }
        if (attempt >= maximum or !(try isRetryableAssistantError(a, response))) {
            if (attempt > 0) if (callbacks.onRetryFinished) |call| try call(callbacks.context, io, false, attempt, response.errorMessage);
            return response;
        }
        attempt += 1;
        const delay = retryDelayMs(policy.?, attempt);
        const message = response.errorMessage orelse "Unknown error";
        if (callbacks.onRetryScheduled) |call| try call(callbacks.context, io, attempt, maximum, delay, message);
        @import("provider_retry.zig").sleep(io, delay) catch |err| {
            const protection = io.swapCancelProtection(.blocked);
            defer _ = io.swapCancelProtection(protection);
            defer if (err == error.Canceled) io.recancel();
            if (callbacks.onRetryFinished) |call| try call(callbacks.context, io, false, attempt, message);
            if (err == error.Canceled) {
                response.stopReason = .aborted;
                response.errorMessage = null;
                @import("../json/value.zig").markAbsent(&response, "errorMessage");
                return response;
            }
            return err;
        };
        if (callbacks.onRetryAttemptStart) |call| try call(callbacks.context, io);
    }
}
