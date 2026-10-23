const std = @import("std");
const m = @import("../models.zig");
pub fn xaiProvider(a: std.mem.Allocator) !m.Provider {
    var provider = try @import("base.zig").create(a, .{ .id = "xai", .name = "xAI", .baseUrl = "https://api.x.ai/v1", .authName = "xAI API key", .env = &.{"XAI_API_KEY"}, .apis = &.{"openai-responses"} }, @embedFile("../../data/xai.json"));
    provider.auth.oauth = @import("../oauth.zig").xaiOAuth;
    return provider;
}
