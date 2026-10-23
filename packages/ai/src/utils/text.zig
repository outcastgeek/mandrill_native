const std = @import("std");
const t = @import("../types.zig");
/// String content is borrowed; joined block content is allocated with a.
pub fn contentText(a: std.mem.Allocator, content: t.MessageContent, separator: []const u8) ![]const u8 {
    switch (content) {
        .missing, .nullContent => return error.InvalidMessageContent,
        .string => |s| return s,
        .blocks => |blocks| {
            var parts: std.ArrayList([]const u8) = .empty;
            defer parts.deinit(a);
            for (blocks) |block| if (block == .text) {
                try parts.append(a, block.text.text);
            };
            return std.mem.join(a, separator, parts.items);
        },
    }
}
pub fn getSystemMessageText(a: std.mem.Allocator, message: t.SystemMessage) ![]const u8 {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    var parts: std.ArrayList([]const u8) = .empty;
    const text = try contentText(scratch, message.content, "\n");
    if (text.len > 0) try parts.append(scratch, text);
    if (message.sections) |sections| if (sections == .object) {
        const entries = try @import("../json/value.zig").objectEntries(scratch, sections.object);
        for (entries) |entry| if (entry.node.* == .string and entry.node.string.len > 0) {
            try parts.append(scratch, entry.node.string);
        };
    };
    return std.mem.join(a, "\n\n", parts.items);
}
pub fn renderSystemMessageUpdate(a: std.mem.Allocator, message: t.SystemMessage) ![]const u8 {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    var parts: std.ArrayList([]const u8) = .empty;
    const text = try contentText(scratch, message.content, "\n");
    if (text.len > 0) try parts.append(scratch, text);
    if (message.sections) |sections| if (sections == .object) {
        const entries = try @import("../json/value.zig").objectEntries(scratch, sections.object);
        for (entries) |entry| {
            try parts.append(scratch, if (entry.node.* == .null)
                try std.fmt.allocPrint(scratch, "Removed system prompt section \"{s}\".", .{entry.key})
            else
                try std.fmt.allocPrint(scratch, "Updated system prompt section \"{s}\":\n\n{s}", .{ entry.key, try @import("diagnostics.zig").formatThrownValue(scratch, .{ .value = entry.node.* }) }));
        }
    };
    return std.mem.join(a, "\n\n", parts.items);
}
