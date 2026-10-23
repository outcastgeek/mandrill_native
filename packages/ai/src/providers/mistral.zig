const std = @import("std");
const m = @import("../models.zig");
pub fn mistralProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "mistral", .name = "Mistral", .baseUrl = "https://api.mistral.ai", .authName = "Mistral API key", .env = &.{"MISTRAL_API_KEY"}, .apis = &.{"mistral-conversations"} }, @embedFile("../../data/mistral.json"));
}
