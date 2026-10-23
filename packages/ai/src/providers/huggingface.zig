const std = @import("std");
const m = @import("../models.zig");
pub fn huggingfaceProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "huggingface", .name = "Hugging Face", .baseUrl = "https://router.huggingface.co/v1", .authName = "Hugging Face token", .env = &.{"HF_TOKEN"}, .apis = &.{"openai-completions"} }, @embedFile("../../data/huggingface.json"));
}
