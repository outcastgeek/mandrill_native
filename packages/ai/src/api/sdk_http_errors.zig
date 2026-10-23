//! Provider HTTP error shapes.
//! Structured bodies are decoded and serialized exclusively through std.json.
const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const errors = @import("../utils/error_body.zig");
const Value = std.json.Value;
fn fetchText(a: std.mem.Allocator, body: []const u8) ![]const u8 {
    const decoded = try std.fmt.allocPrint(a, "{f}", .{std.unicode.fmtUtf8(body)});
    var reader = std.Io.Reader.fixed(decoded);
    if (std.mem.startsWith(u8, decoded, &.{ 0xef, 0xbb, 0xbf })) reader.toss(3);
    return reader.buffered();
}
fn parsedBody(a: std.mem.Allocator, body: []const u8) !?Value {
    return @import("../json/value.zig").parseFromSliceLeaky(Value, a, body, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |err| {
        if (err == error.OutOfMemory) return err;
        return null;
    };
}
fn sdkMessage(a: std.mem.Allocator, status: ?u16, inner: Value, fallback: ?[]const u8) ![]const u8 {
    const nested = j.get(inner, "message") orelse .null;
    const message = if (j.truthy(nested)) (j.text(nested) orelse try @import("../json/value.zig").stringify(a, nested, .{})) else if (j.truthy(inner)) try @import("../json/value.zig").stringify(a, inner, .{}) else fallback;
    if (status) |code| {
        if (message) |text| if (text.len > 0) return std.fmt.allocPrint(a, "{d} {s}", .{ code, text });
        return std.fmt.allocPrint(a, "{d} status code (no body)", .{code});
    }
    return if (message) |text| (if (text.len > 0) text else "(no status code or body)") else "(no status code or body)";
}
fn openAICompletionsFailure(a: std.mem.Allocator, failure: errors.ProviderFailure) ![]const u8 {
    const normalized = try errors.normalizeProviderError(a, failure);
    var formatted = try errors.formatProviderError(a, normalized, null);
    if (failure == .sdk) if (j.field(j.get(failure.sdk.@"error" orelse .null, "metadata") orelse .null, "raw")) |metadata| if (metadata.len > 0 and std.mem.find(u8, formatted, metadata) == null) {
        formatted = try std.fmt.allocPrint(a, "{s}\n{s}", .{ formatted, metadata });
    };
    return formatted;
}
pub fn openAICompletionsStreamFailure(a: std.mem.Allocator, inner: Value) ![]const u8 {
    return openAICompletionsFailure(a, .{ .sdk = .{ .message = try sdkMessage(a, null, inner, null), .@"error" = inner } });
}
pub fn openAIStreamFailure(a: std.mem.Allocator, model: t.Model, inner: Value) ![]const u8 {
    if (std.mem.eql(u8, model.api, "openai-completions")) return openAICompletionsStreamFailure(a, inner);
    const failure = errors.ProviderFailure{ .sdk = .{ .message = try sdkMessage(a, null, inner, null), .@"error" = inner } };
    if (std.mem.eql(u8, model.api, "openai-responses")) return openAIResponsesFailure(a, model, failure);
    return errors.formatProviderError(a, try errors.normalizeProviderError(a, failure), "Azure OpenAI API error");
}
pub fn openAIResponsesFailure(a: std.mem.Allocator, model: t.Model, failure: errors.ProviderFailure) ![]const u8 {
    const normalized = try errors.normalizeProviderError(a, failure);
    const prefix = try std.fmt.allocPrint(a, "{s} API error", .{if (std.mem.eql(u8, model.provider, "openai")) "OpenAI" else model.provider});
    const formatted = try errors.formatProviderError(a, normalized, prefix);
    const usage_hint = "\nCheck your ChatGPT usage: https://chatgpt.com/settings/usage";
    if (std.mem.find(u8, formatted, "subscription_sharing_usage_limit_exceeded") != null and !std.mem.endsWith(u8, formatted, usage_hint)) return std.fmt.allocPrint(a, "{s}{s}", .{ formatted, usage_hint });
    return formatted;
}
pub fn openAI(a: std.mem.Allocator, model: t.Model, status: u16, body: []const u8) ![]const u8 {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    const text = try fetchText(scratch, body);
    const parsed = (try parsedBody(scratch, text)) orelse .null;
    // OpenAI's APIError.generate selects only the outer `error` member;
    // Anthropic's SDK, below, intentionally retains the entire parsed body.
    const inner = j.get(parsed, "error") orelse .null;
    const message = try sdkMessage(a, status, inner, if (j.truthy(parsed)) null else text);
    const failure = errors.ProviderFailure{ .sdk = .{ .message = message, .status = @floatFromInt(status), .@"error" = inner } };
    if (std.mem.eql(u8, model.api, "openai-responses")) return openAIResponsesFailure(a, model, failure);
    if (std.mem.eql(u8, model.api, "openai-completions")) return openAICompletionsFailure(a, failure);
    const normalized = try errors.normalizeProviderError(a, failure);
    const azure = std.mem.eql(u8, model.api, "azure-openai-responses");
    const prefix: ?[]const u8 = if (azure) "Azure OpenAI API error" else null;
    return errors.formatProviderError(a, normalized, prefix);
}
/// retryProviderRequest includes the SDK Error.message in delay-cap errors,
/// before the outer provider display normalization runs.
pub fn openAIRequestMessage(a: std.mem.Allocator, status: u16, body: []const u8) ![]const u8 {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    const text = try fetchText(scratch, body);
    const parsed = (try parsedBody(scratch, text)) orelse .null;
    return sdkMessage(a, status, j.get(parsed, "error") orelse .null, if (j.truthy(parsed)) null else text);
}
pub fn anthropic(a: std.mem.Allocator, status: u16, body: []const u8) ![]const u8 {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    const text = try fetchText(scratch, body);
    const parsed = (try parsedBody(scratch, text)) orelse .null;
    return sdkMessage(a, status, parsed, if (j.truthy(parsed)) null else text);
}
pub fn google(a: std.mem.Allocator, status: u16, status_text: []const u8, response_headers: t.ProviderHeaders, body: []const u8) ![]const u8 {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    const text = try fetchText(scratch, body);
    var is_json = false;
    for (response_headers) |header| if (std.ascii.eqlIgnoreCase(header.name, "content-type")) {
        if (header.value) |value| is_json = std.mem.find(u8, value, "application/json") != null;
        break;
    };
    const value = if (is_json) try @import("../json/value.zig").parseFromSliceLeaky(Value, scratch, text, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) else try j.from(scratch, .{ .@"error" = .{ .message = text, .code = status, .status = status_text } });
    return @import("../json/value.zig").stringify(a, value, .{});
}
