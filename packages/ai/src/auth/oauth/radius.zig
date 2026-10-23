const std = @import("std");
const auth = @import("../types.zig");
const transport = @import("../../transport/http.zig");
const uri = @import("../../transport/uri.zig");
const j = @import("../../json/value.zig");
const http = @import("http.zig");
const polling = @import("device_code.zig");
const callback = @import("callback_server.zig");
pub const RadiusOAuthOptions = struct { name: []const u8, gateway: []const u8 };
pub const RadiusOAuth = struct {
    options: RadiusOAuthOptions,
    pub fn authMethod(self: *@This()) auth.OAuthAuth {
        return .{ .name = self.options.name, .context = self, .login = login, .refresh = refresh, .refreshWithError = refreshWithError, .toAuth = http.auth };
    }
    fn endpoint(self: *@This(), a: std.mem.Allocator, path: []const u8) ![]const u8 {
        const raw = try std.Uri.parse(try @import("../../providers/radius.zig").normalizeRadiusGatewayUrl(a, self.options.gateway));
        var result = raw;
        result.path = .{ .raw = path };
        result.query = null;
        result.fragment = null;
        return uri.render(a, result);
    }
    fn formRequest(self: *@This(), a: std.mem.Allocator, io: std.Io, network: transport.Transport, path: []const u8, fields: []const uri.Pair) !http.Response {
        const url = try uri.parseHttpUrl(a, try self.endpoint(a, path));
        var response = try network.send(a, io, .{ .method = .POST, .uri = url, .body = try uri.encodeForm(a, fields), .headers = &.{ .{ .name = "accept", .value = "application/json" }, .{ .name = "content-type", .value = "application/x-www-form-urlencoded" } } });
        defer response.deinit();
        const wire = response.readAll(a, .unlimited) catch |err| {
            if (err == error.OutOfMemory or (response.status >= 200 and response.status < 300)) return err;
            // readOAuthResponseError catches response.text() rejection after
            // Fetch has supplied a non-success response, including body abort.
            return .{ .status = response.status, .statusText = try a.dupe(u8, response.statusText), .raw = "", .body = j.map(a) };
        };
        defer a.free(wire);
        const text = try http.responseText(a, wire, .fetch);
        errdefer a.free(text);
        const status_text = try a.dupe(u8, response.statusText);
        errdefer a.free(status_text);
        const body = j.parseFromSliceLeaky(std.json.Value, a, text, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |err| {
            if (err == error.OutOfMemory) return err;
            return .{ .status = response.status, .statusText = status_text, .raw = text, .body = j.map(a), .jsonError = err };
        };
        return .{ .status = response.status, .statusText = status_text, .raw = text, .body = body };
    }
    fn token(self: *@This(), a: std.mem.Allocator, io: std.Io, network: transport.Transport, fields: []const uri.Pair) !http.Response {
        return self.formRequest(a, io, network, "/v1/oauth/token", fields);
    }
    fn credential(io: std.Io, response: http.Response, a: std.mem.Allocator, error_message: ?*?[]const u8) !auth.OAuthCredential {
        if (!response.ok()) {
            http.remember(error_message, try responseMessage(a, response, "Radius OAuth token request failed"));
            return error.RadiusOAuthTokenFailed;
        }
        if (response.jsonError) |err| return err;
        if (response.body == .null) {
            http.remember(error_message, "Cannot read properties of null (reading 'access_token')");
            return error.InvalidOAuthResponse;
        }
        const now = @as(f64, @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds()));
        const seconds = try @import("../../utils/numbers.zig").coerceValue(a, j.get(response.body, "expires_in"));
        const expires = now + seconds * 1000 - 60_000;
        var fields = j.map(a);
        try j.put(a, &fields, "type", j.str("oauth"));
        if (j.get(response.body, "access_token")) |raw| try j.put(a, &fields, "access", try j.clone(a, raw));
        if (j.get(response.body, "refresh_token")) |raw| try j.put(a, &fields, "refresh", try j.clone(a, raw));
        try j.put(a, &fields, "expires", j.num(expires));
        if (j.get(response.body, "scope")) |raw| try j.put(a, &fields, "scope", try j.clone(a, raw));
        var result = auth.OAuthCredential{ .access = j.field(fields, "access") orelse "", .refresh = j.field(fields, "refresh") orelse "", .expires = expires, .metadata = fields, .jsonFields = fields };
        inline for (.{ "access", "refresh" }) |name| {
            if (j.get(fields, name)) |raw| {
                if (raw != .string) try j.retainField(a, &result, name, raw);
            } else j.markAbsent(&result, name);
        }
        return result;
    }
    const Exchange = struct {
        owner: *RadiusOAuth,
        network: transport.Transport,
        verifier: []const u8,
        errorMessage: ?[]const u8 = null,
        fn call(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, code: []const u8) !auth.OAuthCredential {
            const self: *@This() = @ptrCast(@alignCast(ctx.?));
            return RadiusOAuth.credential(io, try self.owner.token(a, io, self.network, &.{ .{ .name = "grant_type", .value = "authorization_code" }, .{ .name = "client_id", .value = "pi-gateway" }, .{ .name = "redirect_uri", .value = "http://127.0.0.1:1456/oauth/callback" }, .{ .name = "code", .value = code }, .{ .name = "code_verifier", .value = self.verifier } }), a, &self.errorMessage);
        }
    };
    const Poll = struct {
        owner: *RadiusOAuth,
        a: std.mem.Allocator,
        network: transport.Transport,
        device: []const u8,
        fn call(ctx: ?*anyopaque, io: std.Io) !polling.PollResult(auth.OAuthCredential) {
            const self: *@This() = @ptrCast(@alignCast(ctx.?));
            const response = try self.owner.token(self.a, io, self.network, &.{ .{ .name = "grant_type", .value = "urn:ietf:params:oauth:grant-type:device_code" }, .{ .name = "client_id", .value = "pi-gateway" }, .{ .name = "device_code", .value = self.device } });
            if (response.ok()) return .{ .complete = try RadiusOAuth.credential(io, response, self.a, null) };
            const err = j.field(response.body, "error") orelse "";
            if (std.mem.eql(u8, err, "authorization_pending")) return .pending;
            if (std.mem.eql(u8, err, "slow_down")) return .{ .slow_down = null };
            if (std.mem.eql(u8, err, "expired_token")) return .{ .failed = "Device authorization expired." };
            if (std.mem.eql(u8, err, "access_denied")) return .{ .failed = "Device authorization was denied." };
            return .{ .failed = try RadiusOAuth.responseMessage(self.a, response, "Radius OAuth token request failed") };
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
    fn loginInner(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, interaction: auth.Interaction, _: auth.LoginOptions) !auth.OAuthCredential {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        const method = try interaction.prompt(interaction.context, a, io, try j.from(a, .{ .type = "select", .message = try std.fmt.allocPrint(a, "Sign in to {s}:", .{self.options.name}), .options = .{ .{ .id = "browser", .label = "Sign in with browser (recommended)" }, .{ .id = "device-code", .label = "Sign in with device code (when signing in from another device)" } } }));
        if (std.mem.eql(u8, method, "device-code")) {
            const response = try self.formRequest(a, io, network, "/v1/oauth/device", &.{ .{ .name = "client_id", .value = "pi-gateway" }, .{ .name = "scope", .value = "gateway offline_access" } });
            if (!response.ok()) {
                http.remember(interaction.errorMessage, try responseMessage(a, response, "Radius OAuth device authorization failed"));
                return error.RadiusDeviceAuthorizationFailed;
            }
            if (response.jsonError) |err| return err;
            if (response.body == .null) {
                http.remember(interaction.errorMessage, "Cannot read properties of null (reading 'device_code')");
                return error.InvalidOAuthResponse;
            }
            const device = j.get(response.body, "device_code") orelse .null;
            const user = j.get(response.body, "user_code") orelse .null;
            const verification = j.get(response.body, "verification_uri") orelse .null;
            const raw_lifetime = j.get(response.body, "expires_in") orelse .null;
            const raw_interval = j.getNonNull(response.body, "interval");
            if (!j.truthy(device) or !j.truthy(user) or !j.truthy(verification) or !j.truthy(raw_lifetime)) {
                http.remember(interaction.errorMessage, "Radius OAuth device authorization response is missing required fields");
                return error.InvalidOAuthResponse;
            }
            var notification = try j.from(a, .{ .type = "device_code", .userCode = user, .verificationUri = verification, .expiresInSeconds = raw_lifetime });
            if (j.get(response.body, "interval")) |value| try j.put(a, &notification, "intervalSeconds", value);
            try interaction.notify(interaction.context, io, notification);
            const device_text = try @import("../../utils/diagnostics.zig").formatThrownValue(a, .{ .value = device });
            const interval = if (raw_interval) |value| try @import("../../utils/numbers.zig").coerceValue(a, value) else null;
            var poll = Poll{ .owner = self, .a = a, .network = network, .device = device_text };
            return polling.pollOAuthDeviceCodeFlow(auth.OAuthCredential, io, .{ .context = &poll, .intervalSeconds = interval, .expiresInSeconds = j.number(raw_lifetime), .poll = Poll.call, .errorMessage = interaction.errorMessage });
        }
        if (!std.mem.eql(u8, method, "browser")) {
            http.remember(interaction.errorMessage, try std.fmt.allocPrint(a, "Unknown {s} sign-in method: {s}", .{ self.options.name, method }));
            return error.UnknownLoginMethod;
        }
        const discovery = try http.request(a, io, network, .GET, try self.endpoint(a, "/v1/oauth"), null, null, &.{});
        if (!discovery.ok()) {
            http.remember(interaction.errorMessage, try std.fmt.allocPrint(a, "Could not load Radius OAuth config from {s}: {d} {s}", .{ try @import("../../providers/radius.zig").normalizeRadiusGatewayUrl(a, self.options.gateway), discovery.status, discovery.raw }));
            return error.RadiusDiscoveryFailed;
        }
        if (discovery.jsonError) |err| return err;
        if (discovery.body == .null) {
            http.remember(interaction.errorMessage, "Cannot read properties of null (reading 'authorizationEndpoint')");
            return error.InvalidOAuthResponse;
        }
        const authorization_endpoint = j.field(discovery.body, "authorizationEndpoint") orelse {
            http.remember(interaction.errorMessage, try std.fmt.allocPrint(a, "Invalid Radius OAuth config from {s}", .{try @import("../../providers/radius.zig").normalizeRadiusGatewayUrl(a, self.options.gateway)}));
            return error.RadiusDiscoveryFailed;
        };
        const codes = try @import("pkce.zig").generatePKCE(io);
        const state = try @import("../../utils/uuid.zig").randomV4(io);
        var exchange = Exchange{ .owner = self, .network = network, .verifier = &codes.verifier };
        var authorize = try std.Uri.parse(authorization_endpoint);
        authorize.query = .{ .percent_encoded = try uri.encodeForm(a, &.{ .{ .name = "response_type", .value = "code" }, .{ .name = "client_id", .value = "pi-gateway" }, .{ .name = "redirect_uri", .value = "http://127.0.0.1:1456/oauth/callback" }, .{ .name = "scope", .value = "gateway offline_access" }, .{ .name = "code_challenge", .value = &codes.challenge }, .{ .name = "code_challenge_method", .value = "S256" }, .{ .name = "handoff", .value = "url" }, .{ .name = "state", .value = &state } }) };
        const server = try callback.CallbackServer(auth.OAuthCredential).start(a, io, .{ .providerName = "Radius", .host = "127.0.0.1", .port = 1456, .path = "/oauth/callback", .state = &state, .context = &exchange, .complete = Exchange.call });
        defer {
            server.stop();
            if (exchange.errorMessage) |message| http.remember(interaction.errorMessage, a.dupe(u8, message) catch "Radius OAuth token request failed");
            server.close();
        }
        try interaction.notify(interaction.context, io, try j.from(a, .{ .type = "progress", .message = try std.fmt.allocPrint(a, "Listening for OAuth callback on {s}", .{server.redirectUri}) }));
        try interaction.notify(interaction.context, io, try j.from(a, .{ .type = "auth_url", .url = try uri.render(a, authorize), .instructions = "Continue in your browser." }));
        const result = (try server.waitWithError(a, interaction.errorMessage)) orelse {
            http.remember(interaction.errorMessage, "OAuth callback did not complete.");
            return error.OAuthCallbackIncomplete;
        };
        return (try @import("../../types.zig").Owned(auth.Credential).clone(a, .{ .oauth = result })).value().oauth;
    }
    fn responseMessage(a: std.mem.Allocator, response: http.Response, prefix: []const u8) ![]const u8 {
        const code = j.field(response.body, "error");
        // JSON null throws during the source's data.error lookup inside its
        // try block, selecting the same raw-text fallback as invalid JSON.
        const description = if (response.jsonError != null or response.body == .null) response.raw else j.field(response.body, "error_description");
        var detail = if (description) |text| if (text.len > 0) text else try std.fmt.allocPrint(a, "{d}", .{response.status}) else try std.fmt.allocPrint(a, "{d}", .{response.status});
        if (code) |value| if (value.len > 0) {
            detail = if (description) |text| if (text.len > 0) try std.fmt.allocPrint(a, "{s}: {s}", .{ value, text }) else value else value;
        };
        return std.fmt.allocPrint(a, "{s}: {s}", .{ prefix, detail });
    }
    fn refresh(context: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, previous: auth.OAuthCredential) !auth.OAuthCredential {
        return refreshWithError(context, a, io, network, previous, null);
    }
    fn refreshWithError(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, previous: auth.OAuthCredential, error_message: ?*?[]const u8) !auth.OAuthCredential {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        const raw_refresh = try j.fieldValue(a, previous, "refresh");
        const refresh_text = @import("../../utils/diagnostics.zig").formatThrownValue(a, if (raw_refresh) |value| .{ .value = value } else .undefined_value) catch |err| {
            if (err == error.CannotConvertObjectToPrimitive) http.remember(error_message, "Cannot convert object to primitive value");
            return err;
        };
        const response = self.token(a, io, network, &.{ .{ .name = "grant_type", .value = "refresh_token" }, .{ .name = "client_id", .value = "pi-gateway" }, .{ .name = "refresh_token", .value = refresh_text } }) catch |err| {
            if (err == error.Canceled) http.remember(error_message, "Login cancelled");
            return err;
        };
        return credential(io, response, a, error_message);
    }
};
pub fn createRadiusOAuth(options: RadiusOAuthOptions) RadiusOAuth {
    return .{ .options = options };
}
