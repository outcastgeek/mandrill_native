const std = @import("std");
const ai = @import("ai");
const j = ai.ai.json_value;
test "streaming JSON uses standard tokens for nested incomplete values" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const a = arena.allocator();
    const value = try ai.json.parseStreamingJson(a, "{\"text\":\"ab\\u00");
    try std.testing.expectEqualStrings("ab", j.field(value, "text").?);
    const nested = try ai.json.parseStreamingJson(a, "{\"items\":[1,{\"name\":\"caf");
    const items = j.get(nested, "items").?;
    try std.testing.expectEqual(@as(usize, 2), items.array.items.len);
    try std.testing.expectEqualStrings("caf", j.field(items.array.items[1], "name").?);
}
test "final tool arguments reject truncated JSON and non-object JSON" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const a = arena.allocator();
    const empty = try ai.json.parseToolCallArguments(a, "");
    try std.testing.expect(empty.argumentsError == null);
    const truncated = "{\"kind\":\"seek\",\"clipId\":\"clip-fl";
    const result = try ai.json.parseToolCallArguments(a, truncated);
    try std.testing.expect(result.argumentsError != null);
    try std.testing.expect(std.mem.endsWith(u8, result.argumentsError.?, truncated));
    const array = try ai.json.parseToolCallArguments(a, "[1,2]");
    try std.testing.expectEqualStrings("Tool arguments must be a JSON object\n\nReceived arguments:\n[1,2]", array.argumentsError.?);
}
test "G1 upstream malformed-string repair is explicitly unresolved" {
    return error.SkipZigTest;
}
