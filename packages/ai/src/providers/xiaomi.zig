const std = @import("std");
const m = @import("../models.zig");
pub fn xiaomiProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "xiaomi", .name = "Xiaomi", .baseUrl = "https://api.xiaomimimo.com/v1", .authName = "Xiaomi API key", .env = &.{"XIAOMI_API_KEY"}, .apis = &.{"openai-completions"} }, @embedFile("../../data/xiaomi.json"));
}
