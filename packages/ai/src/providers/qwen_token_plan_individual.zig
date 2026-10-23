const std = @import("std");
const m = @import("../models.zig");
pub fn qwenTokenPlanIndividualProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "qwen-token-plan-individual", .name = "Qwen Token Plan Individual", .baseUrl = "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1", .authName = "Qwen Token Plan Individual API key", .env = &.{"QWEN_TOKEN_PLAN_API_KEY"}, .apis = &.{"openai-completions"} }, @embedFile("../../data/qwen-token-plan-individual.json"));
}
