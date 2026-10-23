const std = @import("std");
const t = @import("types.zig");
const auth = @import("auth/types.zig");
const Entry = struct { provider: []const u8, variable: []const u8 };
const entries = [_]Entry{
    .{ .provider = "ant-ling", .variable = "ANT_LING_API_KEY" },                           .{ .provider = "qwen-token-plan", .variable = "QWEN_TOKEN_PLAN_API_KEY" },             .{ .provider = "qwen-token-plan-cn", .variable = "QWEN_TOKEN_PLAN_CN_API_KEY" }, .{ .provider = "qwen-token-plan-individual", .variable = "QWEN_TOKEN_PLAN_API_KEY" },
    .{ .provider = "openai", .variable = "OPENAI_API_KEY" },                               .{ .provider = "azure", .variable = "AZURE_OPENAI_API_KEY" },                          .{ .provider = "nvidia", .variable = "NVIDIA_API_KEY" },                         .{ .provider = "deepseek", .variable = "DEEPSEEK_API_KEY" },
    .{ .provider = "google", .variable = "GEMINI_API_KEY" },                               .{ .provider = "google-vertex", .variable = "GOOGLE_CLOUD_API_KEY" },                  .{ .provider = "groq", .variable = "GROQ_API_KEY" },                             .{ .provider = "cerebras", .variable = "CEREBRAS_API_KEY" },
    .{ .provider = "xai", .variable = "XAI_API_KEY" },                                     .{ .provider = "typesafe", .variable = "TYPESAFE_API_KEY" },                           .{ .provider = "radius", .variable = "RADIUS_API_KEY" },                         .{ .provider = "openrouter", .variable = "OPENROUTER_API_KEY" },
    .{ .provider = "vercel-ai-gateway", .variable = "AI_GATEWAY_API_KEY" },                .{ .provider = "zai", .variable = "ZAI_API_KEY" },                                     .{ .provider = "zai-coding-cn", .variable = "ZAI_CODING_CN_API_KEY" },           .{ .provider = "mistral", .variable = "MISTRAL_API_KEY" },
    .{ .provider = "minimax", .variable = "MINIMAX_API_KEY" },                             .{ .provider = "minimax-cn", .variable = "MINIMAX_CN_API_KEY" },                       .{ .provider = "moonshotai", .variable = "MOONSHOT_API_KEY" },                   .{ .provider = "moonshotai-cn", .variable = "MOONSHOT_API_KEY" },
    .{ .provider = "huggingface", .variable = "HF_TOKEN" },                                .{ .provider = "fireworks", .variable = "FIREWORKS_API_KEY" },                         .{ .provider = "together", .variable = "TOGETHER_API_KEY" },                     .{ .provider = "baseten", .variable = "BASETEN_API_KEY" },
    .{ .provider = "opencode", .variable = "OPENCODE_API_KEY" },                           .{ .provider = "opencode-go", .variable = "OPENCODE_API_KEY" },                        .{ .provider = "kimi-coding", .variable = "KIMI_API_KEY" },                      .{ .provider = "meta", .variable = "META_API_KEY" },
    .{ .provider = "cloudflare-workers-ai", .variable = "CLOUDFLARE_API_KEY" },            .{ .provider = "cloudflare-ai-gateway", .variable = "CLOUDFLARE_API_KEY" },            .{ .provider = "xiaomi", .variable = "XIAOMI_API_KEY" },                         .{ .provider = "xiaomi-token-plan-cn", .variable = "XIAOMI_TOKEN_PLAN_CN_API_KEY" },
    .{ .provider = "xiaomi-token-plan-ams", .variable = "XIAOMI_TOKEN_PLAN_AMS_API_KEY" }, .{ .provider = "xiaomi-token-plan-sgp", .variable = "XIAOMI_TOKEN_PLAN_SGP_API_KEY" }, .{ .provider = "github-copilot", .variable = "COPILOT_GITHUB_TOKEN" },
};
pub fn env(a: std.mem.Allocator, io: std.Io, context: auth.AuthContext, overlay: t.ProviderEnv, name: []const u8) !?[]const u8 {
    return @import("utils/provider_env.zig").getProviderEnvValue(a, io, context, name, overlay);
}
// JavaScript environment truthiness excludes the empty string, while retaining
// whitespace. Optional presence alone does not express that distinction.
fn nonempty(value: ?[]const u8) bool {
    return if (value) |text| text.len > 0 else false;
}
fn hasVertexAdcCredentials(a: std.mem.Allocator, io: std.Io, context: auth.AuthContext, overlay: t.ProviderEnv) !bool {
    // Availability discovery inspects existence only. Credential bytes still
    // cross the explicit in-memory input boundary, never this filesystem hook.
    if (overlay.get("GOOGLE_APPLICATION_CREDENTIALS")) |path| if (path.len > 0)
        return context.fileExistsFn(context.context, io, path);
    if (context.vertexAdcExistsCache) |cache| switch (cache.load(.acquire)) {
        1 => return false,
        2 => return true,
        else => {},
    };
    const found = try discoverVertexAdcCredentials(a, io, context, overlay);
    if (context.vertexAdcExistsCache) |cache| {
        const previous = cache.cmpxchgStrong(0, if (found) 2 else 1, .acq_rel, .acquire);
        if (previous) |state| return state == 2;
    }
    return found;
}
fn discoverVertexAdcCredentials(a: std.mem.Allocator, io: std.Io, context: auth.AuthContext, overlay: t.ProviderEnv) !bool {
    if (try env(a, io, context, overlay, "GOOGLE_APPLICATION_CREDENTIALS")) |path|
        return context.fileExistsFn(context.context, io, path);
    const home = if (context.homeDirFn) |get| try get(context.context, a, io) else (try context.env(a, io, if (@import("builtin").os.tag == .windows) "USERPROFILE" else "HOME"));
    const base = home orelse return false;
    const path = try std.fs.path.join(a, &.{ base, ".config", "gcloud", "application_default_credentials.json" });
    return context.fileExistsFn(context.context, io, path);
}
pub fn findEnvKeys(a: std.mem.Allocator, io: std.Io, context: auth.AuthContext, provider: []const u8, overlay: t.ProviderEnv) !?[]const []const u8 {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    var found: std.ArrayList([]const u8) = .empty;
    errdefer found.deinit(a);
    if (std.mem.eql(u8, provider, "anthropic")) {
        inline for (.{ "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_OAUTH_TOKEN", "ANTHROPIC_API_KEY" }) |name| if (try env(scratch, io, context, overlay, name)) |value| if (value.len > 0) {
            try found.append(a, name);
        };
    } else for (entries) |entry| if (std.mem.eql(u8, entry.provider, provider)) {
        if (try env(scratch, io, context, overlay, entry.variable)) |value| if (value.len > 0) {
            try found.append(a, entry.variable);
        };
    };
    return if (found.items.len > 0) try found.toOwnedSlice(a) else null;
}
pub fn getEnvApiKey(a: std.mem.Allocator, io: std.Io, context: auth.AuthContext, provider: []const u8, overlay: t.ProviderEnv) !?[]const u8 {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    if (try findEnvKeys(scratch, io, context, provider, overlay)) |keys| for (keys) |name| {
        if (std.mem.eql(u8, provider, "anthropic") and std.mem.eql(u8, name, "ANTHROPIC_AUTH_TOKEN")) continue;
        return env(a, io, context, overlay, name);
    };
    if (std.mem.eql(u8, provider, "amazon-bedrock")) {
        if (nonempty(try env(scratch, io, context, overlay, "AWS_PROFILE"))) return try a.dupe(u8, "<authenticated>");
        if (nonempty(try env(scratch, io, context, overlay, "AWS_ACCESS_KEY_ID")) and nonempty(try env(scratch, io, context, overlay, "AWS_SECRET_ACCESS_KEY"))) return try a.dupe(u8, "<authenticated>");
        inline for (.{ "AWS_BEARER_TOKEN_BEDROCK", "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI", "AWS_CONTAINER_CREDENTIALS_FULL_URI", "AWS_WEB_IDENTITY_TOKEN_FILE" }) |name| if (try env(scratch, io, context, overlay, name)) |value| if (value.len > 0) return try a.dupe(u8, "<authenticated>");
    }
    if (std.mem.eql(u8, provider, "google-vertex")) {
        const has_project = nonempty(try env(scratch, io, context, overlay, "GOOGLE_CLOUD_PROJECT")) or nonempty(try env(scratch, io, context, overlay, "GCLOUD_PROJECT"));
        const location = try env(scratch, io, context, overlay, "GOOGLE_CLOUD_LOCATION");
        if (try hasVertexAdcCredentials(scratch, io, context, overlay) and has_project and nonempty(location)) return try a.dupe(u8, "<authenticated>");
    }
    return null;
}

// Public environment names; the legacy file name does not enable file retrieval.
pub const ANTHROPIC_AUTH_TOKEN_ENV = "ANTHROPIC_AUTH_TOKEN";
pub const ANTHROPIC_OAUTH_TOKEN_ENV = "ANTHROPIC_OAUTH_TOKEN";
pub const ANTHROPIC_API_KEY_ENV = "ANTHROPIC_API_KEY";
pub const ANTHROPIC_FEDERATION_RULE_ID_ENV = "ANTHROPIC_FEDERATION_RULE_ID";
pub const ANTHROPIC_ORGANIZATION_ID_ENV = "ANTHROPIC_ORGANIZATION_ID";
pub const ANTHROPIC_SERVICE_ACCOUNT_ID_ENV = "ANTHROPIC_SERVICE_ACCOUNT_ID";
pub const ANTHROPIC_IDENTITY_TOKEN_FILE_ENV = "ANTHROPIC_IDENTITY_TOKEN_FILE";
pub const ANTHROPIC_WORKSPACE_ID_ENV = "ANTHROPIC_WORKSPACE_ID";
pub const ANTHROPIC_IDENTITY_TOKEN_ENV = "ANTHROPIC_IDENTITY_TOKEN";
