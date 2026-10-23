const std = @import("std");
const auth = @import("../types.zig");
const transport = @import("../../transport/http.zig");
const j = @import("../../json/value.zig");
const http = @import("http.zig");
const polling = @import("device_code.zig");
const client_id = "b1a00492-073a-47ea-816f-4c329264a828";
const token_url = "https://auth.x.ai/oauth2/token";
fn post(a: std.mem.Allocator, io: std.Io, network: transport.Transport, url: []const u8, fields: []const @import("../../transport/uri.zig").Pair, error_message: ?*?[]const u8) !http.Response {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const uri = @import("../../transport/uri.zig");
    const parsed = try uri.parseHttpUrl(temporary.allocator(), url);
    const encoded = try uri.encodeForm(temporary.allocator(), fields);
    defer std.crypto.secureZero(u8, @constCast(encoded));
    var response = network.send(a, io, .{ .method = .POST, .uri = parsed, .body = encoded, .headers = &.{ .{ .name = "accept", .value = "application/json" }, .{ .name = "content-type", .value = "application/x-www-form-urlencoded" } } }) catch |err| {
        if (err == error.OutOfMemory) return err;
        if (err == error.Canceled) {
            http.remember(error_message, "Login cancelled");
            return err;
        }
        io.checkCancel() catch |cancel| {
            http.remember(error_message, "Login cancelled");
            return cancel;
        };
        return err;
    };
    defer response.deinit();
    const wire = response.readAll(a, .unlimited) catch |err| {
        if (err == error.OutOfMemory) return err;
        if (err == error.Canceled) {
            http.remember(error_message, "Login cancelled");
            return err;
        }
        io.checkCancel() catch |cancel| {
            http.remember(error_message, "Login cancelled");
            return cancel;
        };
        http.remember(error_message, try std.fmt.allocPrint(a, "xAI OAuth returned invalid JSON (HTTP {d})", .{response.status}));
        return error.InvalidOAuthResponse;
    };
    defer a.free(wire);
    const text = try http.responseText(a, wire, .fetch);
    errdefer a.free(text);
    const value = j.parseFromSliceLeaky(std.json.Value, a, text, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |err| {
        if (err == error.OutOfMemory) return err;
        io.checkCancel() catch |cancel| {
            http.remember(error_message, "Login cancelled");
            return cancel;
        };
        http.remember(error_message, try std.fmt.allocPrint(a, "xAI OAuth returned invalid JSON (HTTP {d})", .{response.status}));
        return error.InvalidOAuthResponse;
    };
    return .{ .status = response.status, .raw = text, .body = if (value == .object) value else j.map(a) };
}
fn requestFailure(a: std.mem.Allocator, action: []const u8, response: http.Response) ![]const u8 {
    var parts: std.ArrayList([]const u8) = .empty;
    for ([_][]const u8{ "error", "error_description" }) |key| if (j.field(response.body, key)) |text| if (text.len > 0) try parts.append(a, text);
    const detail = if (parts.items.len > 0) try std.fmt.allocPrint(a, ": {s}", .{try std.mem.join(a, ": ", parts.items)}) else "";
    return std.fmt.allocPrint(a, "xAI OAuth {s} failed (HTTP {d}){s}", .{ action, response.status, detail });
}
fn required(a: std.mem.Allocator, body: std.json.Value, key: []const u8, error_message: ?*?[]const u8) ![]const u8 {
    return http.requiredString(body, key) catch |err| {
        http.remember(error_message, try std.fmt.allocPrint(a, "Invalid xAI OAuth response field: {s}", .{key}));
        return err;
    };
}
fn positive(a: std.mem.Allocator, body: std.json.Value, key: []const u8, error_message: ?*?[]const u8) !f64 {
    return http.positiveNumber(body, key) catch |err| {
        http.remember(error_message, try std.fmt.allocPrint(a, "Invalid xAI OAuth response field: {s}", .{key}));
        return err;
    };
}
fn credential(a: std.mem.Allocator, io: std.Io, body: std.json.Value, previous: ?std.json.Value, error_message: ?*?[]const u8) !auth.OAuthCredential {
    const access = try required(a, body, "access_token", error_message);
    const refreshed_token = if (j.get(body, "refresh_token") == null and previous != null and j.truthy(previous.?)) previous.? else j.str(try required(a, body, "refresh_token", error_message));
    const expires = if (j.get(body, "expires_in") != null) try positive(a, body, "expires_in", error_message) else 3600;
    var result = auth.OAuthCredential{ .access = access, .refresh = j.text(refreshed_token) orelse "", .expires = @import("../../utils/datetime.zig").expiresAfterNumber(io, expires) - 300_000 };
    try j.retainField(a, &result, "refresh", refreshed_token);
    return result;
}
fn trustedUri(a: std.mem.Allocator, raw: []const u8, error_message: ?*?[]const u8) ![]const u8 {
    if (try @import("../../schema/formats.zig").canParseAbsoluteUrl(a, raw)) |known| if (!known) {
        http.remember(error_message, "Untrusted verification URI in xAI OAuth response");
        return error.UntrustedVerificationUri;
    };
    const url = @import("../../transport/uri.zig");
    const uri = url.parseHttpUrl(a, raw) catch |err| {
        if (err == error.OutOfMemory) return err;
        http.remember(error_message, "Untrusted verification URI in xAI OAuth response");
        return error.UntrustedVerificationUri;
    };
    if (!std.ascii.eqlIgnoreCase(uri.scheme, "https")) {
        http.remember(error_message, "Untrusted verification URI in xAI OAuth response");
        return error.UntrustedVerificationUri;
    }
    return url.render(a, uri);
}
const Poll = struct {
    allocator: std.mem.Allocator,
    transport: transport.Transport,
    device_code: []const u8,
    errorMessage: ?*?[]const u8 = null,
    fn call(ctx: ?*anyopaque, io: std.Io) !polling.PollResult(auth.OAuthCredential) {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        const response = try post(self.allocator, io, self.transport, token_url, &.{ .{ .name = "grant_type", .value = "urn:ietf:params:oauth:grant-type:device_code" }, .{ .name = "client_id", .value = client_id }, .{ .name = "device_code", .value = self.device_code } }, self.errorMessage);
        if (response.ok()) return .{ .complete = try credential(self.allocator, io, response.body, null, self.errorMessage) };
        const err = j.field(response.body, "error") orelse return .{ .failed = try requestFailure(self.allocator, "device token polling", response) };
        if (std.mem.eql(u8, err, "authorization_pending")) return .pending;
        if (std.mem.eql(u8, err, "slow_down")) return .{ .slow_down = if (j.get(response.body, "interval")) |v| j.number(v) else null };
        if (std.mem.eql(u8, err, "access_denied") or std.mem.eql(u8, err, "authorization_denied")) return .{ .failed = "xAI device authorization was denied" };
        if (std.mem.eql(u8, err, "expired_token")) return .{ .failed = "xAI device code expired" };
        return .{ .failed = try requestFailure(self.allocator, "device token polling", response) };
    }
};
fn login(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, interaction: auth.Interaction, _: auth.LoginOptions) !auth.OAuthCredential {
    const response = try post(a, io, network, "https://auth.x.ai/oauth2/device/code", &.{ .{ .name = "client_id", .value = client_id }, .{ .name = "scope", .value = "openid profile email offline_access grok-cli:access api:access" }, .{ .name = "referrer", .value = "pi" } }, interaction.errorMessage);
    if (!response.ok()) {
        http.remember(interaction.errorMessage, try requestFailure(a, "device authorization", response));
        return error.XaiDeviceAuthorizationFailed;
    }
    // The pinned parser validates the optional complete URI before required fields.
    const complete = if (j.field(response.body, "verification_uri_complete")) |v| (if (v.len > 0) try trustedUri(a, v, interaction.errorMessage) else null) else null;
    const device = try required(a, response.body, "device_code", interaction.errorMessage);
    const user = try required(a, response.body, "user_code", interaction.errorMessage);
    const verification = try trustedUri(a, try required(a, response.body, "verification_uri", interaction.errorMessage), interaction.errorMessage);
    const lifetime = try positive(a, response.body, "expires_in", interaction.errorMessage);
    const raw_interval = if (j.get(response.body, "interval")) |v| j.number(v) else null;
    const interval = if (raw_interval != null and std.math.isFinite(raw_interval.?) and raw_interval.? > 0) raw_interval else null;
    var notification = try j.from(a, .{ .type = "device_code", .userCode = user, .verificationUri = complete orelse verification, .expiresInSeconds = lifetime });
    if (interval) |value| try j.put(a, &notification, "intervalSeconds", j.num(value));
    try interaction.notify(interaction.context, io, notification);
    var poll = Poll{ .allocator = a, .transport = network, .device_code = device, .errorMessage = interaction.errorMessage };
    return polling.pollOAuthDeviceCodeFlow(auth.OAuthCredential, io, .{ .context = &poll, .intervalSeconds = interval, .expiresInSeconds = lifetime, .waitBeforeFirstPoll = true, .poll = Poll.call, .errorMessage = interaction.errorMessage });
}
fn refresh(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, previous: auth.OAuthCredential) !auth.OAuthCredential {
    return refreshWithError(null, a, io, network, previous, null);
}
fn refreshWithError(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, previous: auth.OAuthCredential, error_message: ?*?[]const u8) !auth.OAuthCredential {
    const raw_refresh = try j.fieldValue(a, previous, "refresh");
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const text = @import("../../utils/diagnostics.zig").formatThrownValue(temporary.allocator(), if (raw_refresh) |value| .{ .value = value } else .undefined_value) catch |err| {
        if (err == error.OutOfMemory) return err;
        io.checkCancel() catch |cancel| {
            http.remember(error_message, "Login cancelled");
            return cancel;
        };
        if (err == error.CannotConvertObjectToPrimitive) http.remember(error_message, "Cannot convert object to primitive value");
        return err;
    };
    defer if (raw_refresh != null) std.crypto.secureZero(u8, @constCast(text));
    const response = try post(a, io, network, token_url, &.{ .{ .name = "grant_type", .value = "refresh_token" }, .{ .name = "client_id", .value = client_id }, .{ .name = "refresh_token", .value = text } }, error_message);
    if (!response.ok()) {
        http.remember(error_message, try requestFailure(a, "token refresh", response));
        return error.XaiTokenRefreshFailed;
    }
    return credential(a, io, response.body, raw_refresh, error_message);
}
pub const xaiOAuth = auth.OAuthAuth{ .name = "xAI (Grok/X subscription)", .isSubscription = true, .loginLabel = "Sign in with SuperGrok or X Premium", .login = login, .refresh = refresh, .refreshWithError = refreshWithError, .toAuth = http.auth };
