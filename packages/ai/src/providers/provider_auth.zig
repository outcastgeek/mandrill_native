const std = @import("std");
const auth = @import("../auth/types.zig");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const helpers = @import("../auth/helpers.zig");
fn prompt(a: std.mem.Allocator, io: std.Io, interaction: auth.Interaction, kind: []const u8, message: []const u8) ![]const u8 {
    try io.checkCancel();
    const result = try interaction.prompt(interaction.context, a, io, try j.from(a, .{ .type = kind, .message = message }));
    try io.checkCancel();
    return result;
}
fn environment(a: std.mem.Allocator, io: std.Io, ctx: auth.AuthContext, key: []const u8) !?[]const u8 {
    try io.checkCancel();
    const value = try ctx.env(a, io, key);
    errdefer if (value) |text| a.free(text);
    try io.checkCancel();
    return value;
}
fn scoped(a: std.mem.Allocator, io: std.Io, ctx: auth.AuthContext, credential: ?auth.ApiKeyCredential, key: []const u8) !?[]const u8 {
    if (credential) |value| if (value.env) |env| if (j.field(env, key)) |text| return text;
    return environment(a, io, ctx, key);
}
fn cloudflareValue(a: std.mem.Allocator, io: std.Io, ctx: auth.AuthContext, credential: ?auth.ApiKeyCredential, key: []const u8) !?[]const u8 {
    if (credential) |value| {
        if (std.mem.eql(u8, key, "CLOUDFLARE_API_KEY")) {
            if (j.fieldPresent(value, "key")) return value.key;
        } else if (value.env) |env| {
            if (j.get(env, key)) |present| return j.text(present);
        }
    }
    return environment(a, io, ctx, key);
}
fn stored(a: std.mem.Allocator, credential: ?auth.ApiKeyCredential) !?auth.AuthResult {
    if (credential) |value| if (value.key) |key| if (key.len > 0) return .{ .auth = .{ .apiKey = key }, .source = "stored credential", .env = try helpers.credentialEnv(a, value.env) };
    return null;
}
pub fn cloudflare(comptime gateway: bool) auth.ApiKeyAuth {
    return .{ .name = "Cloudflare API key", .login = struct {
        fn login(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, interaction: auth.Interaction) !auth.ApiKeyCredential {
            const key = try prompt(a, io, interaction, "secret", "Enter Cloudflare API key");
            const account = try prompt(a, io, interaction, "text", "Enter Cloudflare account ID");
            var env = j.map(a);
            try j.put(a, &env, "CLOUDFLARE_ACCOUNT_ID", j.str(account));
            if (gateway) try j.put(a, &env, "CLOUDFLARE_GATEWAY_ID", j.str(try prompt(a, io, interaction, "text", "Enter Cloudflare AI Gateway ID")));
            return .{ .key = key, .env = env };
        }
    }.login, .resolve = struct {
        fn resolve(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, ctx: auth.AuthContext, credential: ?auth.ApiKeyCredential) !?auth.AuthResult {
            const key = try cloudflareValue(a, io, ctx, credential, "CLOUDFLARE_API_KEY");
            const account = try cloudflareValue(a, io, ctx, credential, "CLOUDFLARE_ACCOUNT_ID");
            const gateway_id = if (gateway) try cloudflareValue(a, io, ctx, credential, "CLOUDFLARE_GATEWAY_ID") else null;
            if (key == null or key.?.len == 0 or account == null or account.?.len == 0 or (gateway and (gateway_id == null or gateway_id.?.len == 0))) return null;
            var env = t.ProviderEnv{};
            try env.put(a, "CLOUDFLARE_ACCOUNT_ID", account.?);
            if (gateway_id) |id| try env.put(a, "CLOUDFLARE_GATEWAY_ID", id);
            return .{ .auth = if (gateway) .{ .headers = try a.dupe(t.ProviderHeader, &.{ .{ .name = "cf-aig-authorization", .value = try std.fmt.allocPrint(a, "Bearer {s}", .{key.?}) }, .{ .name = "Authorization", .value = null }, .{ .name = "x-api-key", .value = null } }) } else .{ .apiKey = key.? }, .env = env, .source = if (credential != null) "stored credential" else "CLOUDFLARE_API_KEY" };
        }
    }.resolve };
}
fn anthropicResolve(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, ctx: auth.AuthContext, credential: ?auth.ApiKeyCredential) !?auth.AuthResult {
    try io.checkCancel();
    if (try stored(a, credential)) |value| return value;
    if (try environment(a, io, ctx, "ANTHROPIC_AUTH_TOKEN")) |value| if (value.len > 0) return .{ .auth = .{ .headers = try a.dupe(t.ProviderHeader, &.{.{ .name = "Authorization", .value = try std.fmt.allocPrint(a, "Bearer {s}", .{value}) }}) }, .source = "ANTHROPIC_AUTH_TOKEN" };
    inline for (.{ "ANTHROPIC_OAUTH_TOKEN", "ANTHROPIC_API_KEY" }) |key| if (try environment(a, io, ctx, key)) |value| if (value.len > 0) return .{ .auth = .{ .apiKey = value }, .source = key };
    var env = t.ProviderEnv{};
    inline for (.{ "ANTHROPIC_FEDERATION_RULE_ID", "ANTHROPIC_ORGANIZATION_ID" }) |key| {
        const value = (try environment(a, io, ctx, key)) orelse return null;
        if (value.len == 0) return null;
        try env.put(a, key, value);
    }
    const token_file = try environment(a, io, ctx, "ANTHROPIC_IDENTITY_TOKEN_FILE");
    if (token_file) |path| if (path.len > 0) {
        try env.put(a, "ANTHROPIC_IDENTITY_TOKEN_FILE", path);
    };
    if ((token_file == null or token_file.?.len == 0) and ctx.inputs.anthropicIdentityToken == null) {
        const token = (try environment(a, io, ctx, "ANTHROPIC_IDENTITY_TOKEN")) orelse return null;
        if (token.len == 0) return null;
    }
    inline for (.{ "ANTHROPIC_SERVICE_ACCOUNT_ID", "ANTHROPIC_WORKSPACE_ID" }) |key| if (try environment(a, io, ctx, key)) |value| if (value.len > 0) {
        try env.put(a, key, value);
    };
    return .{ .auth = .{}, .env = env, .source = "workload identity federation" };
}
pub fn anthropic() auth.ApiKeyAuth {
    var result = helpers.envApiKeyAuth("Anthropic API key", &.{});
    result.resolve = anthropicResolve;
    return result;
}
fn bedrockResolve(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, ctx: auth.AuthContext, credential: ?auth.ApiKeyCredential) !?auth.AuthResult {
    if (try stored(a, credential)) |value| return value;
    if (try environment(a, io, ctx, "AWS_BEARER_TOKEN_BEDROCK")) |value| if (value.len > 0) return .{ .auth = .{}, .source = "AWS_BEARER_TOKEN_BEDROCK" };
    if (try scoped(a, io, ctx, credential, "AWS_PROFILE")) |profile| if (profile.len > 0) return .{ .auth = .{}, .env = try helpers.credentialEnv(a, if (credential) |v| v.env else null), .source = if (credential != null and j.field(credential.?.env orelse .null, "AWS_PROFILE") != null) "stored credential" else "AWS_PROFILE" };
    if (try environment(a, io, ctx, "AWS_ACCESS_KEY_ID")) |key| if (key.len > 0) if (try environment(a, io, ctx, "AWS_SECRET_ACCESS_KEY")) |secret| if (secret.len > 0) return .{ .auth = .{}, .source = "AWS access keys" };
    inline for (.{ "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI", "AWS_CONTAINER_CREDENTIALS_FULL_URI", "AWS_WEB_IDENTITY_TOKEN_FILE" }) |name| if (try environment(a, io, ctx, name)) |value| if (value.len > 0) return .{ .auth = .{}, .source = if (std.mem.eql(u8, name, "AWS_WEB_IDENTITY_TOKEN_FILE")) "web identity token" else "ECS task role" };
    if (ctx.inputs.awsWebIdentityToken != null or ctx.inputs.awsCredentials != null or ctx.inputs.awsConfig != null) return .{ .auth = .{}, .source = "in-memory AWS credentials" };
    return null;
}
fn bedrockLogin(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, interaction: auth.Interaction) !auth.ApiKeyCredential {
    try io.checkCancel();
    const method = try interaction.prompt(interaction.context, a, io, try j.from(a, .{ .type = "select", .message = "Select Amazon Bedrock authentication method:", .options = .{ .{ .id = "bearer-token", .label = "Bearer token" }, .{ .id = "aws-profile", .label = "AWS profile" }, .{ .id = "credential-chain", .label = "Existing AWS credential chain" } } }));
    try io.checkCancel();
    if (std.mem.eql(u8, method, "bearer-token")) return .{ .key = try prompt(a, io, interaction, "secret", "Enter Amazon Bedrock bearer token") };
    try interaction.notify(interaction.context, io, try j.from(a, .{ .type = "info", .message = "Amazon Bedrock supports AWS profiles, IAM credentials, and role-based credentials.", .links = .{.{ .label = "AWS credential provider chain", .url = "https://docs.aws.amazon.com/sdkref/latest/guide/standardized-credentials.html" }} }));
    if (std.mem.eql(u8, method, "aws-profile")) return .{ .env = try j.from(a, .{ .AWS_PROFILE = try prompt(a, io, interaction, "text", "Enter AWS profile name") }) };
    if (!std.mem.eql(u8, method, "credential-chain")) {
        if (interaction.errorMessage) |slot| slot.* = try std.fmt.allocPrint(a, "Unknown Amazon Bedrock auth method: {s}", .{method});
        return error.UnknownAuthenticationMethod;
    }
    _ = try prompt(a, io, interaction, "text", "Configure AWS credentials, then press Enter to continue");
    return .{};
}
pub fn bedrock() auth.ApiKeyAuth {
    return .{ .name = "AWS credentials or bearer token", .login = bedrockLogin, .resolve = bedrockResolve };
}
fn vertexResolve(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, ctx: auth.AuthContext, credential: ?auth.ApiKeyCredential) !?auth.AuthResult {
    const key = (if (credential) |value| value.key else null) orelse (try environment(a, io, ctx, "GOOGLE_CLOUD_API_KEY"));
    if (key) |value| if (value.len > 0) return .{ .auth = .{ .apiKey = value }, .source = if (credential != null and credential.?.key != null and credential.?.key.?.len > 0) "stored credential" else "GOOGLE_CLOUD_API_KEY" };
    const adc_path = try scoped(a, io, ctx, credential, "GOOGLE_APPLICATION_CREDENTIALS");
    try io.checkCancel();
    const discovered = try ctx.fileExistsFn(ctx.context, io, adc_path orelse "~/.config/gcloud/application_default_credentials.json");
    try io.checkCancel();
    // Native inputs replace prohibited credential-byte retrieval. Keep this
    // supplied-document route while retaining the source discovery operation.
    const inline_document = if (credential) |value| j.field(value.env orelse .null, "GOOGLE_CREDENTIALS_JSON") else null;
    const has_credentials = discovered or ctx.inputs.googleCredentials != null or (inline_document != null and inline_document.?.len > 0);
    const project = (try scoped(a, io, ctx, credential, "GOOGLE_CLOUD_PROJECT")) orelse (try environment(a, io, ctx, "GCLOUD_PROJECT"));
    const location = try scoped(a, io, ctx, credential, "GOOGLE_CLOUD_LOCATION");
    if (has_credentials and project != null and project.?.len > 0 and location != null and location.?.len > 0) return .{ .auth = .{}, .env = try helpers.credentialEnv(a, if (credential) |value| value.env else null), .source = if (credential != null) "stored credential" else "gcloud application default credentials" };
    return null;
}
fn vertexLogin(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, interaction: auth.Interaction) !auth.ApiKeyCredential {
    try io.checkCancel();
    const method = try interaction.prompt(interaction.context, a, io, try j.from(a, .{ .type = "select", .message = "Select Google Vertex AI authentication method:", .options = .{ .{ .id = "api-key", .label = "Google Cloud API key" }, .{ .id = "adc", .label = "Application Default Credentials" }, .{ .id = "service-account", .label = "Service account credentials" } } }));
    try io.checkCancel();
    if (std.mem.eql(u8, method, "api-key")) return .{ .key = try prompt(a, io, interaction, "secret", "Enter Google Cloud API key") };
    const adc = std.mem.eql(u8, method, "adc");
    if (!adc and !std.mem.eql(u8, method, "service-account")) {
        if (interaction.errorMessage) |slot| slot.* = try std.fmt.allocPrint(a, "Unknown Google Vertex AI auth method: {s}", .{method});
        return error.UnknownAuthenticationMethod;
    }
    try interaction.notify(interaction.context, io, try j.from(a, .{ .type = "info", .message = if (adc) "Provide the project and location for Application Default Credentials supplied in memory or through metadata." else "Provide service account credentials as JSON, project, and location.", .links = .{.{ .label = "Application Default Credentials", .url = "https://cloud.google.com/docs/authentication/provide-credentials-adc" }} }));
    const project = try prompt(a, io, interaction, "text", "Enter Google Cloud project ID");
    const location = try prompt(a, io, interaction, "text", "Enter Google Cloud location");
    var env = try j.from(a, .{ .GOOGLE_CLOUD_PROJECT = project, .GOOGLE_CLOUD_LOCATION = location });
    if (!adc) {
        const text = try prompt(a, io, interaction, "secret", "Enter service account credentials JSON");
        const document = try j.parseFromSliceLeaky(std.json.Value, a, text, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always });
        if (document != .object) return error.InvalidGoogleCredentials;
        try j.put(a, &env, "GOOGLE_CREDENTIALS_JSON", j.str(try j.stringify(a, document, .{})));
    }
    return .{ .env = env };
}
pub fn vertex() auth.ApiKeyAuth {
    return .{ .name = "Google Cloud credentials", .login = vertexLogin, .resolve = vertexResolve };
}
