const std = @import("std");
const m = @import("../models.zig");
pub fn fireworksProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "fireworks", .name = "Fireworks", .baseUrl = "https://api.fireworks.ai/inference", .authName = "Fireworks API key", .env = &.{"FIREWORKS_API_KEY"}, .apis = &.{ "anthropic-messages", "openai-completions" } }, @embedFile("../../data/fireworks.json"));
}
