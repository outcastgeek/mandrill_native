//! Native HTTP behavior for the pinned Google auth library's OAuth retry policy.
//! Token exchange and JSON decoding remain in Zig and use the caller's I/O.
const std = @import("std");
const t = @import("../../types.zig");
const transport = @import("../../transport/http.zig");
const uri = @import("../../transport/uri.zig");
const oauth = @import("../oauth/http.zig");
pub const timeoutMessage = "The operation was aborted.";
pub const Response = struct {
    status: u16,
    body: std.json.Value,
    raw: []const u8,
    headers: t.ProviderHeaders,
    /// Constructed before the default error redactor mutates response data.
    failureMessage: ?[]const u8 = null,
    pub fn ok(self: @This()) bool {
        return self.status >= 200 and self.status < 300;
    }
};
pub const FailureResponse = struct {
    allocator: std.mem.Allocator,
    arena: *std.heap.ArenaAllocator,
    body: std.json.Value,
    status: u16,
    gaxios: bool = true,
    name: []const u8,
    pub fn init(a: std.mem.Allocator, value: std.json.Value, status: u16) !@This() {
        return initNamed(a, value, status, true, "Error");
    }
    fn initNamed(a: std.mem.Allocator, value: std.json.Value, status: u16, gaxios: bool, name: []const u8) !@This() {
        const arena = try a.create(std.heap.ArenaAllocator);
        errdefer a.destroy(arena);
        arena.* = .init(a);
        errdefer arena.deinit();
        return .{ .allocator = a, .arena = arena, .body = try @import("../../json/value.zig").clone(arena.allocator(), value), .status = status, .gaxios = gaxios, .name = try arena.allocator().dupe(u8, name) };
    }
    pub fn deinit(self: *@This()) void {
        self.arena.deinit();
        self.allocator.destroy(self.arena);
        self.* = undefined;
    }
    pub fn setName(self: *@This(), name: []const u8) !void {
        self.name = try self.arena.allocator().dupe(u8, name);
    }
    pub fn clone(self: @This(), a: std.mem.Allocator) !@This() {
        return initNamed(a, self.body, self.status, self.gaxios, self.name);
    }
};
pub const requiredString = oauth.requiredString;
pub const positiveNumber = oauth.positiveNumber;
pub fn errorMessage(a: std.mem.Allocator, response: Response) ![]const u8 {
    if (response.failureMessage) |message| return a.dupe(u8, message);
    const j = @import("../../json/value.zig");
    const diagnostics = @import("../../utils/diagnostics.zig");
    var message = if (response.body == .string) response.body.string else try std.fmt.allocPrint(a, "Request failed with status code {d}", .{response.status});
    const detail = j.get(response.body, "error") orelse return a.dupe(u8, message);
    if (!j.truthy(detail)) return a.dupe(u8, message);
    if (detail == .string) return a.dupe(u8, detail.string);
    if (detail != .object and detail != .array) return a.dupe(u8, message);
    if (j.field(detail, "message")) |text| message = text;
    if (j.get(detail, "errors")) |errors| if (errors == .array) {
        var parts: std.ArrayList([]const u8) = .empty;
        defer parts.deinit(a);
        for (errors.array.items) |entry| {
            if (entry == .null) return error.GoogleGaxiosNullErrorEntry;
            if (j.field(entry, "message")) |text| try parts.append(a, text);
        }
        const joined = try std.mem.join(a, "\n", parts.items);
        if (joined.len > 0) message = joined;
    };
    if (j.get(detail, "message")) |raw| return diagnostics.formatThrownValue(a, .{ .value = raw });
    return a.dupe(u8, message);
}
fn retryable(status: u16) bool {
    return (status >= 100 and status <= 199) or status == 408 or status == 429 or (status >= 500 and status <= 599);
}
fn retryMethod(method: std.http.Method) bool {
    return switch (method) {
        .GET, .PUT, .POST, .HEAD, .OPTIONS, .DELETE => true,
        else => false,
    };
}
fn wait(io: std.Io, attempt: u32) !void {
    const milliseconds: u64 = if (attempt == 0) 100 else ((@as(u64, 1) << @as(u6, @intCast(attempt))) - 1) * 500;
    try io.sleep(.fromMilliseconds(@intCast(milliseconds)), .awake);
}
pub const Policy = struct {
    timeoutMs: ?u64 = null,
    maxRetries: u32 = 3,
    noResponseRetries: u32 = 2,
    clientCertificate: ?transport.ClientCertificate = null,
    responseType: enum { json, text, auto } = .json,
    authClientHeaders: bool = false,
    acceptJson: ?bool = null,
    errorMessage: ?*?[]const u8 = null,
    errorStorage: ?*t.ErrorMessageStorage = null,
    errorAllocator: ?std.mem.Allocator = null,
    rawUrl: ?std.json.Value = null,
    failure: ?*?FailureResponse = null,
    failureName: ?*?[]const u8 = null,
};
fn fetchText(a: std.mem.Allocator, bytes: []const u8) ![]const u8 {
    const decoded = try std.fmt.allocPrint(a, "{f}", .{std.unicode.fmtUtf8(bytes)});
    defer a.free(decoded);
    var reader = std.Io.Reader.fixed(decoded);
    if (std.mem.startsWith(u8, decoded, &.{ 0xef, 0xbb, 0xbf })) reader.toss(3);
    return a.dupe(u8, reader.buffered());
}
const redacted = "<<REDACTED> - See `errorRedactor` option in `gaxios` for configuration>.";
fn freeResponseHeaders(a: std.mem.Allocator, received: []t.ProviderHeader) void {
    for (received) |header| {
        a.free(header.name);
        if (header.value) |value| a.free(value);
    }
    a.free(received);
}
fn responseHeaders(a: std.mem.Allocator, io: std.Io, source: t.ProviderHeaders) ![]t.ProviderHeader {
    const regex = @import("../../regex/regexp.zig");
    var received: std.ArrayList(t.ProviderHeader) = .empty;
    errdefer {
        for (received.items) |header| {
            a.free(header.name);
            if (header.value) |value| a.free(value);
        }
        received.deinit(a);
    }
    for (source) |header| if (header.value) |value| {
        if (!try regex.matches(a, io, "^[!#$%&'*+.^_`|~0-9A-Za-z-]+$", header.name, .{})) continue;
        if (try regex.matches(a, io,
            \\[^\t\x20-\x7E\x80-\xFF]
        , value, .{})) continue;
        for (received.items) |*previous| {
            if (std.ascii.eqlIgnoreCase(previous.name, header.name)) {
                const joined = try std.fmt.allocPrint(a, "{s}, {s}", .{ previous.value.?, value });
                a.free(previous.value.?);
                previous.value = joined;
                break;
            }
        } else {
            const name = try std.ascii.allocLowerString(a, header.name);
            errdefer a.free(name);
            const copied = try a.dupe(u8, value);
            errdefer a.free(copied);
            try received.append(a, .{ .name = name, .value = copied });
        }
    };
    for (received.items) |*header| if (std.mem.eql(u8, header.name, "content-encoding")) {
        const lowered = try std.ascii.allocLowerString(a, header.value.?);
        a.free(header.value.?);
        header.value = lowered;
    };
    std.mem.sort(t.ProviderHeader, received.items, {}, struct {
        fn less(_: void, left: t.ProviderHeader, right: t.ProviderHeader) bool {
            return std.mem.order(u8, left.name, right.name) == .lt;
        }
    }.less);
    return received.toOwnedSlice(a);
}
fn redactResponse(a: std.mem.Allocator, io: std.Io, value: *std.json.Value, received: []t.ProviderHeader) !void {
    const regex = @import("../../regex/regexp.zig");
    const j = @import("../../json/value.zig");
    for (received) |*header| if (try regex.matches(a, io, "^authentication$|^authorization$|secret", header.name, .{ .ignoreCase = true })) {
        const replacement = try a.dupe(u8, redacted);
        if (header.value) |previous| a.free(previous);
        header.value = replacement;
    };
    if (value.* == .string) {
        if (try regex.matches(a, io, "grant_type=|assertion=|secret", value.string, .{ .ignoreCase = true })) value.* = j.str(redacted);
    } else if (value.* == .object) {
        // JSON cannot supply callable forEach/set methods. The pinned
        // redactor's URLSearchParams-like branch still selects these names
        // by property presence and then throws at the call boundary.
        if (j.get(value.*, "forEach") != null and j.get(value.*, "set") != null) return error.GoogleGaxiosRedactionFailed;
        for ([_][]const u8{ "grant_type", "assertion", "client_secret" }) |name| {
            if (j.get(value.*, name) != null) try j.put(a, value, name, j.str(redacted));
        }
    }
}
fn hasUrlCredentials(endpoint: std.Uri) bool {
    for ([_]?std.Uri.Component{ endpoint.user, endpoint.password }) |component| if (component) |value| {
        if (switch (value) {
            .raw, .percent_encoded => |text| text.len > 0,
        }) return true;
    };
    return false;
}
fn readResponseAttempt(a: std.mem.Allocator, io: std.Io, network: transport.Transport, request_options: transport.Request) !transport.BufferedResponse {
    try io.checkCancel();
    if (hasUrlCredentials(request_options.uri)) return error.RequestUrlIncludesCredentials;
    if (request_options.body != null and (request_options.method == .GET or request_options.method == .HEAD)) return error.HttpMethodBodyUnsupported;
    var response = try network.send(a, io, request_options);
    errdefer response.deinit();
    return .{ .response = response, .bytes = if (response.status == 204 or !response.bodyPresent) try a.alloc(u8, 0) else try response.readAll(a, .unlimited) };
}
fn responseTimeout(io: std.Io, milliseconds: f64) std.Io.Cancelable!void {
    try io.sleep(.fromMilliseconds(@intCast(@import("../../utils/provider_retry.zig").timerMilliseconds(milliseconds))), .awake);
}
fn readResponse(a: std.mem.Allocator, io: std.Io, network: transport.Transport, request_options: transport.Request) !transport.BufferedResponse {
    const timeout = request_options.timeoutMs orelse return readResponseAttempt(a, io, network, request_options);
    const Outcome = union(enum) { response: anyerror!transport.BufferedResponse, timeout: std.Io.Cancelable!void };
    var buffer: [2]Outcome = undefined;
    var select = std.Io.Select(Outcome).init(io, &buffer);
    defer while (select.cancel()) |outcome| switch (outcome) {
        .response => |result| {
            if (result) |response| {
                var owned = response;
                owned.deinit(a);
            } else |_| {}
        },
        .timeout => {},
    };
    var untimed = request_options;
    untimed.timeoutMs = null;
    try select.concurrent(.response, readResponseAttempt, .{ a, io, network, untimed });
    try select.concurrent(.timeout, responseTimeout, .{ io, timeout });
    return switch (try select.await()) {
        .response => |result| result,
        .timeout => |result| {
            try result;
            return error.RequestTimeout;
        },
    };
}
fn mergeRequestHeaders(a: std.mem.Allocator, sources: []const t.ProviderHeaders) !t.ProviderHeaders {
    var merged: std.ArrayList(t.ProviderHeader) = .empty;
    errdefer merged.deinit(a);
    for (sources) |source| for (source) |header| {
        if (!std.ascii.eqlIgnoreCase(header.name, "set-cookie") or header.value == null) {
            var index: usize = 0;
            while (index < merged.items.len) {
                if (std.ascii.eqlIgnoreCase(merged.items[index].name, header.name)) _ = merged.orderedRemove(index) else index += 1;
            }
        }
        if (header.value != null) try merged.append(a, header);
    };
    return merged.toOwnedSlice(a);
}
pub fn requestWithPolicy(a: std.mem.Allocator, io: std.Io, network: transport.Transport, method: std.http.Method, url: []const u8, body: ?[]const u8, content_type: ?[]const u8, extra: t.ProviderHeaders, policy: Policy) !Response {
    if (policy.failure) |slot| {
        if (slot.*) |*previous| previous.deinit();
        slot.* = null;
    }
    if (policy.failureName) |slot| slot.* = null;
    const h = @import("../../utils/headers.zig");
    var defaults: std.ArrayList(t.ProviderHeader) = .empty;
    if (policy.acceptJson orelse (policy.responseType == .json)) try defaults.append(a, .{ .name = "accept", .value = "application/json" });
    if (content_type) |value| try defaults.append(a, .{ .name = "content-type", .value = value });
    var request_headers = try mergeRequestHeaders(a, &.{ defaults.items, extra });
    var validated: std.ArrayList(t.ProviderHeader) = .empty;
    for (request_headers) |header| if (header.value) |value| {
        var diagnostic: ?[]const u8 = null;
        h.appendFetch(a, &validated, header.name, value, &diagnostic) catch |err| {
            defer if (diagnostic) |message| a.free(message);
            if (policy.errorMessage) |slot| if (diagnostic) |message| {
                slot.* = if (policy.errorStorage) |storage| try storage.copy(message) else try (policy.errorAllocator orelse a).dupe(u8, message);
            };
            return err;
        };
    };
    request_headers = try validated.toOwnedSlice(a);
    const missing_url = if (policy.rawUrl) |value| !@import("../../json/value.zig").truthy(value) else url.len == 0;
    if (missing_url) {
        if (policy.errorMessage) |slot| slot.* = "URL is required.";
        return error.GoogleUrlRequired;
    }
    const url_text = if (policy.rawUrl) |value| try @import("../../utils/diagnostics.zig").formatThrownValue(a, .{ .value = value }) else url;
    var endpoint = uri.parseHttpUrl(a, url_text) catch |err| {
        if (err != error.OutOfMemory and err != error.Canceled) {
            if (policy.failureName) |slot| slot.* = "TypeError";
            if (policy.errorMessage) |slot| slot.* = "Invalid URL";
        }
        return err;
    };
    if (!hasUrlCredentials(endpoint)) {
        endpoint.user = null;
        endpoint.password = null;
    }
    if (policy.authClientHeaders) {
        var generated: std.ArrayList(t.ProviderHeader) = .empty;
        if (h.get(request_headers, "x-goog-api-client") == null) try generated.append(a, .{ .name = "x-goog-api-client", .value = "gl-node/unknown" });
        const user_agent = h.get(request_headers, "user-agent");
        if (user_agent == null or user_agent.?.len == 0) try generated.append(a, .{ .name = "user-agent", .value = "google-api-nodejs-client/10.6.2" }) else if (std.mem.find(u8, user_agent.?, "google-api-nodejs-client/") == null) try generated.append(a, .{ .name = "user-agent", .value = try std.fmt.allocPrint(a, "{s} google-api-nodejs-client/10.6.2", .{user_agent.?}) });
        request_headers = try mergeRequestHeaders(a, &.{ request_headers, generated.items });
    }
    var adapter_headers: std.ArrayList(t.ProviderHeader) = .empty;
    if (h.get(request_headers, "accept") == null) try adapter_headers.append(a, .{ .name = "accept", .value = "*/*" });
    if (h.get(request_headers, "user-agent") == null) try adapter_headers.append(a, .{ .name = "user-agent", .value = "node-fetch" });
    if (body) |bytes| try adapter_headers.append(a, .{ .name = "content-length", .value = try std.fmt.allocPrint(a, "{d}", .{bytes.len}) }) else if (method == .POST or method == .PUT) try adapter_headers.append(a, .{ .name = "content-length", .value = "0" });
    request_headers = try mergeRequestHeaders(a, &.{ request_headers, adapter_headers.items });
    var attempt: u32 = 0;
    while (true) {
        var buffered = readResponse(a, io, network, .{ .method = method, .uri = endpoint, .body = body, .headers = request_headers, .timeoutMs = if (policy.timeoutMs == 0) null else if (policy.timeoutMs) |milliseconds| @floatFromInt(milliseconds) else null, .clientCertificate = policy.clientCertificate }) catch |err| {
            if (err == error.Canceled or err == error.OutOfMemory or err == error.RequestTimeout or err == error.MtlsTransportUnavailable or !retryMethod(method) or attempt >= policy.maxRetries or attempt >= policy.noResponseRetries) {
                if (err == error.RequestTimeout) {
                    if (policy.failureName) |slot| slot.* = "Error";
                    if (policy.errorMessage) |slot| slot.* = timeoutMessage;
                }
                if (err == error.HttpMethodBodyUnsupported) {
                    if (policy.failureName) |slot| slot.* = "Error";
                    if (policy.errorMessage) |slot| slot.* = "Request with GET/HEAD method cannot have body";
                }
                return err;
            }
            try wait(io, attempt);
            attempt += 1;
            continue;
        };
        var released = false;
        defer if (!released) buffered.response.deinit();
        const response = buffered.response;
        defer a.free(buffered.bytes);
        const received = try responseHeaders(a, io, response.headers);
        var retained_headers = false;
        defer if (!retained_headers) freeResponseHeaders(a, received);
        const content_type_value = h.get(received, "content-type");
        const media_type = if (content_type_value) |value| try std.ascii.allocLowerString(a, value) else null;
        defer if (media_type) |value| a.free(value);
        const json_type = if (media_type) |value| std.mem.find(u8, value, "application/json") != null else false;
        const blob_type = response.status != 204 and policy.responseType == .auto and media_type != null and !json_type and !std.mem.startsWith(u8, media_type.?, "text/");
        // Explicit text/JSON and automatic textual responses use Fetch text.
        // Blob's own token fields are absent; retain its reachable empty own
        // property record without claiming a JavaScript Blob object identity.
        const bytes = if (response.status == 204) try a.dupe(u8, "") else if (blob_type) try a.dupe(u8, buffered.bytes) else try fetchText(a, buffered.bytes);
        var retained = false;
        defer if (!retained) a.free(bytes);
        var value = if (blob_type) @import("../../json/value.zig").object() else if (response.status == 204 or policy.responseType == .text or (policy.responseType == .auto and !json_type)) std.json.Value{ .string = bytes } else @import("../../json/value.zig").parseFromSliceLeaky(std.json.Value, a, bytes, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |err| blk: {
            if (err == error.OutOfMemory) return err;
            break :blk std.json.Value{ .string = bytes };
        };
        var failure_message: ?[]const u8 = null;
        errdefer if (failure_message) |message| a.free(message);
        if (response.status < 200 or response.status >= 300) {
            const explanation = errorMessage(a, .{ .status = response.status, .body = value, .raw = bytes, .headers = received }) catch |err| {
                const message = switch (err) {
                    error.GoogleGaxiosNullErrorEntry => "Cannot use 'in' operator to search for 'message' in null",
                    error.CannotConvertObjectToPrimitive => "Cannot convert object to primitive value",
                    else => return err,
                };
                if (retryMethod(method) and attempt < policy.maxRetries and attempt < policy.noResponseRetries) {
                    buffered.response.deinit();
                    released = true;
                    try wait(io, attempt);
                    attempt += 1;
                    continue;
                }
                if (policy.failureName) |slot| slot.* = "Error";
                if (policy.errorMessage) |slot| slot.* = message;
                return error.GoogleGaxiosErrorExtractionFailed;
            };
            if (retryable(response.status) and retryMethod(method) and attempt < policy.maxRetries) {
                a.free(explanation);
                buffered.response.deinit();
                released = true;
                try wait(io, attempt);
                attempt += 1;
                continue;
            }
            failure_message = explanation;
        }
        if (failure_message != null) redactResponse(a, io, &value, received) catch |err| {
            if (err == error.GoogleGaxiosRedactionFailed) {
                if (policy.failureName) |slot| slot.* = "TypeError";
                if (policy.errorMessage) |slot| slot.* = "obj.forEach is not a function";
            }
            return err;
        };
        if (response.status < 200 or response.status >= 300) if (policy.failure) |slot| {
            slot.* = try FailureResponse.init(policy.errorAllocator orelse a, value, response.status);
        };
        retained = true;
        retained_headers = true;
        return .{ .status = response.status, .raw = bytes, .body = value, .headers = received, .failureMessage = failure_message };
    }
}
pub fn request(a: std.mem.Allocator, io: std.Io, network: transport.Transport, method: std.http.Method, url: []const u8, body: ?[]const u8, content_type: ?[]const u8, extra: t.ProviderHeaders) !Response {
    return requestWithPolicy(a, io, network, method, url, body, content_type, extra, .{});
}
pub fn postForm(a: std.mem.Allocator, io: std.Io, network: transport.Transport, url: []const u8, fields: []const uri.Pair) !Response {
    return postFormWithPolicy(a, io, network, url, fields, .{ .authClientHeaders = true });
}
pub fn postFormWithPolicy(a: std.mem.Allocator, io: std.Io, network: transport.Transport, url: []const u8, fields: []const uri.Pair, policy: Policy) !Response {
    return requestWithPolicy(a, io, network, .POST, url, try uri.encodeForm(a, fields), "application/x-www-form-urlencoded;charset=UTF-8", &.{}, policy);
}
