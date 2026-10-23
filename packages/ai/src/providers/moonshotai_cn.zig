const std = @import("std");
const m = @import("../models.zig");
pub fn moonshotaiCnProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "moonshotai-cn", .name = "Moonshot AI CN", .baseUrl = "https://api.moonshot.cn/v1", .authName = "Moonshot AI API key", .env = &.{"MOONSHOT_API_KEY"}, .apis = &.{"openai-completions"} }, @embedFile("../../data/moonshotai-cn.json"));
}
