const std = @import("std");
const t = @import("../types.zig");
pub fn inferCopilotInitiator(messages: []const t.Message) []const u8 {
    return if (messages.len > 0 and messages[messages.len - 1] != .user) "agent" else "user";
}
pub fn hasCopilotVisionInput(messages: []const t.Message) bool {
    for (messages) |message| {
        const blocks: []const t.Content = switch (message) {
            .user => |value| if (value.content == .blocks) value.content.blocks else &.{},
            .toolResult => |value| value.content,
            else => &.{},
        };
        for (blocks) |block| if (block == .image) return true;
    }
    return false;
}
pub fn buildCopilotDynamicHeaders(a: std.mem.Allocator, messages: []const t.Message, hasImages: bool) !t.ProviderHeaders {
    var result: std.ArrayList(t.ProviderHeader) = .empty;
    try result.appendSlice(a, &.{ .{ .name = "X-Initiator", .value = inferCopilotInitiator(messages) }, .{ .name = "Openai-Intent", .value = "conversation-edits" } });
    if (hasImages) try result.append(a, .{ .name = "Copilot-Vision-Request", .value = "true" });
    return result.toOwnedSlice(a);
}
