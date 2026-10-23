const std = @import("std");
const m = @import("../models.zig");
pub fn minimaxProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "minimax", .name = "MiniMax", .baseUrl = "https://api.minimax.io/anthropic", .authName = "MiniMax API key", .env = &.{"MINIMAX_API_KEY"}, .apis = &.{"anthropic-messages"} }, @embedFile("../../data/minimax.json"));
}
