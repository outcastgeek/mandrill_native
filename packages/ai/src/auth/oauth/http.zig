const std = @import("std");
const j = @import("../../json/value.zig");
const transport = @import("../../transport/http.zig");
const uri = @import("../../transport/uri.zig");
pub const Response = struct {
    status: u16,
    body: std.json.Value,
    raw: []const u8,
    statusText: []const u8 = "",
    headers: @import("../../types.zig").ProviderHeaders = &.{},
    jsonError: ?anyerror = null,
    pub fn ok(self: @This()) bool {
        return self.status >= 200 and self.status < 300;
    }
};
pub fn remember(slot: ?*?[]const u8, message: []const u8) void {
    if (slot) |out| out.* = message;
}
pub const JsonDecoding = enum { fetch, smithy };
pub const RequestOptions = struct { timeoutMs: ?u64 = null, decoding: JsonDecoding = .fetch, followRedirects: bool = true, decompressResponse: bool = true, ignoreHttpErrorBodyReadFailure: bool = false };
/// Shared OAuth response text conversion, including Fetch's UTF8/BOM behavior.
pub fn responseText(a: std.mem.Allocator, wire: []const u8, decoding: JsonDecoding) ![]const u8 {
    const decoded = try std.fmt.allocPrint(a, "{f}", .{std.unicode.fmtUtf8(wire)});
    defer a.free(decoded);
    var reader = std.Io.Reader.fixed(decoded);
    if (decoding == .fetch and std.mem.startsWith(u8, decoded, &.{ 0xef, 0xbb, 0xbf })) reader.toss(3);
    return a.dupe(u8, reader.buffered());
}
pub fn request(a: std.mem.Allocator, io: std.Io, http: transport.Transport, method: std.http.Method, url: []const u8, body: ?[]const u8, content_type: ?[]const u8, extra: @import("../../types.zig").ProviderHeaders) !Response {
    return requestWithTimeout(a, io, http, method, url, body, content_type, extra, null);
}
pub fn requestWithTimeout(a: std.mem.Allocator, io: std.Io, http: transport.Transport, method: std.http.Method, url: []const u8, body: ?[]const u8, content_type: ?[]const u8, extra: @import("../../types.zig").ProviderHeaders, timeout_ms: ?u64) !Response {
    return requestWithDecoding(a, io, http, method, url, body, content_type, extra, timeout_ms, .fetch);
}
pub fn requestWithDecoding(a: std.mem.Allocator, io: std.Io, http: transport.Transport, method: std.http.Method, url: []const u8, body: ?[]const u8, content_type: ?[]const u8, extra: @import("../../types.zig").ProviderHeaders, timeout_ms: ?u64, decoding: JsonDecoding) !Response {
    return requestWithOptions(a, io, http, method, url, body, content_type, extra, .{ .timeoutMs = timeout_ms, .decoding = decoding });
}
pub fn requestWithOptions(a: std.mem.Allocator, io: std.Io, http: transport.Transport, method: std.http.Method, url: []const u8, body: ?[]const u8, content_type: ?[]const u8, extra: @import("../../types.zig").ProviderHeaders, options: RequestOptions) !Response {
    var defaults: std.ArrayList(@import("../../types.zig").ProviderHeader) = .empty;
    try defaults.append(a, .{ .name = "accept", .value = "application/json" });
    if (content_type) |value| try defaults.append(a, .{ .name = "content-type", .value = value });
    const request_uri = if (options.decoding == .fetch) try uri.parseHttpUrl(a, url) else try std.Uri.parse(url);
    var buffered = try http.sendReadAllWithOptions(a, io, .{ .method = method, .uri = request_uri, .headers = try @import("../../utils/headers.zig").merge(a, &.{ defaults.items, extra }), .body = body, .timeoutMs = if (options.timeoutMs) |milliseconds| @floatFromInt(milliseconds) else null, .followRedirects = options.followRedirects, .decompressResponse = options.decompressResponse }, .unlimited, .{ .ignoreHttpErrorBodyReadFailure = options.ignoreHttpErrorBodyReadFailure });
    defer buffered.releaseResponse();
    defer a.free(buffered.bytes);
    const response = buffered.response;
    const bytes = try responseText(a, buffered.bytes, options.decoding);
    errdefer a.free(bytes);
    const status_text = try a.dupe(u8, response.statusText);
    const response_headers = try @import("../../utils/headers.zig").clone(a, response.headers);
    const parsed = if (options.decoding == .smithy and bytes.len == 0) j.map(a) else @import("../../json/value.zig").parseFromSliceLeaky(std.json.Value, a, bytes, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |err| {
        if (err == error.OutOfMemory) return err;
        return .{ .status = response.status, .statusText = status_text, .headers = response_headers, .raw = bytes, .body = j.map(a), .jsonError = err };
    };
    return .{ .status = response.status, .statusText = status_text, .headers = response_headers, .raw = bytes, .body = parsed };
}
pub fn postJson(a: std.mem.Allocator, io: std.Io, http: transport.Transport, url: []const u8, value: anytype) !Response {
    return request(a, io, http, .POST, url, try @import("../../json/value.zig").stringify(a, value, .{ .emit_null_optional_fields = false }), "application/json", &.{});
}
pub fn postJsonWithTimeout(a: std.mem.Allocator, io: std.Io, http: transport.Transport, url: []const u8, value: anytype, timeout_ms: ?u64) !Response {
    return requestWithTimeout(a, io, http, .POST, url, try @import("../../json/value.zig").stringify(a, value, .{ .emit_null_optional_fields = false }), "application/json", &.{}, timeout_ms);
}
pub fn postForm(a: std.mem.Allocator, io: std.Io, http: transport.Transport, url: []const u8, fields: []const uri.Pair) !Response {
    return request(a, io, http, .POST, url, try uri.encodeForm(a, fields), "application/x-www-form-urlencoded", &.{});
}
pub fn postFormWithTimeout(a: std.mem.Allocator, io: std.Io, http: transport.Transport, url: []const u8, fields: []const uri.Pair, timeout_ms: ?u64) !Response {
    return requestWithTimeout(a, io, http, .POST, url, try uri.encodeForm(a, fields), "application/x-www-form-urlencoded", &.{}, timeout_ms);
}
pub fn requiredString(value: std.json.Value, key: []const u8) ![]const u8 {
    const string = j.field(value, key) orelse return error.InvalidOAuthResponse;
    if (string.len == 0) return error.InvalidOAuthResponse;
    return string;
}
pub fn positiveNumber(value: std.json.Value, key: []const u8) !f64 {
    const number = j.number(j.get(value, key) orelse return error.InvalidOAuthResponse) orelse return error.InvalidOAuthResponse;
    if (!std.math.isFinite(number) or number <= 0) return error.InvalidOAuthResponse;
    return number;
}
pub fn credential(io: std.Io, value: std.json.Value, previous_refresh: ?[]const u8, skew: i64, default_lifetime: ?f64) !@import("../types.zig").OAuthCredential {
    const access = try requiredString(value, "access_token");
    const refresh = if (j.get(value, "refresh_token") == null and previous_refresh != null) previous_refresh.? else try requiredString(value, "refresh_token");
    const expires = if (j.get(value, "expires_in") == null and default_lifetime != null) default_lifetime.? else try positiveNumber(value, "expires_in");
    return .{ .access = access, .refresh = refresh, .expires = @import("../../utils/datetime.zig").expiresAfterNumber(io, expires) - @as(f64, @floatFromInt(skew)) };
}
pub fn auth(_: ?*anyopaque, _: std.mem.Allocator, _: std.Io, credentials: @import("../types.zig").OAuthCredential) !@import("../types.zig").ModelAuth {
    return .{ .apiKey = credentials.access };
}
