const std = @import("std");
const m = @import("../models.zig");
pub fn antLingProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "ant-ling", .name = "Ant Ling", .baseUrl = "https://api.ant-ling.com/v1", .authName = "Ant Ling API key", .env = &.{"ANT_LING_API_KEY"}, .apis = &.{"openai-completions"} }, @embedFile("../../data/ant-ling.json"));
}
