const std = @import("std");
const m = @import("../models.zig");
pub fn cloudflareAIGatewayProvider(a: std.mem.Allocator) !m.Provider {
    var provider = try @import("base.zig").create(a, .{ .id = "cloudflare-ai-gateway", .name = "Cloudflare AI Gateway", .baseUrl = null, .authName = "", .env = &.{}, .apis = &.{ "anthropic-messages", "openai-completions", "openai-responses" } }, @embedFile("../../data/cloudflare-ai-gateway.json"));
    provider.auth.apiKey = @import("cloudflare_auth.zig").cloudflareAIGatewayAuth();
    return provider;
}
