const std = @import("std");
const m = @import("../models.zig");
pub fn kimiCodingProvider(a: std.mem.Allocator) !m.Provider {
    var provider = try @import("base.zig").create(a, .{ .id = "kimi-coding", .name = "Kimi For Coding", .baseUrl = "https://api.kimi.com/coding/v1", .authName = "Kimi API key", .env = &.{"KIMI_API_KEY"}, .apis = &.{"openai-completions"} }, @embedFile("../../data/kimi-coding.json"));
    provider.auth.oauth = @import("../oauth.zig").kimiCodingOAuth;
    return provider;
}
