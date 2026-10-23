const std = @import("std");
const m = @import("../models.zig");
pub fn xiaomiTokenPlanAmsProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "xiaomi-token-plan-ams", .name = "Xiaomi Token Plan AMS", .baseUrl = "https://token-plan-ams.xiaomimimo.com/v1", .authName = "Xiaomi Token Plan AMS API key", .env = &.{"XIAOMI_TOKEN_PLAN_AMS_API_KEY"}, .apis = &.{"openai-completions"} }, @embedFile("../../data/xiaomi-token-plan-ams.json"));
}
