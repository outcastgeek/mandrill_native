const std = @import("std");
const t = @import("../types.zig");
const auth = @import("../auth/types.zig");
pub fn validateTimeout(options: t.StreamOptions, error_message: ?*?[]const u8) !void {
    if (!@import("../json/value.zig").fieldPresent(options, "timeoutMs")) return;
    const value = options.timeoutMs orelse {
        if (error_message) |slot| slot.* = "timeout must be an integer";
        return error.InvalidSdkTimeout;
    };
    if (!std.math.isFinite(value) or @trunc(value) != value) {
        if (error_message) |slot| slot.* = "timeout must be an integer";
        return error.InvalidSdkTimeout;
    }
    if (value < 0) {
        if (error_message) |slot| slot.* = "timeout must be a positive integer";
        return error.InvalidSdkTimeout;
    }
}
fn platform(a: std.mem.Allocator, version: []const u8) !t.ProviderHeaders {
    // Retain the pinned SDK's wire metadata and its unknown-host fallback.
    // There is no Node, Bun, browser, or JavaScript runtime in this adapter.
    return a.dupe(t.ProviderHeader, &.{
        .{ .name = "x-stainless-lang", .value = "js" },         .{ .name = "x-stainless-package-version", .value = version },
        .{ .name = "x-stainless-os", .value = "Unknown" },      .{ .name = "x-stainless-arch", .value = "unknown" },
        .{ .name = "x-stainless-runtime", .value = "unknown" }, .{ .name = "x-stainless-runtime-version", .value = "unknown" },
    });
}
fn ambient(a: std.mem.Allocator, io: std.Io, context: ?auth.AuthContext, name: []const u8) !?[]const u8 {
    const host = context orelse return null;
    const raw = (host.env(a, io, name) catch |err| {
        if (err == error.Canceled or err == error.OutOfMemory) return err;
        return null;
    }) orelse return null;
    defer a.free(raw);
    try io.checkCancel();
    const value = try @import("../utils/unicode.zig").trimWhitespace(a, raw);
    if (value.len == 0) {
        a.free(value);
        return null;
    }
    return value;
}
/// The SDK constructor parses this ambient record before merging caller
/// defaults. Header names use its exact HTTP-token regex; values use std
/// Unicode trimming. This helper reads only the explicit environment context.
pub fn openAICustomHeaders(a: std.mem.Allocator, io: std.Io, context: ?auth.AuthContext, error_message: ?*?[]const u8) !t.ProviderHeaders {
    const j = @import("../json/value.zig");
    const parsed = try customHeaderObject(a, io, context, "OPENAI_CUSTOM_HEADERS");
    const entries = try j.objectEntries(a, parsed.object);
    defer a.free(entries);
    for (entries) |entry| {
        const name = entry.key;
        if (!try @import("../regex/regexp.zig").matches(a, io, "^[!#$%&'*+\\-.^_`|~0-9A-Za-z]+$", name, .{})) {
            if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Header name must be a valid HTTP token [\"{s}\"]", .{name});
            return error.InvalidSdkHeaderName;
        }
    }
    return @import("../utils/headers.zig").merge(a, &.{try @import("../utils/headers.zig").fromValue(a, parsed)});
}
fn customHeaderObject(a: std.mem.Allocator, io: std.Io, context: ?auth.AuthContext, env_name: []const u8) !std.json.Value {
    const j = @import("../json/value.zig");
    var parsed = j.map(a);
    const source = (try ambient(a, io, context, env_name)) orelse return parsed;
    var lines = std.mem.splitScalar(u8, source, '\n');
    while (lines.next()) |line| {
        const raw_name, const raw_value = std.mem.cutScalar(u8, line, ':') orelse continue;
        const name = try @import("../utils/unicode.zig").trimWhitespace(a, raw_name);
        if (std.mem.eql(u8, name, "__proto__")) continue;
        try j.put(a, &parsed, name, j.str(try @import("../utils/unicode.zig").trimWhitespace(a, raw_value)));
    }
    return parsed;
}
/// Anthropic spreads the parsed environment object and caller defaults into
/// one ordered object before any SDK header normalization or accumulation.
pub fn anthropicCustomHeaders(a: std.mem.Allocator, io: std.Io, context: ?auth.AuthContext) !t.ProviderHeaders {
    return @import("../utils/headers.zig").fromValue(a, try customHeaderObject(a, io, context, "ANTHROPIC_CUSTOM_HEADERS"));
}
/// Each outer retry creates a fresh SDK-equivalent attempt, so the SDK's own
/// retry-count header remains zero. Callers merge provider overrides afterward.
pub fn openAI(a: std.mem.Allocator, io: std.Io, context: ?auth.AuthContext, timeout_ms: ?f64) !t.ProviderHeaders {
    var out: std.ArrayList(t.ProviderHeader) = .empty;
    errdefer out.deinit(a);
    try out.appendSlice(a, &.{ .{ .name = "accept", .value = "application/json" }, .{ .name = "user-agent", .value = "OpenAI/JS 7.19.0" }, .{ .name = "x-stainless-retry-count", .value = "0" } });
    try out.appendSlice(a, try platform(a, "7.19.0"));
    if (timeout_ms) |timeout| {
        if (timeout > 0) try out.append(a, .{ .name = "x-stainless-timeout", .value = try @import("../utils/numbers.zig").toString(a, @floor(timeout / 1000)) });
    }
    if (try ambient(a, io, context, "OPENAI_ORG_ID")) |value| try out.append(a, .{ .name = "openai-organization", .value = value });
    if (try ambient(a, io, context, "OPENAI_PROJECT_ID")) |value| try out.append(a, .{ .name = "openai-project", .value = value });
    return out.toOwnedSlice(a);
}
pub fn anthropic(a: std.mem.Allocator, timeout_ms: f64) !t.ProviderHeaders {
    var out: std.ArrayList(t.ProviderHeader) = .empty;
    errdefer out.deinit(a);
    try out.appendSlice(a, &.{ .{ .name = "user-agent", .value = "Anthropic/JS 0.129.0" }, .{ .name = "x-stainless-retry-count", .value = "0" }, .{ .name = "x-stainless-timeout", .value = try @import("../utils/numbers.zig").toString(a, @floor(timeout_ms / 1000)) } });
    try out.appendSlice(a, try platform(a, "0.129.0"));
    return out.toOwnedSlice(a);
}
/// The pinned Anthropic SDK accumulates this one header across sources and
/// differently-cased object keys. Explicit null clears its ordered token set.
pub fn mergeAnthropic(a: std.mem.Allocator, sources: []const t.ProviderHeaders) !t.ProviderHeaders {
    const headers = @import("../utils/headers.zig");
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    var tokens: std.StringArrayHashMapUnmanaged(void) = .{};
    var present = false;
    for (sources) |source| for (source) |header| if (std.ascii.eqlIgnoreCase(header.name, "x-stainless-helper")) {
        present = true;
        if (header.value) |value| {
            var parts = std.mem.splitScalar(u8, value, ',');
            while (parts.next()) |part| {
                const token = try @import("../utils/unicode.zig").trimWhitespace(scratch, part);
                if (token.len > 0) try tokens.put(scratch, token, {});
            }
        } else tokens.clearRetainingCapacity();
    };
    const result = try headers.merge(a, sources);
    if (!present) return result;
    // A nonnull empty value remains a present empty header in buildHeaders.
    var last_nonnull = false;
    for (sources) |source| for (source) |header| if (std.ascii.eqlIgnoreCase(header.name, "x-stainless-helper")) {
        last_nonnull = header.value != null;
    };
    return headers.merge(a, &.{ result, &.{.{ .name = "x-stainless-helper", .value = if (last_nonnull) try std.mem.join(a, ", ", tokens.keys()) else null }} });
}
