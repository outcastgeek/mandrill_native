const std = @import("std");
const m = @import("../models.zig");
pub fn moonshotaiProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "moonshotai", .name = "Moonshot AI", .baseUrl = "https://api.moonshot.ai/v1", .authName = "Moonshot AI API key", .env = &.{"MOONSHOT_API_KEY"}, .apis = &.{"openai-completions"} }, @embedFile("../../data/moonshotai.json"));
}
