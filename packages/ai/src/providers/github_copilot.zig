const std = @import("std");
const m = @import("../models.zig");
const t = @import("../types.zig");
const auth = @import("../auth/types.zig");
pub fn githubCopilotProvider(a: std.mem.Allocator) !m.Provider {
    var provider = try @import("base.zig").create(a, .{ .id = "github-copilot", .name = "GitHub Copilot", .baseUrl = "https://api.individual.githubcopilot.com", .authName = "GitHub Copilot token", .env = &.{"COPILOT_GITHUB_TOKEN"}, .apis = &.{ "anthropic-messages", "openai-completions", "openai-responses" } }, @embedFile("../../data/github-copilot.json"));
    provider.auth.oauth = @import("../oauth.zig").githubCopilotOAuth;
    provider.filterModels = filter;
    return provider;
}
fn filter(_: ?*anyopaque, a: std.mem.Allocator, models: []const t.Model, credential: ?auth.Credential) ![]const t.Model {
    if (credential == null or credential.? != .oauth) return models;
    const metadata = credential.?.oauth.metadata orelse return models;
    const ids = @import("../json/value.zig").get(metadata, "availableModelIds") orelse return models;
    if (ids != .array) return models;
    for (ids.array.items) |id| if (id != .string) return models;
    var out: std.ArrayList(t.Model) = .empty;
    for (models) |model| {
        for (ids.array.items) |id| if (std.mem.eql(u8, id.string, model.id)) {
            try out.append(a, model);
            break;
        };
    }
    return out.toOwnedSlice(a);
}
