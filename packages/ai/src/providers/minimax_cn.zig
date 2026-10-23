const std = @import("std");
const m = @import("../models.zig");
pub fn minimaxCnProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "minimax-cn", .name = "MiniMax CN", .baseUrl = "https://api.minimaxi.com/anthropic", .authName = "MiniMax CN API key", .env = &.{"MINIMAX_CN_API_KEY"}, .apis = &.{"anthropic-messages"} }, @embedFile("../../data/minimax-cn.json"));
}
