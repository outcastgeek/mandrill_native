const std = @import("std");
const m = @import("../models.zig");
pub fn zaiProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "zai", .name = "Z.AI", .baseUrl = "https://api.z.ai/api/coding/paas/v4", .authName = "Z.AI API key", .env = &.{"ZAI_API_KEY"}, .apis = &.{"openai-completions"} }, @embedFile("../../data/zai.json"));
}
