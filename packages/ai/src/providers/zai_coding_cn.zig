const std = @import("std");
const m = @import("../models.zig");
pub fn zaiCodingCnProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "zai-coding-cn", .name = "Z.AI Coding CN", .baseUrl = "https://open.bigmodel.cn/api/coding/paas/v4", .authName = "Z.AI Coding CN API key", .env = &.{"ZAI_CODING_CN_API_KEY"}, .apis = &.{"openai-completions"} }, @embedFile("../../data/zai-coding-cn.json"));
}
