//! Public classifier transport helpers; std.json owns the wire boundaries.
const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const native = @import("nonchat.zig");
pub const ClassifierHttpError = struct { message: []const u8, name: []const u8 = "Error", status: ?u16 = null, headers: ?t.ProviderHeaders = null, body: []const u8 = "" };
pub fn isRecord(value: std.json.Value) bool {
    return value == .object;
}
pub fn requiredNumber(a: std.mem.Allocator, label: []const u8, value: std.json.Value, field: []const u8, error_message: ?*?[]const u8) !f64 {
    return native.number(value) catch |err| {
        if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "{s} returned an invalid {s}", .{ label, field });
        return err;
    };
}
pub fn parseClassifierUsage(a: std.mem.Allocator, value: std.json.Value, model: t.Model) !?t.Usage {
    return native.parseUsage(a, model, value);
}
/// Results and optional failure details use operation.allocator. The original
/// native error is returned; HTTP/timeout fields remain available in detail.
pub fn postClassifierRequest(operation: @import("../models.zig").OperationContext, label: []const u8, url: std.Uri, model: t.Model, body: std.json.Value, options: t.ClassifierOptions, no_retry_statuses: []const u16, detail: ?*?ClassifierHttpError) !std.json.Value {
    if (detail) |slot| slot.* = null;
    if (options.errorMessage) |slot| slot.* = null;
    const a = operation.allocator;
    var client = native.Client{ .operation = operation, .model = model, .options = options, .errorLabel = label, .captureHttpError = detail != null, .noRetryStatuses = no_retry_statuses };
    return client.post(a, url, body, true, true, 2, null) catch |err| {
        if (err == error.OutOfMemory) return err;
        const provided = if (options.errorMessage) |slot| slot.* else null;
        const message = if (err == error.ProviderHttpError) try std.fmt.allocPrint(a, "{s} returned {d}", .{ label, client.status orelse 0 }) else if (err == error.RequestTimeout) try std.fmt.allocPrint(a, "Request timed out after {s}ms", .{try @import("../utils/numbers.zig").toString(a, options.timeoutMs orelse 0)}) else if (err == error.Canceled) provided orelse "Request aborted" else if (err == error.MissingAuthentication) try std.fmt.allocPrint(a, "No API key for provider: {s}", .{model.provider}) else provided orelse client.failureMessage orelse @errorName(err);
        if (options.errorMessage) |slot| slot.* = message;
        if (detail) |slot| {
            if (err == error.ProviderHttpError) {
                const decoded = if (client.errorBody) |bytes| try std.fmt.allocPrint(a, "{f}", .{std.unicode.fmtUtf8(bytes)}) else "";
                const text = if (std.mem.startsWith(u8, decoded, &.{ 0xef, 0xbb, 0xbf })) decoded[3..] else decoded;
                slot.* = .{ .message = message, .status = client.status, .headers = client.failureHeaders, .body = text };
            } else if (err == error.RequestTimeout) slot.* = .{ .message = message, .name = "TimeoutError" };
        }
        return err;
    };
}
