const std = @import("std");
const t = @import("../types.zig");
const length = @import("unicode.zig").utf16Length;
const text = @import("text.zig");
const j = @import("../json/value.zig");
pub const ContextUsageEstimate = struct {
    tokens: f64,
    usageTokens: f64,
    trailingTokens: f64,
    lastUsageIndex: ?usize,
    jsonFields: std.json.Value = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub fn calculateContextTokens(a: std.mem.Allocator, usage: t.Usage) !std.json.Value {
    if (try j.fieldValue(a, usage, "totalTokens")) |total| if (j.truthy(total)) return total;
    return @import("usage.zig").sum(a, usage, &.{ "input", "output", "cacheRead", "cacheWrite" });
}
pub fn estimateTextTokens(bytes: []const u8) !f64 {
    return @ceil(@as(f64, @floatFromInt(try length(bytes))) / 3.5);
}
pub fn estimateTextAndImageContentTokens(content: t.MessageContent) !f64 {
    if (content == .missing or content == .nullContent) return error.InvalidMessageContent;
    if (content == .string) return estimateTextTokens(content.string);
    var chars: usize = 0;
    for (content.blocks) |block| {
        chars += if (block == .text) try length(block.text.text) else 4800;
    }
    return @ceil(@as(f64, @floatFromInt(chars)) / 3.5);
}
fn toolsTokens(a: std.mem.Allocator, tools: anytype) !f64 {
    const values = tools orelse return 0;
    if (values.len == 0) return 0;
    const encoded = try @import("../json/value.zig").stringify(a, values, .{ .emit_null_optional_fields = false });
    defer a.free(encoded);
    return estimateTextTokens(encoded);
}
pub fn estimateMessageTokens(a: std.mem.Allocator, message: t.Message) !f64 {
    switch (message) {
        .system => |s| {
            const rendered = try text.getSystemMessageText(a, s);
            defer a.free(rendered);
            return try estimateTextTokens(rendered) + try toolsTokens(a, s.toolsAdded) + try toolsTokens(a, s.toolsRemoved);
        },
        .user => |u| return estimateTextAndImageContentTokens(u.content),
        .toolResult => |r| return estimateTextAndImageContentTokens(.{ .blocks = r.content }),
        .assistant => |assistant| {
            var chars: usize = 0;
            for (assistant.content) |block| switch (block) {
                .text => |v| chars += try length(v.text),
                .thinking => |v| chars += try length(v.thinking),
                .toolCall => |v| {
                    const encoded = try @import("../json/value.zig").stringify(a, v.arguments, .{});
                    defer a.free(encoded);
                    chars += try length(v.name) + try length(encoded);
                },
                else => {},
            };
            return @ceil(@as(f64, @floatFromInt(chars)) / 3.5);
        },
    }
}
/// Accept a transcript context or a borrowed message list, as the pinned
/// helper does. A native array value remains alive for this entire call.
pub fn estimateContextTokens(a: std.mem.Allocator, context: anytype) !ContextUsageEstimate {
    const messages: []const t.Message = switch (@typeInfo(@TypeOf(context))) {
        .@"struct" => context.messages,
        .array => &context,
        .pointer => |info| if (info.size == .one and @typeInfo(info.child) == .@"struct") context.messages else context,
        else => @compileError("expected a transcript context or a message list"),
    };
    var latest: f64 = -std.math.inf(f64);
    var usage: ?t.Usage = null;
    var index: ?usize = null;
    for (messages, 0..) |message, i| {
        const timestamp = switch (message) {
            inline else => |v| v.timestamp,
        };
        if (message == .assistant) {
            const assistant = message.assistant;
            if (assistant.timestamp >= latest and assistant.stopReason != .aborted and assistant.stopReason != .@"error" and try @import("numbers.zig").coerceValue(a, try calculateContextTokens(a, assistant.usage)) > 0) {
                usage = assistant.usage;
                index = i;
            }
        }
        latest = @import("numbers.zig").maximum(latest, timestamp);
    }
    var trailing: f64 = 0;
    const start = if (index) |i| i + 1 else 0;
    for (messages[start..]) |message| trailing += try estimateMessageTokens(a, message);
    const used = if (usage) |u| try calculateContextTokens(a, u) else j.num(0);
    const total = try @import("cost.zig").addValues(a, used, j.num(trailing));
    var result = ContextUsageEstimate{ .tokens = j.number(total) orelse 0, .usageTokens = j.number(used) orelse 0, .trailingTokens = trailing, .lastUsageIndex = index };
    j.markPresent(&result, "lastUsageIndex");
    if (j.number(total) == null) try j.retainField(a, &result, "tokens", total);
    if (j.number(used) == null) try j.retainField(a, &result, "usageTokens", used);
    return result;
}
