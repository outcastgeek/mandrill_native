const std = @import("std");
const m = @import("../models.zig");
pub fn cerebrasProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "cerebras", .name = "Cerebras", .baseUrl = "https://api.cerebras.ai/v1", .authName = "Cerebras API key", .env = &.{"CEREBRAS_API_KEY"}, .apis = &.{"openai-completions"} }, @embedFile("../../data/cerebras.json"));
}
