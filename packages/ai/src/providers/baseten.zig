const std = @import("std");
const m = @import("../models.zig");
pub fn basetenProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "baseten", .name = "Baseten", .baseUrl = "https://inference.baseten.co/v1", .authName = "Baseten API key", .env = &.{"BASETEN_API_KEY"}, .apis = &.{"openai-completions"} }, @embedFile("../../data/baseten.json"));
}
