const std = @import("std");
const m = @import("../models.zig");
pub fn togetherProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "together", .name = "Together", .baseUrl = "https://api.together.ai/v1", .authName = "Together API key", .env = &.{"TOGETHER_API_KEY"}, .apis = &.{"openai-completions"} }, @embedFile("../../data/together.json"));
}
