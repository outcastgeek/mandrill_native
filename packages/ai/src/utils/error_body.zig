const std = @import("std");
const j = @import("../json/value.zig");
pub const MAX_PROVIDER_ERROR_BODY_CHARS = 4000;
pub const NormalizedProviderError = struct { status: ?f64 = null, body: ?[]const u8 = null, message: []const u8, messageCarriesBody: bool };
/// Host adapters describe unread streams explicitly, rather than serializing
/// implementation objects as provider response bodies.
pub const ResponseBody = union(enum) { text: []const u8, json: std.json.Value, unread };
pub const SdkError = struct {
    message: []const u8,
    statusCode: ?f64 = null,
    status: ?f64 = null,
    body: ?[]const u8 = null,
    @"error": ?std.json.Value = null,
    metadata: ?struct { httpStatusCode: ?f64 = null } = null,
    response: ?struct { statusCode: ?f64 = null, body: ResponseBody = .unread } = null,
};
pub const ProviderFailure = union(enum) { sdk: SdkError, value: std.json.Value };
/// Inputs are decoded JSON values, so host classes, undefined, functions and
/// unread streams never enter the serializer. Native allocation errors remain
/// errors. The caller owns the resulting string.
pub fn safeJsonStringify(a: std.mem.Allocator, value: std.json.Value) ![]const u8 {
    return @import("../json/value.zig").stringify(a, value, .{});
}
/// An untruncated result borrows text; a truncated result is allocated with a.
pub fn truncateErrorText(a: std.mem.Allocator, text: []const u8, maxChars: usize) ![]const u8 {
    const units = try std.unicode.wtf8ToWtf16LeAlloc(a, text);
    defer a.free(units);
    if (units.len <= maxChars) return text;
    const prefix = try std.unicode.wtf16LeToWtf8Alloc(a, units[0..maxChars]);
    defer a.free(prefix);
    return std.fmt.allocPrint(a, "{s}... [truncated {d} chars]", .{ prefix, units.len - maxChars });
}
fn bodyObject(a: std.mem.Allocator, value: ?std.json.Value) !?[]const u8 {
    const object = value orelse return null;
    if (object != .object or object.object.count() == 0) return null;
    return try safeJsonStringify(a, object);
}
pub fn normalizeProviderError(a: std.mem.Allocator, failure: ProviderFailure) !NormalizedProviderError {
    if (failure == .value) return .{ .message = try safeJsonStringify(a, failure.value), .messageCarriesBody = false };
    const sdk = failure.sdk;
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    const status = sdk.statusCode orelse sdk.status orelse (if (sdk.metadata) |metadata| metadata.httpStatusCode else null) orelse (if (sdk.response) |response| response.statusCode else null);
    var picked = sdk.body orelse try bodyObject(scratch, sdk.@"error");
    if (picked == null) if (sdk.response) |response| {
        picked = switch (response.body) {
            .text => |text| text,
            .json => |value| try bodyObject(scratch, value),
            .unread => null,
        };
    };
    var body: ?[]const u8 = null;
    if (picked) |text| {
        const trimmed = try @import("unicode.zig").trimWhitespace(a, text);
        errdefer a.free(trimmed);
        if (trimmed.len > 0) {
            const truncated = try truncateErrorText(a, trimmed, MAX_PROVIDER_ERROR_BODY_CHARS);
            if (truncated.ptr != trimmed.ptr) a.free(trimmed);
            body = truncated;
        } else a.free(trimmed);
    }
    return .{ .status = status, .body = body, .message = sdk.message, .messageCarriesBody = body == null or std.mem.find(u8, sdk.message, body.?) != null };
}
pub fn formatProviderError(a: std.mem.Allocator, norm: NormalizedProviderError, prefix: ?[]const u8) ![]const u8 {
    const message = if (norm.messageCarriesBody or norm.status == null or norm.body == null) norm.message else norm.body.?;
    if (norm.status) |status| {
        if (prefix != null or (!norm.messageCarriesBody and norm.body != null)) {
            var temporary = std.heap.ArenaAllocator.init(a);
            defer temporary.deinit();
            const status_text = try @import("numbers.zig").toString(temporary.allocator(), status);
            if (prefix) |label| return std.fmt.allocPrint(a, "{s} ({s}): {s}", .{ label, status_text, message });
            return std.fmt.allocPrint(a, "{s}: {s}", .{ status_text, message });
        }
    }
    return message;
}
/// Preserve non-JSON error bodies; structured bodies are parsed by std.json.
pub fn format(a: std.mem.Allocator, status: u16, body: []const u8) ![]const u8 {
    const parsed = @import("../json/value.zig").parseFromSlice(std.json.Value, a, body, .{ .duplicate_field_behavior = .use_last }) catch |err| {
        if (err == error.OutOfMemory) return err;
        return std.fmt.allocPrint(a, "{d} {s}", .{ status, body });
    };
    defer parsed.deinit();
    const error_value = j.get(parsed.value, "error") orelse parsed.value;
    const message = j.field(error_value, "message") orelse j.text(error_value);
    const encoded = if (message == null) try @import("../json/value.zig").stringify(a, error_value, .{}) else message.?;
    defer if (message == null) a.free(encoded);
    return std.fmt.allocPrint(a, "{d} {s}", .{ status, encoded });
}
