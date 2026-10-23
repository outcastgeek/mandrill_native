const std = @import("std");
const auth = @import("../types.zig");
const transport = @import("../../transport/http.zig");
const uri = @import("../../transport/uri.zig");
const j = @import("../../json/value.zig");
const http = @import("http.zig");
const callback = @import("callback_server.zig");
const polling = @import("device_code.zig");
const client_id = "app_EMoamEEZ73f0CkXaXp7hrann";
const token_url = "https://auth.openai.com/oauth/token";
const redirect = "http://localhost:1455/auth/callback";
fn postJson(a: std.mem.Allocator, io: std.Io, network: transport.Transport, url: []const u8, body: anytype) !http.Response {
    return http.request(a, io, network, .POST, url, try j.stringify(a, body, .{}), "application/json", &.{.{ .name = "accept", .value = null }});
}
fn postForm(a: std.mem.Allocator, io: std.Io, network: transport.Transport, fields: []const uri.Pair) !http.Response {
    return http.request(a, io, network, .POST, token_url, try uri.encodeForm(a, fields), "application/x-www-form-urlencoded", &.{.{ .name = "accept", .value = null }});
}
fn credential(a: std.mem.Allocator, io: std.Io, response: http.Response, operation: []const u8, error_message: ?*?[]const u8) !auth.OAuthCredential {
    if (!response.ok()) {
        http.remember(error_message, try std.fmt.allocPrint(a, "OpenAI Codex token {s} failed ({d}): {s}", .{ operation, response.status, if (response.raw.len > 0) response.raw else response.statusText }));
        return error.OpenAiCodexTokenFailed;
    }
    if (response.jsonError) |err| return err;
    const access = j.field(response.body, "access_token");
    const refresh_token = j.field(response.body, "refresh_token");
    const seconds = if (j.get(response.body, "expires_in")) |value| j.number(value) else null;
    if (access == null or access.?.len == 0 or refresh_token == null or refresh_token.?.len == 0 or seconds == null) {
        http.remember(error_message, try std.fmt.allocPrint(a, "OpenAI Codex token {s} response missing fields: {s}", .{ operation, try @import("../../json/value.zig").stringify(a, response.body, .{}) }));
        return error.InvalidOAuthResponse;
    }
    const claims = @import("jwt.zig").payload(a, access.?) catch |err| {
        if (err == error.OutOfMemory) return err;
        http.remember(error_message, "Failed to extract accountId from token");
        return error.MissingChatGptAccountId;
    };
    const account = j.field(j.get(claims, "https://api.openai.com/auth") orelse .null, "chatgpt_account_id");
    if (account == null or account.?.len == 0) {
        http.remember(error_message, "Failed to extract accountId from token");
        return error.MissingChatGptAccountId;
    }
    return .{ .access = access.?, .refresh = refresh_token.?, .expires = @import("../../utils/datetime.zig").expiresAfterNumber(io, seconds.?), .metadata = try j.from(a, .{ .accountId = account.? }) };
}
fn exchange(a: std.mem.Allocator, io: std.Io, network: transport.Transport, code: []const u8, verifier: []const u8, redirect_uri: []const u8, error_message: ?*?[]const u8) !auth.OAuthCredential {
    return credential(a, io, try postForm(a, io, network, &.{ .{ .name = "grant_type", .value = "authorization_code" }, .{ .name = "client_id", .value = client_id }, .{ .name = "code", .value = code }, .{ .name = "code_verifier", .value = verifier }, .{ .name = "redirect_uri", .value = redirect_uri } }), "exchange", error_message);
}
const DeviceResult = struct { code: []const u8, verifier: []const u8 };
const Poll = struct {
    a: std.mem.Allocator,
    network: transport.Transport,
    device: []const u8,
    user: []const u8,
    fn call(ctx: ?*anyopaque, io: std.Io) !polling.PollResult(DeviceResult) {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        const response = try postJson(self.a, io, self.network, "https://auth.openai.com/api/accounts/deviceauth/token", .{ .device_auth_id = self.device, .user_code = self.user });
        if (response.ok()) {
            if (response.jsonError) |err| return err;
            const code = j.field(response.body, "authorization_code");
            const verifier = j.field(response.body, "code_verifier");
            if (code == null or code.?.len == 0 or verifier == null or verifier.?.len == 0) return .{ .failed = try std.fmt.allocPrint(self.a, "Invalid OpenAI Codex device auth token response: {s}", .{try j.stringify(self.a, response.body, .{})}) };
            return .{ .complete = .{ .code = code.?, .verifier = verifier.? } };
        }
        if (response.status == 403 or response.status == 404) return .pending;
        const raw = j.get(response.body, "error") orelse .null;
        const code = if (raw == .object) j.field(raw, "code") else j.text(raw);
        if (code) |value| {
            if (std.mem.eql(u8, value, "deviceauth_authorization_pending")) return .pending;
            if (std.mem.eql(u8, value, "slow_down")) return .{ .slow_down = null };
        }
        return .{ .failed = try std.fmt.allocPrint(self.a, "OpenAI Codex device auth failed with status {d}{s}", .{ response.status, if (response.raw.len > 0) try std.fmt.allocPrint(self.a, ": {s}", .{response.raw}) else "" }) };
    }
};
fn identity(_: ?*anyopaque, a: std.mem.Allocator, _: std.Io, code: []const u8) ![]const u8 {
    return a.dupe(u8, code);
}
fn browser(a: std.mem.Allocator, io: std.Io, network: transport.Transport, interaction: auth.Interaction, options: auth.LoginOptions) !auth.OAuthCredential {
    const codes = try @import("pkce.zig").generatePKCE(io);
    var random: [16]u8 = undefined;
    try io.randomSecure(&random);
    const state = std.fmt.bytesToHex(random, .lower);
    const configured_host = options.env.get("PI_OAUTH_CALLBACK_HOST") orelse "";
    const server = callback.CallbackServer([]const u8).start(a, io, .{ .providerName = "OpenAI", .host = if (configured_host.len > 0) configured_host else "127.0.0.1", .redirectHost = "localhost", .port = 1455, .path = "/auth/callback", .state = &state, .complete = identity }) catch |err| blk: {
        if (err == error.Canceled or err == error.OutOfMemory) return err;
        break :blk null;
    };
    defer if (server) |value| value.close();
    var authorize = try std.Uri.parse("https://auth.openai.com/oauth/authorize");
    authorize.query = .{ .percent_encoded = try uri.encodeForm(a, &.{ .{ .name = "response_type", .value = "code" }, .{ .name = "client_id", .value = client_id }, .{ .name = "redirect_uri", .value = redirect }, .{ .name = "scope", .value = "openid profile email offline_access" }, .{ .name = "code_challenge", .value = &codes.challenge }, .{ .name = "code_challenge_method", .value = "S256" }, .{ .name = "state", .value = &state }, .{ .name = "id_token_add_organizations", .value = "true" }, .{ .name = "codex_cli_simplified_flow", .value = "true" }, .{ .name = "originator", .value = options.agentName orelse "pi" } }) };
    try interaction.notify(interaction.context, io, try j.from(a, .{ .type = "auth_url", .url = try uri.render(a, authorize), .instructions = "A browser window should open. Complete login to finish." }));
    const code = switch (try callback.waitForCallbackOrManualInput([]const u8, a, io, interaction, server, "Complete login in your browser, or paste the authorization code / redirect URL here:", redirect)) {
        .callback => |value| value,
        .manual => |input| blk: {
            const parsed = try @import("authorization_input.zig").parse(a, input);
            if (parsed.state) |actual| if (actual.len > 0 and !std.mem.eql(u8, actual, &state)) {
                http.remember(interaction.errorMessage, "State mismatch");
                return error.OAuthStateMismatch;
            };
            const code = parsed.code orelse "";
            if (code.len == 0) {
                http.remember(interaction.errorMessage, "Missing authorization code");
                return error.MissingAuthorizationCode;
            }
            break :blk code;
        },
    };
    return exchange(a, io, network, code, &codes.verifier, redirect, interaction.errorMessage);
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
    const method = try interaction.prompt(interaction.context, a, io, try j.from(a, .{ .type = "select", .message = "Select OpenAI Codex login method:", .options = .{ .{ .id = "browser", .label = "Browser login (default)" }, .{ .id = "device_code", .label = "Device code login (headless)" } } }));
    if (std.mem.eql(u8, method, "browser")) return browser(a, io, network, interaction, options);
    if (!std.mem.eql(u8, method, "device_code")) {
        http.remember(interaction.errorMessage, try std.fmt.allocPrint(a, "Unknown OpenAI Codex login method: {s}", .{method}));
        return error.UnknownLoginMethod;
    }
    const response = try postJson(a, io, network, "https://auth.openai.com/api/accounts/deviceauth/usercode", .{ .client_id = client_id });
    if (!response.ok()) {
        http.remember(interaction.errorMessage, if (response.status == 404) "OpenAI Codex device code login is not enabled for this server. Use browser login or verify the server URL." else try std.fmt.allocPrint(a, "OpenAI Codex device code request failed with status {d}{s}", .{ response.status, if (response.raw.len > 0) try std.fmt.allocPrint(a, ": {s}", .{response.raw}) else "" }));
        return error.OpenAiCodexDeviceAuthorizationFailed;
    }
    if (response.jsonError) |err| return err;
    const device = j.field(response.body, "device_auth_id");
    const user = j.field(response.body, "user_code");
    const raw = j.get(response.body, "interval") orelse .null;
    const interval: ?f64 = if (raw == .string) blk: {
        const text = try @import("../../utils/unicode.zig").trimWhitespace(a, raw.string);
        defer a.free(text);
        break :blk if (text.len == 0) @as(f64, 0) else std.fmt.parseFloat(f64, text) catch null;
    } else j.number(raw);
    if (device == null or device.?.len == 0 or user == null or user.?.len == 0 or interval == null or !std.math.isFinite(interval.?) or interval.? < 0) {
        http.remember(interaction.errorMessage, try std.fmt.allocPrint(a, "Invalid OpenAI Codex device code response: {s}", .{try j.stringify(a, response.body, .{})}));
        return error.InvalidOAuthResponse;
    }
    try interaction.notify(interaction.context, io, try j.from(a, .{ .type = "device_code", .userCode = user.?, .verificationUri = "https://auth.openai.com/codex/device", .intervalSeconds = interval.?, .expiresInSeconds = 900 }));
    var poll = Poll{ .a = a, .network = network, .device = device.?, .user = user.? };
    const result = try polling.pollOAuthDeviceCodeFlow(DeviceResult, io, .{ .context = &poll, .intervalSeconds = interval.?, .expiresInSeconds = 900, .poll = Poll.call, .errorMessage = interaction.errorMessage });
    return exchange(a, io, network, result.code, result.verifier, "https://auth.openai.com/deviceauth/callback", interaction.errorMessage);
}
fn refresh(context: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, previous: auth.OAuthCredential) !auth.OAuthCredential {
    return refreshWithError(context, a, io, network, previous, null);
}
fn refreshWithError(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, previous: auth.OAuthCredential, error_message: ?*?[]const u8) !auth.OAuthCredential {
    const response = postForm(a, io, network, &.{ .{ .name = "grant_type", .value = "refresh_token" }, .{ .name = "refresh_token", .value = previous.refresh }, .{ .name = "client_id", .value = client_id } }) catch |err| {
        if (err != error.OutOfMemory) http.remember(error_message, try std.fmt.allocPrint(a, "OpenAI Codex token refresh error: {s}", .{@errorName(err)}));
        return err;
    };
    return credential(a, io, response, "refresh", error_message);
}
pub const openaiCodexOAuth = auth.OAuthAuth{ .name = "OpenAI (ChatGPT Plus/Pro)", .isSubscription = true, .login = login, .refresh = refresh, .refreshWithError = refreshWithError, .toAuth = http.auth };
