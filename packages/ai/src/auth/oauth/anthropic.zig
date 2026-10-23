const std = @import("std");
const auth = @import("../types.zig");
const transport = @import("../../transport/http.zig");
const j = @import("../../json/value.zig");
const uri = @import("../../transport/uri.zig");
const http = @import("http.zig");
const pkce = @import("pkce.zig");
const callback = @import("callback_server.zig");
const client_id = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const token_url = "https://platform.claude.com/v1/oauth/token";
const scopes = "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";
fn acceptCode(_: ?*anyopaque, _: std.mem.Allocator, _: std.Io, code: []const u8) ![]const u8 {
    return code;
}
fn tokenCredential(a: std.mem.Allocator, io: std.Io, value: std.json.Value, error_message: ?*?[]const u8) !auth.OAuthCredential {
    if (value == .null) {
        http.remember(error_message, "Cannot read properties of null (reading 'refresh_token')");
        return error.InvalidOAuthResponse;
    }
    // The pinned flow returns token fields directly and applies numeric
    // multiplication to expires_in; it does not impose positive lifetimes.
    const now = @as(f64, @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds()));
    const seconds = try @import("../../utils/numbers.zig").coerceValue(a, j.get(value, "expires_in"));
    const expires = now + seconds * 1000 - 300_000;
    var fields = j.map(a);
    try j.put(a, &fields, "type", j.str("oauth"));
    if (j.get(value, "refresh_token")) |raw| try j.put(a, &fields, "refresh", try j.clone(a, raw));
    if (j.get(value, "access_token")) |raw| try j.put(a, &fields, "access", try j.clone(a, raw));
    try j.put(a, &fields, "expires", j.num(expires));
    var result = auth.OAuthCredential{ .refresh = j.field(fields, "refresh") orelse "", .access = j.field(fields, "access") orelse "", .expires = expires, .metadata = fields, .jsonFields = fields };
    inline for (.{ "refresh", "access" }) |name| {
        if (j.get(fields, name)) |raw| {
            if (raw != .string) try j.retainField(a, &result, name, raw);
        } else j.markAbsent(&result, name);
    }
    return result;
}
fn exchange(a: std.mem.Allocator, io: std.Io, network: transport.Transport, code: []const u8, state: []const u8, verifier: []const u8, redirect: []const u8, error_message: ?*?[]const u8) !auth.OAuthCredential {
    const response = http.postJsonWithTimeout(a, io, network, token_url, .{ .grant_type = "authorization_code", .client_id = client_id, .code = code, .state = state, .redirect_uri = redirect, .code_verifier = verifier }, 30_000) catch |err| {
        if (err != error.OutOfMemory) {
            if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Token exchange request failed. url={s}; redirect_uri={s}; response_type=authorization_code; details={s}", .{ token_url, redirect, @errorName(err) });
        }
        return err;
    };
    if (!response.ok()) {
        if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Token exchange request failed. url={s}; redirect_uri={s}; response_type=authorization_code; details=Error: HTTP request failed. status={d}; url={s}; body={s}", .{ token_url, redirect, response.status, token_url, response.raw });
        return error.AnthropicTokenExchangeFailed;
    }
    if (response.jsonError) |err| {
        if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Token exchange returned invalid JSON. url={s}; body={s}; details={s}", .{ token_url, response.raw, @errorName(err) });
        return error.InvalidOAuthResponse;
    }
    return tokenCredential(a, io, response.body, error_message);
}
fn login(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, interaction: auth.Interaction, options: auth.LoginOptions) !auth.OAuthCredential {
    var shared = @import("../../utils/synchronized_allocator.zig").SynchronizedAllocator{ .child = a, .io = io };
    const result = try loginInner(ctx, shared.allocator(), io, network, interaction, options);
    return @import("../../types.zig").cloneInto(auth.OAuthCredential, a, result);
}
fn loginInner(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, interaction: auth.Interaction, options: auth.LoginOptions) !auth.OAuthCredential {
    const method = try interaction.prompt(interaction.context, a, io, try j.from(a, .{ .type = "select", .message = "Select Anthropic login method:", .options = .{ .{ .id = "browser", .label = "Browser login (default)" }, .{ .id = "copy_code", .label = "Copy code login (headless)" } } }));
    const copy = std.mem.eql(u8, method, "copy_code");
    if (!copy and !std.mem.eql(u8, method, "browser")) {
        http.remember(interaction.errorMessage, try std.fmt.allocPrint(a, "Unknown Anthropic login method: {s}", .{method}));
        return error.UnknownAnthropicLoginMethod;
    }
    const codes = try pkce.generatePKCE(io);
    var server: ?*callback.CallbackServer([]const u8) = null;
    if (!copy) {
        const configured_host = options.env.get("PI_OAUTH_CALLBACK_HOST") orelse "";
        var settings = callback.CallbackServer([]const u8).Options{ .providerName = "Anthropic", .host = if (configured_host.len > 0) configured_host else "127.0.0.1", .port = 53692, .path = "/callback", .redirectHost = "localhost", .state = &codes.verifier, .complete = acceptCode };
        server = callback.CallbackServer([]const u8).start(a, io, settings) catch |err| blk: {
            if (err == error.OutOfMemory or err == error.Canceled) return err;
            settings.port = 0;
            break :blk callback.CallbackServer([]const u8).start(a, io, settings) catch |fallback_error| {
                if (fallback_error == error.OutOfMemory or fallback_error == error.Canceled) return fallback_error;
                break :blk null;
            };
        };
    }
    defer if (server) |s| s.close();
    const redirect = if (copy) "https://platform.claude.com/oauth/code/callback" else if (server) |s| s.redirectUri else "http://localhost:53692/callback";
    var authorize = try std.Uri.parse("https://claude.ai/oauth/authorize");
    authorize.query = .{ .percent_encoded = try uri.encodeForm(a, &.{
        .{ .name = "code", .value = "true" },                     .{ .name = "client_id", .value = client_id },          .{ .name = "response_type", .value = "code" },  .{ .name = "redirect_uri", .value = redirect }, .{ .name = "scope", .value = scopes },
        .{ .name = "code_challenge", .value = &codes.challenge }, .{ .name = "code_challenge_method", .value = "S256" }, .{ .name = "state", .value = &codes.verifier },
    }) };
    try interaction.notify(interaction.context, io, try j.from(a, .{ .type = "auth_url", .url = try uri.render(a, authorize), .instructions = if (copy) "Complete login in your browser, then copy the code Anthropic shows and paste it here." else "Complete login in your browser. If the browser is on another machine, paste the final redirect URL here." }));
    var code: []const u8 = undefined;
    var state: []const u8 = &codes.verifier;
    if (copy) {
        const input = try interaction.prompt(interaction.context, a, io, try j.from(a, .{ .type = "manual_code", .message = "Paste the code Anthropic shows after you sign in:", .placeholder = "code#state" }));
        const parsed = try @import("authorization_input.zig").parse(a, input);
        if (parsed.state) |s| {
            if (s.len > 0 and !std.mem.eql(u8, s, &codes.verifier)) {
                http.remember(interaction.errorMessage, "OAuth state mismatch");
                return error.OAuthStateMismatch;
            }
            state = s;
        }
        code = parsed.code orelse "";
    } else switch (try callback.waitForCallbackOrManualInput([]const u8, a, io, interaction, server, "Complete login in your browser, or paste the authorization code / redirect URL here:", redirect)) {
        .callback => |value| code = value,
        .manual => |input| {
            const parsed = try @import("authorization_input.zig").parse(a, input);
            if (parsed.state) |s| {
                if (s.len > 0 and !std.mem.eql(u8, s, &codes.verifier)) {
                    http.remember(interaction.errorMessage, "OAuth state mismatch");
                    return error.OAuthStateMismatch;
                }
                state = s;
            }
            code = parsed.code orelse "";
        },
    }
    if (code.len == 0) {
        http.remember(interaction.errorMessage, "Missing authorization code");
        return error.MissingAuthorizationCode;
    }
    try interaction.notify(interaction.context, io, try j.from(a, .{ .type = "progress", .message = "Exchanging authorization code for tokens..." }));
    return exchange(a, io, network, code, state, &codes.verifier, redirect, interaction.errorMessage);
}
fn refresh(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, credential: auth.OAuthCredential) !auth.OAuthCredential {
    return refreshWithError(null, a, io, network, credential, null);
}
fn refreshWithError(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, credential: auth.OAuthCredential, error_message: ?*?[]const u8) !auth.OAuthCredential {
    var body = try j.from(a, .{ .grant_type = "refresh_token", .client_id = client_id });
    if (try j.fieldValue(a, credential, "refresh")) |refresh_token| try j.put(a, &body, "refresh_token", refresh_token);
    const response = http.postJsonWithTimeout(a, io, network, token_url, body, 30_000) catch |err| {
        if (err != error.OutOfMemory) {
            if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Anthropic token refresh request failed. url={s}; details={s}", .{ token_url, @errorName(err) });
        }
        return err;
    };
    if (!response.ok()) {
        if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Anthropic token refresh request failed. url={s}; details=Error: HTTP request failed. status={d}; url={s}; body={s}", .{ token_url, response.status, token_url, response.raw });
        return error.AnthropicTokenRefreshFailed;
    }
    if (response.jsonError) |err| {
        if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Anthropic token refresh returned invalid JSON. url={s}; body={s}; details={s}", .{ token_url, response.raw, @errorName(err) });
        return error.InvalidOAuthResponse;
    }
    return tokenCredential(a, io, response.body, error_message);
}
pub const anthropicOAuth = auth.OAuthAuth{ .name = "Anthropic (Claude Pro/Max)", .isSubscription = true, .login = login, .refresh = refresh, .refreshWithError = refreshWithError, .toAuth = http.auth };
