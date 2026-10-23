const std = @import("std");
const ai = @import("ai");
const mock = @import("mocks.zig");
test "cost tiers apply request-wide above strict threshold including cache" {
    var model = mock.model("openai-completions");
    model.cost.tiers = &.{.{ .inputTokensAbove = 100, .input = 2, .output = 4, .cacheRead = 0.2, .cacheWrite = 2.5 }};
    var usage = ai.types.Usage{ .input = 60, .cacheRead = 40, .output = 10 };
    _ = try ai.ai.cost.calculateCost(std.testing.allocator, model, &usage);
    try std.testing.expectApproxEqAbs(@as(f64, 0.00006), usage.cost.input, 1e-12);
    usage.cacheRead = 41;
    _ = try ai.ai.cost.calculateCost(std.testing.allocator, model, &usage);
    try std.testing.expectApproxEqAbs(@as(f64, 0.00012), usage.cost.input, 1e-12);
    usage.cacheWrite = 5;
    usage.cacheWrite1h = 5;
    _ = try ai.ai.cost.calculateCost(std.testing.allocator, model, &usage);
    try std.testing.expectApproxEqAbs(@as(f64, 0.00002), usage.cost.cacheWrite, 1e-12);
}
test "explicit null thinking levels clamp to supported levels" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const a = arena.allocator();
    var model = mock.model("openai-responses");
    model.reasoning = true;
    const parsed = try std.json.parseFromSlice(std.json.Value, a, "{\"off\":null,\"minimal\":null,\"low\":\"low\",\"medium\":null,\"high\":\"high\",\"xhigh\":null,\"max\":\"max\"}", .{});
    defer parsed.deinit();
    model.thinkingLevelMap = parsed.value;
    try std.testing.expectEqual(ai.types.ThinkingLevel.low, ai.ai.thinking.clampThinkingLevel(model, .off));
    try std.testing.expectEqual(ai.types.ThinkingLevel.high, ai.ai.thinking.clampThinkingLevel(model, .xhigh));
    try std.testing.expectEqual(@as(usize, 4), try ai.ai.unicode.utf16Length("A😀é"));
}
test "fixed retry and overflow classifiers retain quota exclusions" {
    var message = ai.types.AssistantMessage{ .api = "fixture", .provider = "fixture", .model = "fixture", .timestamp = 0, .stopReason = .@"error", .errorMessage = "The service is temporarily unavailable" };
    try std.testing.expect(ai.ai.retry.isRetryableAssistantError(message));
    message.errorMessage = "429 insufficient_quota";
    try std.testing.expect(!ai.ai.retry.isRetryableAssistantError(message));
    message.errorMessage = "Input length (265330) exceeds model's maximum context length (262144).";
    try std.testing.expect(ai.ai.overflow.isContextOverflow(message, null));
    message.errorMessage = "Throttling error: Too many tokens, please wait before trying again.";
    try std.testing.expect(!ai.ai.overflow.isContextOverflow(message, null));
    message.stopReason = .length;
    message.usage = .{ .input = 100, .output = 0 };
    try std.testing.expect(ai.ai.overflow.isContextOverflow(message, 100));
}
