const std = @import("std");
const m = @import("../models.zig");
pub fn groqProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "groq", .name = "Groq", .baseUrl = "https://api.groq.com/openai/v1", .authName = "Groq API key", .env = &.{"GROQ_API_KEY"}, .apis = &.{"openai-completions"} }, @embedFile("../../data/groq.json"));
}
