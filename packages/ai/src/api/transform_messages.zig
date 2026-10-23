const std = @import("std");
const t = @import("../types.zig");
pub const NormalizeId = *const fn (std.mem.Allocator, []const u8, t.Model, t.AssistantMessage) anyerror![]const u8;
pub const IdNormalizer = struct {
    context: ?*anyopaque = null,
    call: *const fn (?*anyopaque, std.mem.Allocator, []const u8, t.Model, t.AssistantMessage) anyerror![]const u8,
};
/// The pinned non-Unicode identifier replacement operates on UTF-16 units.
pub fn normalizeAsciiToolId(a: std.mem.Allocator, id: []const u8, limit: usize) ![]const u8 {
    const units = try std.unicode.wtf8ToWtf16LeAlloc(a, id);
    defer a.free(units);
    const out = try a.alloc(u8, @min(limit, units.len));
    for (out, units[0..out.len]) |*byte, unit| byte.* = if (unit <= 0x7f and (std.ascii.isAlphanumeric(@intCast(unit)) or unit == '_' or unit == '-')) @intCast(unit) else '_';
    return out;
}
fn images(a: std.mem.Allocator, content: []const t.Content, placeholder: []const u8) ![]const t.Content {
    var out: std.ArrayList(t.Content) = .empty;
    var previous = false;
    for (content) |block| {
        if (block == .image) {
            if (!previous) try out.append(a, .{ .text = .{ .text = placeholder } });
            previous = true;
        } else {
            try out.append(a, block);
            previous = block == .text and std.mem.eql(u8, block.text.text, placeholder);
        }
    }
    return out.toOwnedSlice(a);
}
pub const SplitErrorToolResult = struct { content: []const t.Content, caption: []const u8, images: []const t.ImageContent };
fn normalizeNullishContent(a: std.mem.Allocator, message: *t.Message) !void {
    switch (message.*) {
        inline .system, .user => |*value| {
            if (value.content != .missing and value.content != .nullContent) return;
            const raw = try @import("../json/value.zig").fieldValue(a, value.*, "content");
            if (raw != null and raw.? != .null) return;
            value.content = .{ .blocks = &.{} };
            @import("../json/value.zig").markPresent(value, "content");
        },
        inline .assistant, .toolResult => |*value| {
            if (value.content.len != 0) return;
            const raw = try @import("../json/value.zig").fieldValue(a, value.*, "content");
            if (raw != null and raw.? != .null) return;
            value.content = &.{};
            @import("../json/value.zig").markPresent(value, "content");
        },
    }
}
pub fn splitErrorToolResultImages(a: std.mem.Allocator, message: t.ToolResultMessage) !?SplitErrorToolResult {
    if (!message.isError) return null;
    var found: std.ArrayList(t.ImageContent) = .empty;
    for (message.content) |block| if (block == .image) {
        try found.append(a, block.image);
    };
    if (found.items.len == 0) return null;
    return .{ .content = try images(a, message.content, "(image attached after the tool results)"), .caption = try std.fmt.allocPrint(a, "Images from the error result of tool call {s} ({s}):", .{ message.toolCallId, message.toolName }), .images = try found.toOwnedSlice(a) };
}
pub fn transformMessages(a: std.mem.Allocator, io: std.Io, messages: []const t.Message, model: t.Model, normalize_id: ?NormalizeId) ![]const t.Message {
    var legacy = normalize_id orelse return transformMessagesWithNormalizer(a, io, messages, model, null);
    return transformMessagesWithNormalizer(a, io, messages, model, .{ .context = @ptrCast(&legacy), .call = struct {
        fn call(ctx: ?*anyopaque, allocator: std.mem.Allocator, id: []const u8, target: t.Model, assistant: t.AssistantMessage) ![]const u8 {
            const normalize: *const NormalizeId = @ptrCast(@alignCast(ctx.?));
            return normalize.*(allocator, id, target, assistant);
        }
    }.call });
}
/// A stateful normalizer runs in transcript order during the first pass,
/// before failed assistant messages and orphaned tool calls are resolved.
pub fn transformMessagesWithNormalizer(a: std.mem.Allocator, io: std.Io, messages: []const t.Message, model: t.Model, normalize_id: ?IdNormalizer) ![]const t.Message {
    // Working vectors hold copies of records. Returned content buffers remain
    // allocated with a; releasing these vectors does not release that content.
    var transformed: std.ArrayList(t.Message) = .empty;
    defer transformed.deinit(a);
    var ids: std.StringHashMapUnmanaged([]const u8) = .{};
    defer ids.deinit(a);
    const vision = for (model.input) |modality| {
        if (modality == .image) break true;
    } else false;
    for (messages) |original| {
        var message = original;
        try normalizeNullishContent(a, &message);
        switch (message) {
            .user => |*user| if (!vision and user.content == .blocks) {
                user.content = .{ .blocks = try images(a, user.content.blocks, "(image omitted: model does not support images)") };
            },
            .toolResult => |*result| {
                if (!vision) result.content = try images(a, result.content, "(tool image omitted: model does not support images)");
                if (ids.get(result.toolCallId)) |id| if (id.len > 0) {
                    result.toolCallId = id;
                };
            },
            .assistant => |*assistant| {
                const same = std.mem.eql(u8, assistant.provider, model.provider) and std.mem.eql(u8, assistant.api, model.api) and std.mem.eql(u8, assistant.model, model.id);
                var content: std.ArrayList(t.Content) = .empty;
                for (assistant.content) |original_block| {
                    var block = original_block;
                    switch (block) {
                        .thinking => |thinking| {
                            if (!same) continue;
                            if (!(thinking.redacted orelse false) and (thinking.thinkingSignature == null or thinking.thinkingSignature.?.len == 0) and try @import("../utils/unicode.zig").isBlank(thinking.thinking)) continue;
                        },
                        .text => |*value| if (!same) {
                            value.* = .{ .text = value.text };
                        },
                        .toolCall => |*call| if (!same) {
                            if (call.thoughtSignature) |signature| if (signature.len > 0) {
                                call.thoughtSignature = null;
                            };
                            if (normalize_id) |normalize| {
                                const id = try normalize.call(normalize.context, a, call.id, model, assistant.*);
                                if (!std.mem.eql(u8, id, call.id)) {
                                    try ids.put(a, call.id, id);
                                    call.id = id;
                                }
                            }
                        },
                        else => {},
                    }
                    try content.append(a, block);
                }
                assistant.content = try content.toOwnedSlice(a);
            },
            else => {},
        }
        try transformed.append(a, message);
    }
    var out: std.ArrayList(t.Message) = .empty;
    var pending: std.ArrayList(t.ToolCall) = .empty;
    defer pending.deinit(a);
    var answered: std.StringHashMapUnmanaged(void) = .{};
    defer answered.deinit(a);
    var held: std.ArrayList(t.Message) = .empty;
    defer held.deinit(a);
    for (transformed.items) |message| switch (message) {
        .assistant => |assistant| {
            try close(a, io, &out, &pending, &answered, &held);
            if (assistant.stopReason == .@"error" or assistant.stopReason == .aborted) {
                // The first pass still ran the normalizer in source order.
                // This newly allocated block vector is omitted from output.
                a.free(assistant.content);
                continue;
            }
            for (assistant.content) |block| if (block == .toolCall) {
                try pending.append(a, block.toolCall);
            };
            try out.append(a, message);
        },
        .toolResult => |result| {
            try answered.put(a, result.toolCallId, {});
            try out.append(a, message);
        },
        .system => {
            if (pending.items.len > 0) try held.append(a, message) else try out.append(a, message);
        },
        .user => {
            try close(a, io, &out, &pending, &answered, &held);
            try out.append(a, message);
        },
    };
    try close(a, io, &out, &pending, &answered, &held);
    return out.toOwnedSlice(a);
}
fn close(a: std.mem.Allocator, io: std.Io, out: *std.ArrayList(t.Message), pending: *std.ArrayList(t.ToolCall), answered: *std.StringHashMapUnmanaged(void), held: *std.ArrayList(t.Message)) !void {
    for (pending.items) |call| if (!answered.contains(call.id)) {
        const content = try a.dupe(t.Content, &.{.{ .text = .{ .text = "No result provided" } }});
        try out.append(a, .{ .toolResult = .{ .toolCallId = call.id, .toolName = call.name, .content = content, .isError = true, .timestamp = @as(f64, @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds())) } });
    };
    pending.clearRetainingCapacity();
    answered.clearRetainingCapacity();
    try out.appendSlice(a, held.items);
    held.clearRetainingCapacity();
}
