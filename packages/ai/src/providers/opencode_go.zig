const std = @import("std");
const m = @import("../models.zig");
pub fn opencodeGoProvider(a: std.mem.Allocator) !m.Provider {
    const provider = try @import("base.zig").create(a, .{ .id = "opencode-go", .name = "OpenCode Go", .baseUrl = null, .authName = "OpenCode API key", .env = &.{"OPENCODE_API_KEY"}, .apis = &.{ "anthropic-messages", "google-generative-ai", "openai-completions", "openai-responses" } }, @embedFile("../../data/opencode-go.json"));
    for (@constCast(provider.apis)) |*api| api.* = try @import("opencode_headers.zig").withOpenCodeSessionHeader(a, api.*);
    return provider;
}
