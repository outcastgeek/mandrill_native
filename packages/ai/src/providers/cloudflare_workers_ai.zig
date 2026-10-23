const std = @import("std");
const m = @import("../models.zig");
pub fn cloudflareWorkersAIProvider(a: std.mem.Allocator) !m.Provider {
    var provider = try @import("base.zig").create(a, .{ .id = "cloudflare-workers-ai", .name = "Cloudflare Workers AI", .baseUrl = null, .authName = "", .env = &.{}, .apis = &.{ "openai-completions", "cloudflare-workers-ai-system-one" } }, @embedFile("../../data/cloudflare-workers-ai.json"));
    provider.auth.apiKey = @import("cloudflare_auth.zig").cloudflareWorkersAIAuth();
    return provider;
}
