const std = @import("std");
const m = @import("../models.zig");
pub fn nvidiaProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "nvidia", .name = "NVIDIA", .baseUrl = "https://integrate.api.nvidia.com/v1", .authName = "NVIDIA API key", .env = &.{"NVIDIA_API_KEY"}, .apis = &.{"openai-completions"} }, @embedFile("../../data/nvidia.json"));
}
