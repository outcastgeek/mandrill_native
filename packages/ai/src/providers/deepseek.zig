const std = @import("std");
const m = @import("../models.zig");
pub fn deepseekProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "deepseek", .name = "DeepSeek", .baseUrl = "https://api.deepseek.com", .authName = "DeepSeek API key", .env = &.{"DEEPSEEK_API_KEY"}, .apis = &.{"openai-completions"} }, @embedFile("../../data/deepseek.json"));
}
