const std = @import("std");
const m = @import("../models.zig");
const t = @import("../types.zig");
const auth = @import("../auth/types.zig");
pub fn openaiProvider(a: std.mem.Allocator) !m.Provider {
    var provider = try @import("base.zig").create(a, .{ .id = "openai", .name = "OpenAI", .baseUrl = "https://api.openai.com/v1", .authName = "OpenAI API key", .env = &.{"OPENAI_API_KEY"}, .apis = &.{ "openai-responses", "openai-decisions" } }, @embedFile("../../data/openai.json"));
    provider.auth.oauth = @import("../oauth.zig").openaiChatGPTOAuth;
    provider.filterAllModels = filter;
    return provider;
}
fn filter(_: ?*anyopaque, a: std.mem.Allocator, models: []const t.Model, credential: ?auth.Credential) ![]const t.Model {
    if (credential == null or credential.? != .oauth) return models;
    var out: std.ArrayList(t.Model) = .empty;
    for (models) |model| if (model.modelType() != .classifier) {
        try out.append(a, model);
    };
    return out.toOwnedSlice(a);
}
