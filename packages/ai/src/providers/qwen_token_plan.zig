const std = @import("std");
const m = @import("../models.zig");
pub fn qwenTokenPlanProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "qwen-token-plan", .name = "Qwen Token Plan", .baseUrl = "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1", .authName = "Qwen Token Plan API key", .env = &.{"QWEN_TOKEN_PLAN_API_KEY"}, .apis = &.{"openai-completions"} }, @embedFile("../../data/qwen-token-plan.json"));
}
