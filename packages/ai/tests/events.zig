const std = @import("std");
const ai = @import("ai");
const t = ai.types;
fn message() t.AssistantMessage {
    return .{ .api = "fixture", .provider = "fixture", .model = "fixture", .timestamp = 0, .content = &.{.{ .text = .{ .text = "partial" } }} };
}
test "result requires no event consumption and is repeatable" {
    const handle = try ai.event_stream.AssistantMessageEventStream.create(std.testing.allocator, std.testing.io);
    defer handle.destroy();
    try handle.push(.{ .start = .{ .partial = message() } });
    var final = message();
    final.stopReason = .stop;
    try handle.push(.{ .done = .{ .reason = .stop, .message = final } });
    try handle.push(.{ .text_delta = .{ .contentIndex = 0, .delta = "ignored", .partial = message() } });
    try std.testing.expectEqualStrings("partial", (try handle.result()).content[0].text.text);
    try std.testing.expectEqual(t.StopReason.stop, (try handle.result()).stopReason);
    var count: usize = 0;
    while (try handle.next()) |_| count += 1;
    try std.testing.expectEqual(@as(usize, 2), count);
}
test "event allocation failure publishes one terminal error with partial output" {
    var failing = std.testing.FailingAllocator.init(std.testing.allocator, .{});
    const handle = try ai.event_stream.AssistantMessageEventStream.create(failing.allocator(), std.testing.io);
    defer handle.destroy();
    try handle.push(.{ .start = .{ .partial = message() } });
    failing.fail_index = failing.alloc_index;
    failing.resize_fail_index = failing.resize_index;
    const large = try std.testing.allocator.alloc(u8, 32768);
    defer std.testing.allocator.free(large);
    @memset(large, 'x');
    var partial = message();
    partial.content = &.{.{ .text = .{ .text = large } }};
    try std.testing.expectError(error.OutOfMemory, handle.push(.{ .text_delta = .{ .contentIndex = 0, .delta = large, .partial = partial } }));
    const result = try handle.result();
    try std.testing.expectEqual(t.StopReason.@"error", result.stopReason);
    try std.testing.expectEqualStrings("partial", result.content[0].text.text);
    var terminals: usize = 0;
    while (try handle.next()) |event| if (event == .@"error" or event == .done) {
        terminals += 1;
    };
    try std.testing.expectEqual(@as(usize, 1), terminals);
}
