const std = @import("std");
const m = @import("../models.zig");
pub fn anthropicProvider(a: std.mem.Allocator) !m.Provider {
    var provider = try @import("base.zig").create(a, .{ .id = "anthropic", .name = "Anthropic", .baseUrl = "https://api.anthropic.com", .authName = "", .env = &.{}, .apis = &.{"anthropic-messages"} }, @embedFile("../../data/anthropic.json"));
    provider.auth.apiKey = @import("provider_auth.zig").anthropic();
    provider.auth.oauth = @import("../oauth.zig").anthropicOAuth;
    return provider;
}
