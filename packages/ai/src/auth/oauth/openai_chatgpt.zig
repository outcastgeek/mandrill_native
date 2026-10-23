const std = @import("std");
const auth = @import("../types.zig");
const transport = @import("../../transport/http.zig");
const uri = @import("../../transport/uri.zig");
const j = @import("../../json/value.zig");
const http = @import("http.zig");
const callback = @import("callback_server.zig");
const token_url = "https://auth.openai.com/api/accounts/oauth/token";
const redirect = "http://127.0.0.1:1455/auth/callback";
const resource = "https://api.openai.com/v1";
const direct_scope = "chatgpt.tokens.use.direct";
const Authorization = struct { code: []const u8, clientId: []const u8 };
fn requestToken(a: std.mem.Allocator, io: std.Io, network: transport.Transport, fields: []const uri.Pair) !http.Response {
    return http.requestWithOptions(a, io, network, .POST, token_url, try uri.encodeForm(a, fields), "application/x-www-form-urlencoded", &.{}, .{ .ignoreHttpErrorBodyReadFailure = true });
}
fn fromCallback(a: std.mem.Allocator, target: std.Uri, expected_state: []const u8, error_message: ?*?[]const u8) !Authorization {
    const query: std.Uri.Component = target.query orelse .{ .percent_encoded = "" };
    const code = (try uri.queryValue(a, query, "code")) orelse "";
    if (code.len == 0) {
        http.remember(error_message, "Missing authorization code");
        return error.MissingAuthorizationCode;
    }
    const state = (try uri.queryValue(a, query, "state")) orelse "";
    if (state.len == 0) {
        http.remember(error_message, "Missing OAuth state");
        return error.MissingOAuthState;
    }
    if (!std.mem.eql(u8, state, expected_state)) {
        http.remember(error_message, "OAuth state mismatch");
        return error.OAuthStateMismatch;
    }
    const client = try @import("../../utils/unicode.zig").trimWhitespace(a, (try uri.queryValue(a, query, "client_id")) orelse "");
    if (client.len == 0) {
        http.remember(error_message, "OpenAI OAuth registration callback did not contain an issued client ID");
        return error.MissingIssuedClientId;
    }
    return .{ .code = try a.dupe(u8, code), .clientId = client };
}
fn callbackQuery(ctx: ?*anyopaque, a: std.mem.Allocator, _: std.Io, target: std.Uri) !callback.CallbackServer(Authorization).QueryResult {
    const expected_state: *const []const u8 = @ptrCast(@alignCast(ctx.?));
    if (try uri.queryValue(a, target.query orelse .{ .percent_encoded = "" }, "error")) |failure| if (failure.len > 0) return .{ .failed = .{ .err = error.OAuthAuthorizationFailed, .message = try std.fmt.allocPrint(a, "ChatGPT authorization failed: {s}", .{failure}), .responseMessage = "ChatGPT was not connected.", .responseDetails = try std.fmt.allocPrint(a, "Error: {s}", .{failure}) } };
    var detail: ?[]const u8 = null;
    const result = fromCallback(a, target, expected_state.*, &detail) catch |err| {
        if (err == error.OutOfMemory or err == error.Canceled) return err;
        return .{ .retry = detail orelse "Invalid callback" };
    };
    return .{ .complete = .{ .value = result, .message = "ChatGPT authentication completed. You can close this window." } };
}
fn fromManual(a: std.mem.Allocator, input: []const u8, state: []const u8, error_message: ?*?[]const u8) !Authorization {
    const text = try @import("../../utils/unicode.zig").trimWhitespace(a, input);
    defer a.free(text);
    if (try @import("../../schema/formats.zig").canParseAbsoluteUrl(a, text) == false) {
        http.remember(error_message, "Paste the full callback URL from the browser");
        return error.InvalidCallbackUrl;
    }
    const target = uri.parseHttpUrl(a, text) catch |err| {
        if (err == error.OutOfMemory) return err;
        http.remember(error_message, "Paste the full callback URL from the browser");
        return error.InvalidCallbackUrl;
    };
    const path = switch (target.path) {
        .raw, .percent_encoded => |part| part,
    };
    if (!std.ascii.eqlIgnoreCase(target.scheme, "http") or target.port != 1455 or target.host == null or !std.mem.eql(u8, try target.host.?.toRawMaybeAlloc(a), "127.0.0.1") or !std.mem.eql(u8, path, "/auth/callback")) {
        http.remember(error_message, "The pasted callback URL must start with " ++ redirect);
        return error.InvalidCallbackUrl;
    }
    if (try uri.queryValue(a, target.query orelse .{ .percent_encoded = "" }, "error")) |failure| if (failure.len > 0) {
        http.remember(error_message, try std.fmt.allocPrint(a, "ChatGPT authorization failed: {s}", .{failure}));
        return error.OAuthAuthorizationFailed;
    };
    return fromCallback(a, target, state, error_message);
}
fn credential(a: std.mem.Allocator, io: std.Io, response: http.Response, client_id: []const u8, error_message: ?*?[]const u8) !auth.OAuthCredential {
    if (!response.ok()) {
        http.remember(error_message, try std.fmt.allocPrint(a, "OpenAI OAuth token request failed ({d}): {s}", .{ response.status, if (response.raw.len > 0) response.raw else response.statusText }));
        return error.OpenAiOAuthTokenFailed;
    }
    if (response.jsonError) |err| return err;
    if (response.body != .object) {
        http.remember(error_message, "OpenAI OAuth token response must be an object");
        return error.InvalidOAuthResponse;
    }
    for ([_][]const u8{ "access_token", "refresh_token", "scope" }) |key| {
        const value = j.field(response.body, key);
        if (value == null or try @import("../../utils/unicode.zig").isBlank(value.?)) {
            http.remember(error_message, try std.fmt.allocPrint(a, "OpenAI OAuth token response has invalid {s}", .{key}));
            return error.InvalidOAuthResponse;
        }
    }
    const seconds = http.positiveNumber(response.body, "expires_in") catch |err| {
        http.remember(error_message, "OpenAI OAuth token response has invalid expires_in");
        return err;
    };
    const scope = j.field(response.body, "scope").?;
    const scopes = try @import("../../utils/unicode.zig").splitWhitespace(a, scope);
    var has_direct = false;
    for (scopes) |item| if (std.mem.eql(u8, item, direct_scope)) {
        has_direct = true;
        break;
    };
    if (!has_direct) {
        http.remember(error_message, "OpenAI OAuth grant did not include " ++ direct_scope);
        return error.DirectTokenScopeMissing;
    }
    return .{ .access = j.field(response.body, "access_token").?, .refresh = j.field(response.body, "refresh_token").?, .expires = @import("../../utils/datetime.zig").expiresAfterNumber(io, seconds) - 180_000, .metadata = try j.from(a, .{ .clientId = client_id, .scopes = scopes }) };
}
fn hostId(a: std.mem.Allocator, id: []const u8) ![]const u8 {
    var reader = std.Io.Reader.fixed(id);
    const sizes = [_]usize{ 8, 4, 4, 4, 12 };
    for (sizes, 0..) |size, index| {
        const part = if (index == sizes.len - 1) reader.buffered() else (try reader.takeDelimiter('-')) orelse return error.InvalidDeviceId;
        if (part.len != size) return error.InvalidDeviceId;
        var bytes: [6]u8 = undefined;
        _ = std.fmt.hexToBytes(bytes[0 .. size / 2], part) catch return error.InvalidDeviceId;
    }
    return std.fmt.allocPrint(a, "urn:uuid:{s}", .{try std.ascii.allocLowerString(a, id)});
}
fn login(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, interaction: auth.Interaction, options: auth.LoginOptions) !auth.OAuthCredential {
    var shared = @import("../../utils/synchronized_allocator.zig").SynchronizedAllocator{ .child = a, .io = io };
    const result = loginInner(ctx, shared.allocator(), io, network, interaction, options) catch |err| {
        if (err == error.Canceled) http.remember(interaction.errorMessage, "Login cancelled");
        return err;
    };
    return @import("../../types.zig").cloneInto(auth.OAuthCredential, a, result);
}
fn loginInner(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, interaction: auth.Interaction, options: auth.LoginOptions) !auth.OAuthCredential {
    const device = options.getDeviceId orelse {
        http.remember(interaction.errorMessage, "Sign in with ChatGPT requires a device ID (UUID) for this installation");
        return error.DeviceIdRequired;
    };
    const host = hostId(a, device(options.deviceContext)) catch |err| {
        if (err == error.InvalidDeviceId) http.remember(interaction.errorMessage, "Sign in with ChatGPT requires a device ID (UUID) for this installation");
        return err;
    };
    const codes = try @import("pkce.zig").generatePKCE(io);
    const state = try @import("pkce.zig").generatePKCE(io);
    const nonce = try @import("pkce.zig").generatePKCE(io);
    var callback_state: []const u8 = &state.verifier;
    const configured_host = options.env.get("PI_OAUTH_CALLBACK_HOST") orelse "";
    const server = callback.CallbackServer(Authorization).start(a, io, .{ .providerName = "OpenAI", .host = if (configured_host.len > 0) configured_host else "127.0.0.1", .redirectHost = "127.0.0.1", .port = 1455, .path = "/auth/callback", .context = @ptrCast(&callback_state), .queryHandler = callbackQuery, .queryResponseHtml = true }) catch |err| {
        if (err == error.AddressInUse) http.remember(interaction.errorMessage, "Port 1455 is in use, probably by an unfinished login in another pi session or by the Codex CLI. Cancel that login and try again.");
        return err;
    };
    defer server.close();
    var authorize = try std.Uri.parse("https://auth.openai.com/api/accounts/authorize");
    authorize.query = .{ .percent_encoded = try uri.encodeForm(a, &.{ .{ .name = "client_id", .value = "dynamic_agent_client" }, .{ .name = "agent_name_hint", .value = options.agentName orelse "Pi" }, .{ .name = "ext_agent_host_id", .value = host }, .{ .name = "response_type", .value = "code" }, .{ .name = "redirect_uri", .value = redirect }, .{ .name = "resource", .value = resource }, .{ .name = "scope", .value = "openid profile email offline_access resource.invoke " ++ direct_scope }, .{ .name = "state", .value = &state.verifier }, .{ .name = "code_challenge", .value = &codes.challenge }, .{ .name = "code_challenge_method", .value = "S256" }, .{ .name = "nonce", .value = &nonce.verifier } }) };
    try interaction.notify(interaction.context, io, try j.from(a, .{ .type = "auth_url", .url = try uri.render(a, authorize), .instructions = "Complete sign-in in your browser. If the callback does not complete, paste the final redirect URL here." }));
    const result = switch (try callback.waitForCallbackOrManualInput(Authorization, a, io, interaction, server, "Complete login in your browser, or paste the final redirect URL here:", redirect)) {
        .callback => |value| value,
        .manual => |input| try fromManual(a, input, &state.verifier, interaction.errorMessage),
    };
    try interaction.notify(interaction.context, io, try j.from(a, .{ .type = "progress", .message = "Exchanging authorization code for tokens..." }));
    const response = try requestToken(a, io, network, &.{ .{ .name = "grant_type", .value = "authorization_code" }, .{ .name = "client_id", .value = result.clientId }, .{ .name = "code", .value = result.code }, .{ .name = "code_verifier", .value = &codes.verifier }, .{ .name = "redirect_uri", .value = redirect }, .{ .name = "resource", .value = resource } });
    if (!response.ok() or response.jsonError != null or response.body != .object) return credential(a, io, response, try a.dupe(u8, result.clientId), interaction.errorMessage);
    const id_token = j.field(response.body, "id_token");
    if (id_token == null or try @import("../../utils/unicode.zig").isBlank(id_token.?)) {
        http.remember(interaction.errorMessage, "OpenAI OAuth token response did not contain an ID token");
        return error.IdTokenMissing;
    }
    return credential(a, io, response, try a.dupe(u8, result.clientId), interaction.errorMessage);
}
fn refresh(context: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, previous: auth.OAuthCredential) !auth.OAuthCredential {
    return refreshWithError(context, a, io, network, previous, null);
}
fn refreshWithError(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, previous: auth.OAuthCredential, error_message: ?*?[]const u8) !auth.OAuthCredential {
    const client = j.field(previous.metadata orelse .null, "clientId");
    if (client == null or try @import("../../utils/unicode.zig").isBlank(client.?)) {
        http.remember(error_message, "Stored OpenAI OAuth credential does not contain an issued client ID; reconnect ChatGPT");
        return error.MissingIssuedClientId;
    }
    return credential(a, io, try requestToken(a, io, network, &.{ .{ .name = "grant_type", .value = "refresh_token" }, .{ .name = "client_id", .value = client.? }, .{ .name = "refresh_token", .value = previous.refresh }, .{ .name = "resource", .value = resource } }), client.?, error_message);
}
pub const openaiChatGPTOAuth = auth.OAuthAuth{ .name = "OpenAI (ChatGPT subscription)", .isSubscription = true, .loginLabel = "Sign in with ChatGPT", .login = login, .refresh = refresh, .refreshWithError = refreshWithError, .toAuth = http.auth };
