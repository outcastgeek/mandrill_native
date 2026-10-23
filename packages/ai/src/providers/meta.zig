const std = @import("std");
const m = @import("../models.zig");
pub fn metaProvider(a: std.mem.Allocator) !m.Provider {
    var provider = try @import("base.zig").create(a, .{ .id = "meta", .name = "Meta", .baseUrl = "https://api.meta.ai/v1", .authName = "Meta Model API key", .env = &.{"META_API_KEY"}, .apis = &.{"openai-responses"} }, @embedFile("../../data/meta.json"));
    provider.auth.oauth = @import("../oauth.zig").metaOAuth;
    return provider;
}
