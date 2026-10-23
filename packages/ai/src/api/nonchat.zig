const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const models = @import("../models.zig");
const headers = @import("../utils/headers.zig");
const retry = @import("../utils/provider_retry.zig");
const http = @import("../transport/http.zig");
const Attempt = struct { response: http.Response, json: ?std.json.Value = null, errorBody: ?[]const u8 = null, abortRelease: bool = false };
const AttemptState = struct { headReceived: bool = false, sdkImages: bool = false, sdkStream: bool = false };
fn fetchText(a: std.mem.Allocator, bytes: []const u8) ![]const u8 {
    const decoded = try std.fmt.allocPrint(a, "{f}", .{std.unicode.fmtUtf8(bytes)});
    var reader = std.Io.Reader.fixed(decoded);
    if (std.mem.startsWith(u8, decoded, &.{ 0xef, 0xbb, 0xbf })) reader.toss(3);
    return reader.buffered();
}
fn validateClassifierTimeout(a: std.mem.Allocator, options: t.StreamOptions, message: *?[]const u8) !void {
    if (!j.fieldPresent(options, "timeoutMs")) return;
    const raw = (try j.fieldValue(a, options, "timeoutMs")) orelse return;
    const milliseconds = switch (raw) {
        .integer => |value| @as(f64, @floatFromInt(value)),
        .float => |value| value,
        else => {
            message.* = try std.fmt.allocPrint(a, "The \"delay\" argument must be of type number. Received {s}", .{try @import("../utils/diagnostics.zig").formatThrownValue(a, .{ .value = raw })});
            return error.InvalidClassifierTimeout;
        },
    };
    const integer = std.math.isFinite(milliseconds) and @floor(milliseconds) == milliseconds;
    if (integer and milliseconds >= 0 and milliseconds <= std.math.maxInt(u32)) return;
    const text = try @import("../utils/numbers.zig").toString(a, milliseconds);
    var display = std.Io.Writer.Allocating.init(a);
    defer display.deinit();
    if (integer and @abs(milliseconds) > 4294967296) {
        const negative = std.mem.startsWith(u8, text, "-");
        if (negative) try display.writer.writeByte('-');
        const digits = text[@intFromBool(negative)..];
        const first = (digits.len - 1) % 3 + 1;
        try display.writer.writeAll(digits[0..first]);
        var groups = std.mem.window(u8, digits[first..], 3, 3);
        while (groups.next()) |group| {
            try display.writer.writeByte('_');
            try display.writer.writeAll(group);
        }
    } else try display.writer.writeAll(text);
    message.* = try std.fmt.allocPrint(a, "The value of \"delay\" is out of range. It must be {s}. Received {s}", .{ if (integer) ">= 0 && <= 4294967295" else "an integer", display.written() });
    return error.InvalidClassifierTimeout;
}
fn readAttempt(transport: http.Transport, a: std.mem.Allocator, io: std.Io, request: http.Request, state: *AttemptState) !Attempt {
    const started = std.Io.Clock.real.now(io).toMilliseconds();
    var response = try transport.send(a, io, request);
    errdefer response.abort();
    state.headReceived = true;
    const success = response.status >= 200 and response.status < 300;
    // SDK streaming replacements return an unconsumed Stream object. The image
    // adapter then observes headers and fails on its missing choices property.
    if (state.sdkImages and success and state.sdkStream) return .{ .response = response, .json = j.map(a), .abortRelease = true };
    if (state.sdkImages and response.status == 204) return .{ .response = response, .json = @as(std.json.Value, .null) };
    var media_json = false;
    var no_body_deadline = false;
    if (state.sdkImages and success) {
        const record = try headers.headersToRecord(a, response.headers);
        no_body_deadline = std.mem.eql(u8, j.field(record, "content-length") orelse "", "0");
        const content_type = j.field(record, "content-type") orelse "";
        const media = try @import("../utils/unicode.zig").trimWhitespace(a, std.mem.sliceTo(content_type, ';'));
        media_json = std.ascii.findIgnoreCase(media, "application/json") != null or (media.len >= 5 and std.ascii.eqlIgnoreCase(media[media.len - 5 ..], "+json"));
        if (media_json and no_body_deadline) return .{ .response = response, .abortRelease = true };
    }
    const remaining: ?f64 = if (state.sdkImages and success and !no_body_deadline and request.timeoutMs != null) @import("../utils/numbers.zig").maximum(0, request.timeoutMs.? - @as(f64, @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds() - started))) else null;
    const bytes = if (remaining) |timeout| try response.readAllWithTimeout(a, io, .unlimited, timeout) else response.readAll(a, .unlimited) catch |err| {
        if (!state.sdkImages or success or err == error.Canceled or err == error.OutOfMemory) return err;
        // The SDK turns a failed HTTP error-body read into that failure text.
        return .{ .response = response, .errorBody = try a.dupe(u8, @errorName(err)), .abortRelease = true };
    };
    if (!success) return .{ .response = response, .errorBody = bytes };
    defer a.free(bytes);
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const text = try fetchText(temporary.allocator(), bytes);
    if (state.sdkImages) {
        if (!media_json) return .{ .response = response, .json = j.str(try a.dupe(u8, text)) };
        if (text.len == 0) return .{ .response = response };
    }
    return .{ .response = response, .json = try @import("../json/value.zig").parseFromSliceLeaky(std.json.Value, a, text, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) };
}
fn timedAttempt(transport: http.Transport, a: std.mem.Allocator, io: std.Io, request: http.Request, whole_body: bool, state: *AttemptState) !Attempt {
    if (!whole_body or request.timeoutMs == null) return readAttempt(transport, a, io, request, state);
    const Outcome = union(enum) { response: anyerror!Attempt, timeout: std.Io.Cancelable!void };
    var buffer: [2]Outcome = undefined;
    var select = std.Io.Select(Outcome).init(io, &buffer);
    defer while (select.cancel()) |outcome| switch (outcome) {
        .response => |result| {
            if (result) |value| {
                var response = value.response;
                response.deinit();
            } else |_| {}
        },
        .timeout => {},
    };
    var untimed = request;
    untimed.timeoutMs = null;
    try select.concurrent(.response, readAttempt, .{ transport, a, io, untimed, state });
    try select.concurrent(.timeout, struct {
        fn sleep(clock_io: std.Io, milliseconds: f64) std.Io.Cancelable!void {
            try clock_io.sleep(.fromMilliseconds(@intCast(@import("../utils/provider_retry.zig").timerMilliseconds(milliseconds))), .awake);
        }
    }.sleep, .{ io, request.timeoutMs.? });
    return switch (try select.await()) {
        .response => |result| result,
        .timeout => |result| {
            try result;
            return error.RequestTimeout;
        },
    };
}
pub const Client = struct {
    operation: models.OperationContext,
    model: t.Model,
    options: t.StreamOptions,
    errorLabel: []const u8,
    captureHttpError: bool = false,
    failureHeaders: ?t.ProviderHeaders = null,
    noRetryStatuses: []const u16 = &.{},
    status: ?u16 = null,
    errorBody: ?[]const u8 = null,
    failureMessage: ?[]const u8 = null,
    sdkImages: bool = false,
    responseUndefined: bool = false,
    sdkCustomHeaders: t.ProviderHeaders = &.{},
    sdkDefaultHeaders: t.ProviderHeaders = &.{},
    const Target = union(enum) { uri: std.Uri, openAIBase: []const u8 };
    pub fn post(self: *@This(), a: std.mem.Allocator, url: std.Uri, original: std.json.Value, observe: bool, require_key: bool, default_retries: u32, no_retry: ?u16) !std.json.Value {
        return self.postTarget(a, .{ .uri = url }, original, observe, require_key, default_retries, no_retry);
    }
    pub fn postOpenAIImages(self: *@This(), a: std.mem.Allocator, base: []const u8, original: std.json.Value) !std.json.Value {
        return self.postTarget(a, .{ .openAIBase = base }, original, true, true, 0, null);
    }
    fn imageEndpoint(a: std.mem.Allocator, base: []const u8) !std.Uri {
        return @import("openai_request_url.zig").endpoint(a, base, "/chat/completions");
    }
    fn postTarget(self: *@This(), a: std.mem.Allocator, target: Target, original: std.json.Value, observe: bool, require_key: bool, default_retries: u32, no_retry: ?u16) !std.json.Value {
        if (require_key and (self.options.apiKey == null or self.options.apiKey.?.len == 0)) return error.MissingAuthentication;
        var payload = original;
        if (observe) if (self.options.callbacks.onPayload) |call| if (try call(self.options.callbacks.context, a, &payload, self.model)) |replacement| {
            payload = replacement;
        };
        const sdk_images = self.sdkImages;
        if (sdk_images and payload == .null) {
            self.failureMessage = "Cannot read properties of null (reading 'stream')";
            return error.InvalidOpenAIRequestPayload;
        }
        const url = switch (target) {
            .uri => |value| value,
            .openAIBase => |base| try imageEndpoint(a, base),
        };
        if (!sdk_images) try validateClassifierTimeout(a, self.options, &self.failureMessage);
        const body_present = !sdk_images or j.truthy(payload);
        const encoded: ?[]const u8 = if (body_present) try @import("../json/value.zig").stringify(a, payload, .{}) else null;
        const timeout = self.options.timeoutMs orelse if (sdk_images) @as(?f64, 600_000) else null;
        if (sdk_images) try @import("sdk_request_headers.zig").validateTimeout(self.options, &self.failureMessage);
        var defaults: std.ArrayList(t.ProviderHeader) = .empty;
        if (sdk_images) {
            if (body_present) try defaults.append(a, .{ .name = "content-type", .value = "application/json" });
            try defaults.appendSlice(a, self.sdkDefaultHeaders);
        }
        if (self.options.apiKey) |key| if (key.len > 0) try defaults.append(a, .{ .name = "authorization", .value = try std.fmt.allocPrint(a, "Bearer {s}", .{key}) });
        if (!sdk_images and body_present) try defaults.append(a, .{ .name = "content-type", .value = "application/json" });
        const request_headers = try headers.merge(a, &.{ defaults.items, self.sdkCustomHeaders, if (sdk_images) &.{} else try headers.fromValue(a, self.model.headers), if (sdk_images) &.{} else self.options.headers, if (sdk_images and body_present) &.{.{ .name = "content-type", .value = "application/json" }} else &.{} });
        const proxy = try @import("../transport/proxy.zig").resolveHttpProxyUrlForTargetWithError(a, self.operation.io, self.operation.authContext, self.options.env, url, &self.failureMessage);
        var index: f64 = 0;
        const maximum = self.options.maxRetries orelse @as(f64, @floatFromInt(default_retries));
        while (true) {
            self.status = null;
            self.errorBody = null;
            self.failureHeaders = null;
            var state = AttemptState{ .sdkImages = sdk_images, .sdkStream = sdk_images and j.truthy(j.get(payload, "stream") orelse .null) };
            var attempt = timedAttempt(self.options.httpTransport orelse self.operation.transport, a, self.operation.io, .{ .method = .POST, .uri = url, .headers = request_headers, .body = encoded, .timeoutMs = timeout, .proxy = proxy, .proxyResolved = true }, !sdk_images, &state) catch |err| {
                if (err == error.Canceled or err == error.OutOfMemory or index >= maximum or (err != error.RequestTimeout and !sdk_images)) {
                    if (sdk_images and err != error.OutOfMemory) {
                        if (err == error.Canceled) self.failureMessage = "Request aborted" else if (err == error.RequestTimeout) self.failureMessage = "Request timed out." else if (!state.headReceived) self.failureMessage = "Connection error.";
                    }
                    return err;
                }
                // Syntax/JSON failures do not have SDK provider-error fields.
                if (sdk_images and state.headReceived and err != error.RequestTimeout) return err;
                const delay = try retry.delay(a, self.operation.io, &.{}, index, self.options.maxRetryDelayMs);
                index += 1;
                try self.sleepRetry(delay);
                continue;
            };
            var released = false;
            defer if (!released) {
                if (attempt.abortRelease) attempt.response.abort() else attempt.response.deinit();
            };
            self.status = attempt.response.status;
            if (attempt.errorBody) |body| {
                self.errorBody = body;
                const excluded = for (self.noRetryStatuses) |status| {
                    if (status == attempt.response.status) break true;
                } else false;
                if (!excluded and attempt.response.status != no_retry and index < maximum and retry.retryable(attempt.response.status, attempt.response.headers)) {
                    const message = if (sdk_images) try @import("sdk_http_errors.zig").openAIRequestMessage(a, attempt.response.status, body) else try std.fmt.allocPrint(a, "{s} returned {d}", .{ self.errorLabel, attempt.response.status });
                    const delay = try retry.delayWithFailure(a, self.operation.io, attempt.response.headers, index, self.options.maxRetryDelayMs, message, &self.failureMessage);
                    if (attempt.abortRelease) attempt.response.abort() else attempt.response.deinit();
                    released = true;
                    index += 1;
                    try self.sleepRetry(delay);
                    continue;
                }
                if (self.captureHttpError) self.failureHeaders = try headers.clone(a, attempt.response.headers);
                return error.ProviderHttpError;
            }
            if (observe) if (self.options.callbacks.onResponse) |call| try call(self.options.callbacks.context, .{ .status = attempt.response.status, .headers = try headers.fromValue(a, try headers.headersToRecord(a, attempt.response.headers)) }, self.model);
            self.responseUndefined = attempt.json == null;
            return attempt.json orelse .null;
        }
    }
    fn sleepRetry(self: *@This(), delay: u64) !void {
        retry.sleep(self.operation.io, delay) catch |err| {
            if (self.sdkImages and err == error.Canceled) self.failureMessage = "Request aborted";
            return err;
        };
    }
    pub fn errorMessage(self: @This(), a: std.mem.Allocator, err: anyerror, label: []const u8) ![]const u8 {
        if (self.failureMessage) |message| return message;
        if (err == error.Canceled) return "Request aborted";
        if (err == error.CannotConvertObjectToPrimitive) return "Cannot convert object to primitive value";
        if (err == error.MissingAuthentication) return std.fmt.allocPrint(a, "No API key for provider: {s}", .{self.model.provider});
        if (self.errorBody) |body| {
            if (self.sdkImages) {
                var model = self.model;
                model.api = "openrouter-images";
                return @import("sdk_http_errors.zig").openAI(a, model, self.status orelse 0, body);
            }
            var temporary = std.heap.ArenaAllocator.init(a);
            defer temporary.deinit();
            const text = try fetchText(temporary.allocator(), body);
            const errors = @import("../utils/error_body.zig");
            return errors.formatProviderError(a, try errors.normalizeProviderError(a, .{ .sdk = .{ .message = try std.fmt.allocPrint(a, "{s} returned {d}", .{ label, self.status orelse 0 }), .status = if (self.status) |status| @floatFromInt(status) else null, .body = text } }), try std.fmt.allocPrint(a, "{s} error", .{label}));
        }
        if (err == error.RequestTimeout) return if (self.sdkImages) a.dupe(u8, "Request timed out.") else std.fmt.allocPrint(a, "Request timed out after {s}ms", .{try @import("../utils/numbers.zig").toString(a, self.options.timeoutMs orelse 0)});
        return @errorName(err);
    }
};
pub fn number(value: std.json.Value) !f64 {
    const n = j.number(value) orelse return error.InvalidClassifierResponse;
    if (!std.math.isFinite(n)) return error.InvalidClassifierResponse;
    return n;
}
pub fn requiredNumber(value: std.json.Value, key: []const u8) !f64 {
    return number(j.get(value, key) orelse return error.InvalidClassifierResponse);
}
pub fn parseUsage(a: std.mem.Allocator, model: t.Model, value: ?std.json.Value) !?t.Usage {
    const raw = value orelse return null;
    if (raw != .object or (j.get(raw, "input_tokens") == null and j.get(raw, "output_tokens") == null)) return null;
    var result = t.Usage{};
    if (j.get(raw, "input_tokens")) |v| {
        const n = number(v) catch 0;
        result.input = if (n > 0) n else 0;
    }
    if (j.get(raw, "output_tokens")) |v| {
        const n = number(v) catch 0;
        result.output = if (n > 0) n else 0;
    }
    result.totalTokens = result.input + result.output;
    _ = try @import("../utils/cost.zig").calculateCost(a, model, &result);
    return result;
}
