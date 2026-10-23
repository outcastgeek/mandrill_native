const std = @import("std");
const m = @import("../models.zig");
pub fn qwenTokenPlanCnProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "qwen-token-plan-cn", .name = "Qwen Token Plan CN", .baseUrl = "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1", .authName = "Qwen Token Plan CN API key", .env = &.{"QWEN_TOKEN_PLAN_CN_API_KEY"}, .apis = &.{"openai-completions"} }, @embedFile("../../data/qwen-token-plan-cn.json"));
}
