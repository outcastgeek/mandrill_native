const std = @import("std");
const auth = @import("../types.zig");
const transport = @import("../../transport/http.zig");
const j = @import("../../json/value.zig");
const http = @import("http.zig");
const polling = @import("device_code.zig");
const uri = @import("../../transport/uri.zig");
const client_id = "1031625952748946";
fn request(a: std.mem.Allocator, io: std.Io, network: transport.Transport, url: []const u8, body: []const u8, headers: @import("../../types.zig").ProviderHeaders, body_canceled: ?*bool) !http.Response {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const parsed = try uri.parseHttpUrl(temporary.allocator(), url);
    const started = std.Io.Clock.awake.now(io).toMilliseconds();
    var response = try network.send(a, io, .{ .method = .POST, .uri = parsed, .body = body, .headers = headers, .timeoutMs = 30_000 });
    defer response.deinit();
    const remaining = @max(@as(i128, 0), 30_000 - (std.Io.Clock.awake.now(io).toMilliseconds() - started));
    const wire = response.readAllWithTimeout(a, io, .unlimited, @floatFromInt(remaining)) catch |err| {
        if (err == error.OutOfMemory) return err;
        // readJson catches body rejection; login's outer catch separately
        // observes caller abort even when a later status/schema error is thrown.
        if (err == error.Canceled) if (body_canceled) |slot| {
            slot.* = true;
        };
        return .{ .status = response.status, .raw = "", .body = .null, .jsonError = err };
    };
    defer a.free(wire);
    const text = try http.responseText(a, wire, .fetch);
    errdefer a.free(text);
    const value = j.parseFromSliceLeaky(std.json.Value, a, text, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |err| {
        if (err == error.OutOfMemory) return err;
        return .{ .status = response.status, .raw = text, .body = .null, .jsonError = err };
    };
    return .{ .status = response.status, .raw = text, .body = if (value == .object or value == .array) value else .null };
}
fn formRequest(a: std.mem.Allocator, io: std.Io, network: transport.Transport, url: []const u8, fields: []const uri.Pair, body_canceled: ?*bool) !http.Response {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const encoded = try uri.encodeForm(temporary.allocator(), fields);
    defer std.crypto.secureZero(u8, @constCast(encoded));
    return request(a, io, network, url, encoded, &.{ .{ .name = "content-type", .value = "application/x-www-form-urlencoded" }, .{ .name = "accept", .value = "application/json" } }, body_canceled);
}
fn trusted(a: std.mem.Allocator, raw: []const u8) !?[]const u8 {
    if (try @import("../../schema/formats.zig").canParseAbsoluteUrl(a, raw)) |known| if (!known) return null;
    const url = uri.parseHttpUrl(a, raw) catch |err| {
        if (err == error.OutOfMemory) return err;
        return null;
    };
    return if (std.ascii.eqlIgnoreCase(url.scheme, "http") or std.ascii.eqlIgnoreCase(url.scheme, "https")) try uri.render(a, url) else null;
}
fn positive(value: std.json.Value, key: []const u8) ?f64 {
    return http.positiveNumber(value, key) catch null;
}
fn detail(a: std.mem.Allocator, value: std.json.Value) ![]const u8 {
    for ([_][]const u8{ "error_description", "detail", "message", "error" }) |key| if (j.field(value, key)) |text| {
        const trimmed = try @import("../../utils/unicode.zig").trimWhitespace(a, text);
        defer a.free(trimmed);
        if (trimmed.len > 0) return std.fmt.allocPrint(a, ": {s}", .{trimmed});
    };
    return "";
}
fn responseJson(response: http.Response) std.json.Value {
    return if (response.jsonError == null and (response.body == .object or response.body == .array)) response.body else .null;
}
fn mint(a: std.mem.Allocator, io: std.Io, network: transport.Transport, identity: ?std.json.Value, error_message: ?*?[]const u8, body_canceled: ?*bool) !auth.OAuthCredential {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const encoded = try j.stringify(temporary.allocator(), j.map(temporary.allocator()), .{});
    const identity_text = @import("../../utils/diagnostics.zig").formatThrownValue(temporary.allocator(), if (identity) |value| .{ .value = value } else .undefined_value) catch |err| {
        if (err == error.CannotConvertObjectToPrimitive) http.remember(error_message, "Cannot convert object to primitive value");
        return err;
    };
    defer if (identity != null) std.crypto.secureZero(u8, @constCast(identity_text));
    const authorization = try std.fmt.allocPrint(temporary.allocator(), "Bearer {s}", .{identity_text});
    defer std.crypto.secureZero(u8, authorization);
    const response = try request(a, io, network, "https://api.meta.ai/muse-code/key", encoded, &.{ .{ .name = "accept", .value = "application/json" }, .{ .name = "Authorization", .value = authorization }, .{ .name = "content-type", .value = "application/json" }, .{ .name = "x-api-version", .value = "1.0.0" } }, body_canceled);
    if (response.status == 401 or response.status == 403) {
        http.remember(error_message, try std.fmt.allocPrint(a, "Meta session expired (status {d}). Run `/login meta` to sign in again.{s}", .{ response.status, try detail(a, response.body) }));
        return error.MetaSessionExpired;
    }
    if (!response.ok()) {
        http.remember(error_message, try std.fmt.allocPrint(a, "Meta API key mint failed with status {d}{s}", .{ response.status, try detail(a, response.body) }));
        return error.MetaApiKeyMintFailed;
    }
    const key = http.requiredString(response.body, "api_key") catch |err| {
        const action = if (j.field(response.body, "action_url")) |value| try trusted(a, value) else null;
        http.remember(error_message, if (action) |url| try std.fmt.allocPrint(a, "Meta did not issue an API key. Complete setup at {s}", .{url}) else "Meta did not issue an API key.");
        return err;
    };
    var result = auth.OAuthCredential{ .refresh = if (identity) |value| j.text(value) orelse "" else "", .access = key, .expires = @import("../../utils/datetime.zig").expiresAfterNumber(io, 86400) };
    // The header template coerces identityToken, while the returned credential
    // retains the original property value without applying that conversion.
    if (identity) |value| try j.retainField(a, &result, "refresh", value) else j.markAbsent(&result, "refresh");
    return result;
}
const Poll = struct {
    allocator: std.mem.Allocator,
    network: transport.Transport,
    device: []const u8,
    body_canceled: ?*bool = null,
    fn call(ctx: ?*anyopaque, io: std.Io) !polling.PollResult([]const u8) {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        const response = try formRequest(self.allocator, io, self.network, "https://auth.meta.com/oidc/device/token/", &.{ .{ .name = "grant_type", .value = "urn:ietf:params:oauth:grant-type:device_code" }, .{ .name = "device_code", .value = self.device }, .{ .name = "client_id", .value = client_id } }, self.body_canceled);
        if (response.ok()) if (j.field(response.body, "access_token")) |token| if (token.len > 0) return .{ .complete = token };
        const err = j.field(response.body, "error") orelse "";
        if (std.mem.eql(u8, err, "authorization_pending")) return .pending;
        if (std.mem.eql(u8, err, "slow_down")) return .{ .slow_down = positive(response.body, "interval") };
        if (std.mem.eql(u8, err, "access_denied")) return .{ .failed = "Meta login was denied." };
        if (std.mem.eql(u8, err, "expired_token")) return .{ .failed = "Meta device authorization expired. Please restart login." };
        return .{ .failed = try std.fmt.allocPrint(self.allocator, "Meta device token request failed with status {d}{s}", .{ response.status, try detail(self.allocator, responseJson(response)) }) };
    }
};
fn login(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, interaction: auth.Interaction, options: auth.LoginOptions) !auth.OAuthCredential {
    var body_canceled = false;
    return loginInner(ctx, a, io, network, interaction, options, &body_canceled) catch |err| {
        if (err == error.OutOfMemory) return err;
        if (err == error.Canceled or body_canceled) {
            http.remember(interaction.errorMessage, "Login cancelled");
            return error.Canceled;
        }
        io.checkCancel() catch |cancel| {
            http.remember(interaction.errorMessage, "Login cancelled");
            return cancel;
        };
        return err;
    };
}
fn loginInner(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, interaction: auth.Interaction, _: auth.LoginOptions, body_canceled: *bool) !auth.OAuthCredential {
    const response = try formRequest(a, io, network, "https://auth.meta.com/oidc/device/authorization/", &.{.{ .name = "client_id", .value = client_id }}, body_canceled);
    if (!response.ok()) {
        http.remember(interaction.errorMessage, try std.fmt.allocPrint(a, "Meta device authorization failed with status {d}{s}", .{ response.status, try detail(a, response.body) }));
        return error.MetaDeviceAuthorizationFailed;
    }
    const device = j.field(response.body, "device_code");
    const user = j.field(response.body, "user_code");
    const complete = if (j.field(response.body, "verification_uri_complete")) |raw| try trusted(a, raw) else null;
    const verification = complete orelse (if (j.field(response.body, "verification_uri")) |raw| try trusted(a, raw) else null);
    if (device == null or device.?.len == 0 or user == null or user.?.len == 0 or verification == null) {
        http.remember(interaction.errorMessage, try std.fmt.allocPrint(a, "Invalid Meta device authorization response: {s}", .{try j.stringify(a, responseJson(response), .{})}));
        return error.InvalidOAuthResponse;
    }
    const interval = positive(response.body, "interval");
    const lifetime = positive(response.body, "expires_in");
    var notification = try j.from(a, .{ .type = "device_code", .userCode = user.?, .verificationUri = verification.? });
    if (interval) |value| try j.put(a, &notification, "intervalSeconds", j.num(value));
    if (lifetime) |value| try j.put(a, &notification, "expiresInSeconds", j.num(value));
    try interaction.notify(interaction.context, io, notification);
    var poll = Poll{ .allocator = a, .network = network, .device = device.?, .body_canceled = body_canceled };
    const identity = try polling.pollOAuthDeviceCodeFlow([]const u8, io, .{ .context = &poll, .intervalSeconds = interval, .expiresInSeconds = lifetime, .waitBeforeFirstPoll = true, .poll = Poll.call, .errorMessage = interaction.errorMessage });
    try interaction.notify(interaction.context, io, try j.from(a, .{ .type = "progress", .message = "Enabling Meta Model API access..." }));
    return mint(a, io, network, j.str(identity), interaction.errorMessage, body_canceled);
}
fn refresh(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, credential: auth.OAuthCredential) !auth.OAuthCredential {
    return refreshWithError(null, a, io, network, credential, null);
}
fn refreshWithError(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, credential: auth.OAuthCredential, error_message: ?*?[]const u8) !auth.OAuthCredential {
    return mint(a, io, network, try j.fieldValue(a, credential, "refresh"), error_message, null);
}
pub const metaOAuth = auth.OAuthAuth{ .name = "Meta (Muse subscription)", .isSubscription = true, .loginLabel = "Sign in with Meta", .login = login, .refresh = refresh, .refreshWithError = refreshWithError, .toAuth = http.auth };
