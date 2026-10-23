const std = @import("std");
const m = @import("../models.zig");
pub fn xiaomiTokenPlanCnProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "xiaomi-token-plan-cn", .name = "Xiaomi Token Plan CN", .baseUrl = "https://token-plan-cn.xiaomimimo.com/v1", .authName = "Xiaomi Token Plan CN API key", .env = &.{"XIAOMI_TOKEN_PLAN_CN_API_KEY"}, .apis = &.{"openai-completions"} }, @embedFile("../../data/xiaomi-token-plan-cn.json"));
}
