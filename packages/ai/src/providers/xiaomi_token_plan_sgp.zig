const std = @import("std");
const m = @import("../models.zig");
pub fn xiaomiTokenPlanSgpProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "xiaomi-token-plan-sgp", .name = "Xiaomi Token Plan SGP", .baseUrl = "https://token-plan-sgp.xiaomimimo.com/v1", .authName = "Xiaomi Token Plan SGP API key", .env = &.{"XIAOMI_TOKEN_PLAN_SGP_API_KEY"}, .apis = &.{"openai-completions"} }, @embedFile("../../data/xiaomi-token-plan-sgp.json"));
}
