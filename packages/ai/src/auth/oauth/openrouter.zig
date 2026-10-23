const std = @import("std");
const auth = @import("../types.zig");
const transport = @import("../../transport/http.zig");
const j = @import("../../json/value.zig");
const uri = @import("../../transport/uri.zig");
const http = @import("http.zig");
const callback = @import("callback_server.zig");
const Exchange = struct {
    network: transport.Transport,
    verifier: []const u8,
    errorMessage: ?[]const u8 = null,
    fn remember(self: *@This(), message: []const u8) void {
        self.errorMessage = message;
    }
    fn call(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, code: []const u8) !auth.OAuthCredential {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        const response = http.requestWithOptions(a, io, self.network, .POST, "https://openrouter.ai/api/v1/auth/keys", try j.stringify(a, .{ .code = code, .code_verifier = self.verifier, .code_challenge_method = "S256" }, .{}), "application/json", &.{}, .{ .timeoutMs = 30_000, .ignoreHttpErrorBodyReadFailure = true }) catch |err| {
            if (err == error.Canceled) self.remember("Login cancelled") else if (err == error.RequestTimeout) self.remember("OpenRouter OAuth token exchange timed out");
            return err;
        };
        if (response.ok() and response.jsonError != null) {
            self.remember("OpenRouter OAuth returned invalid JSON");
            return error.InvalidOAuthResponse;
        }
        if (!response.ok()) {
            const detail = j.field(response.body, "error_description") orelse j.field(response.body, "message") orelse j.field(response.body, "error") orelse j.field(j.get(response.body, "error") orelse .null, "message");
            self.remember(if (detail != null and detail.?.len > 0) try std.fmt.allocPrint(a, "OpenRouter OAuth key exchange failed (HTTP {d}): {s}", .{ response.status, detail.? }) else try std.fmt.allocPrint(a, "OpenRouter OAuth key exchange failed (HTTP {d})", .{response.status}));
            return error.OpenRouterKeyExchangeFailed;
        }
        const key = http.requiredString(response.body, "key") catch |err| {
            self.remember("OpenRouter OAuth response carries no \"key\"");
            return err;
        };
        return .{ .access = key, .refresh = "", .expires = 9007199254740991 };
    }
};
fn login(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, interaction: auth.Interaction, options: auth.LoginOptions) !auth.OAuthCredential {
    var shared = @import("../../utils/synchronized_allocator.zig").SynchronizedAllocator{ .child = a, .io = io };
    const result = loginInner(ctx, shared.allocator(), io, network, interaction, options) catch |err| {
        if (err == error.Canceled) http.remember(interaction.errorMessage, "Login cancelled");
        return err;
    };
    return @import("../../types.zig").cloneInto(auth.OAuthCredential, a, result);
}
fn loginInner(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, interaction: auth.Interaction, options: auth.LoginOptions) !auth.OAuthCredential {
    const codes = try @import("pkce.zig").generatePKCE(io);
    var exchange = Exchange{ .network = network, .verifier = &codes.verifier };
    const random = try @import("../../utils/uuid.zig").randomV4(io);
    const path = try std.fmt.allocPrint(a, "/oauth/callback/{s}", .{random});
    const configured_host = options.env.get("PI_OAUTH_CALLBACK_HOST") orelse "";
    const server = try callback.CallbackServer(auth.OAuthCredential).start(a, io, .{ .providerName = "OpenRouter", .host = if (configured_host.len > 0) configured_host else "127.0.0.1", .port = 0, .path = path, .complete = Exchange.call, .context = &exchange, .timeoutMs = 300_000 });
    defer {
        server.stop();
        if (exchange.errorMessage) |message| if (interaction.errorMessage) |slot| {
            slot.* = a.dupe(u8, message) catch null;
        };
        server.close();
    }
    var authorize = try std.Uri.parse("https://openrouter.ai/auth");
    authorize.query = .{ .percent_encoded = try uri.encodeForm(a, &.{ .{ .name = "callback_url", .value = server.redirectUri }, .{ .name = "code_challenge", .value = &codes.challenge }, .{ .name = "code_challenge_method", .value = "S256" } }) };
    try interaction.notify(interaction.context, io, try j.from(a, .{ .type = "progress", .message = try std.fmt.allocPrint(a, "Listening for OpenRouter OAuth callback on {s}", .{server.redirectUri}) }));
    try interaction.notify(interaction.context, io, try j.from(a, .{ .type = "auth_url", .url = try uri.render(a, authorize), .instructions = "Complete sign-in in your browser. If the browser is on another machine, paste the final redirect URL here." }));
    switch (try callback.waitForCallbackOrManualInput(auth.OAuthCredential, a, io, interaction, server, "Complete sign-in in your browser, or paste the authorization code / redirect URL here:", server.redirectUri)) {
        .callback => |value| return (try @import("../../types.zig").Owned(auth.Credential).clone(a, .{ .oauth = value })).value().oauth,
        .manual => |input| {
            const parsed = try @import("authorization_input.zig").parseOpenRouter(a, input);
            const code = parsed.code orelse "";
            if (code.len == 0) {
                http.remember(interaction.errorMessage, "Missing authorization code");
                return error.MissingAuthorizationCode;
            }
            try interaction.notify(interaction.context, io, try j.from(a, .{ .type = "progress", .message = "Exchanging authorization code for an API key..." }));
            return Exchange.call(&exchange, a, io, code);
        },
    }
}
fn refresh(_: ?*anyopaque, _: std.mem.Allocator, _: std.Io, _: transport.Transport, credential: auth.OAuthCredential) !auth.OAuthCredential {
    return credential;
}
pub const openRouterOAuth = auth.OAuthAuth{ .name = "OpenRouter OAuth", .loginLabel = "Sign in with OpenRouter", .login = login, .refresh = refresh, .toAuth = http.auth };
