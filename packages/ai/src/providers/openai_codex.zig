const std = @import("std");
const m = @import("../models.zig");
pub fn openaiCodexProvider(a: std.mem.Allocator) !m.Provider {
    var provider = try @import("base.zig").create(a, .{ .id = "openai-codex", .name = "OpenAI Codex (legacy)", .baseUrl = "https://chatgpt.com/backend-api", .authName = "", .env = &.{}, .apis = &.{"openai-codex-responses"} }, @embedFile("../../data/openai-codex.json"));
    provider.auth.apiKey = null;
    provider.auth.oauth = @import("../oauth.zig").openaiCodexOAuth;
    return provider;
}
