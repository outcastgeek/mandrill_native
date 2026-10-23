const std = @import("std");
const m = @import("../models.zig");
pub fn vercelAIGatewayProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "vercel-ai-gateway", .name = "Vercel AI Gateway", .baseUrl = "https://ai-gateway.vercel.sh", .authName = "Vercel AI Gateway API key", .env = &.{"AI_GATEWAY_API_KEY"}, .apis = &.{ "anthropic-messages", "typesafe-system-one" } }, @embedFile("../../data/vercel-ai-gateway.json"));
}
