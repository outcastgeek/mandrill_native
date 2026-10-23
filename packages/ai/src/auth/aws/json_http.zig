const std = @import("std");
const m = @import("../../models.zig");
const t = @import("../../types.zig");
const j = @import("../../json/value.zig");
const http = @import("../oauth/http.zig");
const retry = @import("retry.zig");
pub fn numericSchemaValue(value: std.json.Value) std.json.Value {
    if (value == .string) {
        if (std.mem.eql(u8, value.string, "Infinity")) return j.num(std.math.inf(f64));
        if (std.mem.eql(u8, value.string, "-Infinity")) return j.num(-std.math.inf(f64));
        if (std.mem.eql(u8, value.string, "NaN")) return j.num(std.math.nan(f64));
    }
    return value;
}
pub fn structField(value: std.json.Value, key: []const u8) ?std.json.Value {
    const member = j.get(value, key) orelse return null;
    return if (member == .null and j.field(value, "__type") == null) null else member;
}
pub fn stringSchemaMember(a: std.mem.Allocator, value: std.json.Value) !std.json.Value {
    switch (value) {
        .integer, .float, .number_string => {
            const numeric = j.number(value) orelse return error.InvalidJsonNumber;
            if (!std.math.isFinite(numeric)) return j.str(try @import("../../utils/numbers.zig").toString(a, numeric));
        },
        else => {},
    }
    // Objects/arrays on string schemas use ordinary JSON.stringify; nested
    // non-finite values retain the standard compatible tree's null rule.
    return j.clone(a, value);
}
pub const HeaderFactory = struct { context: *anyopaque, call: *const fn (*anyopaque, m.OperationContext) anyerror!t.ProviderHeaders };
pub fn request(operation: m.OperationContext, options: t.StreamOptions, method: std.http.Method, url: []const u8, body: ?std.json.Value, headers: t.ProviderHeaders) !http.Response {
    return requestWithFactory(operation, options, method, url, body, headers, null);
}
pub fn requestWithFactory(operation: m.OperationContext, options: t.StreamOptions, method: std.http.Method, url: []const u8, body: ?std.json.Value, headers: t.ProviderHeaders, factory: ?HeaderFactory) !http.Response {
    const a = operation.allocator;
    var policy = try retry.Policy.init(operation, options);
    const encoded = if (body) |value| try @import("../../json/value.zig").stringify(a, value, .{}) else null;
    while (true) {
        var extra = try policy.requestHeaders(a, headers);
        if (factory) |value| extra = try @import("../../utils/headers.zig").merge(a, &.{ extra, try value.call(value.context, operation) });
        const response = http.requestWithOptions(a, operation.io, operation.transport, method, url, encoded, if (body != null) "application/json" else null, extra, .{ .decoding = .smithy, .followRedirects = false, .decompressResponse = false }) catch |err| {
            if (err == error.Canceled or err == error.OutOfMemory) return err;
            if (try policy.waitForRetry(retry.transportError(err), &.{})) continue;
            return err;
        };
        if (response.ok()) {
            if (response.jsonError) |err| return err;
            return response;
        }
        const code = j.field(response.body, "__type") orelse j.field(response.body, "code") orelse j.field(response.body, "Code");
        _ = try policy.responseClock(response.headers, null);
        if (try policy.waitForRetry(retry.classify(response.status, code, j.field(response.body, "message") orelse j.field(response.body, "Message")), response.headers)) continue;
        if (operation.errorMessage) |slot| slot.* = j.field(response.body, "message") orelse j.field(response.body, "Message") orelse code;
        return response;
    }
}
