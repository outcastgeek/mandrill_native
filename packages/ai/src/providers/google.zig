const std = @import("std");
const m = @import("../models.zig");
pub fn googleProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "google", .name = "Google", .baseUrl = "https://generativelanguage.googleapis.com/v1beta", .authName = "Gemini API key", .env = &.{"GEMINI_API_KEY"}, .apis = &.{"google-generative-ai"} }, @embedFile("../../data/google.json"));
}
