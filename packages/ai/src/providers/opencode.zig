const std = @import("std");
const m = @import("../models.zig");
pub fn opencodeProvider(a: std.mem.Allocator) !m.Provider {
    const provider = try @import("base.zig").create(a, .{ .id = "opencode", .name = "OpenCode Zen", .baseUrl = null, .authName = "OpenCode API key", .env = &.{"OPENCODE_API_KEY"}, .apis = &.{ "anthropic-messages", "google-generative-ai", "openai-completions", "openai-responses", "typesafe-system-one" } }, @embedFile("../../data/opencode.json"));
    for (@constCast(provider.apis)) |*api| if (api.stream != null) {
        api.* = try @import("opencode_headers.zig").withOpenCodeSessionHeader(a, api.*);
    };
    return provider;
}
