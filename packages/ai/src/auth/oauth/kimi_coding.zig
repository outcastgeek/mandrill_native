const std = @import("std");
const auth = @import("../types.zig");
const transport = @import("../../transport/http.zig");
const j = @import("../../json/value.zig");
const uri = @import("../../transport/uri.zig");
const http = @import("http.zig");
const polling = @import("device_code.zig");
const client_id = "17e5f671-d194-4dfb-9706-5516cb48c098";
pub const Configuration = struct { host: []const u8 = "https://auth.kimi.com" };
fn host(ctx: ?*anyopaque) []const u8 {
    return if (ctx) |p| @as(*Configuration, @ptrCast(@alignCast(p))).host else "https://auth.kimi.com";
}
fn endpoint(a: std.mem.Allocator, base: []const u8, path: []const u8) ![]const u8 {
    return std.mem.concat(a, u8, &.{ std.mem.trimEnd(u8, base, "/"), path });
}
fn tokenUrl(a: std.mem.Allocator, base: []const u8) ![]const u8 {
    return endpoint(a, base, "/api/oauth/token");
}
fn formRequest(a: std.mem.Allocator, io: std.Io, network: transport.Transport, url: []const u8, fields: []const uri.Pair, body_canceled: ?*bool) !http.Response {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    const parsed = try uri.parseHttpUrl(scratch, url);
    const body = try uri.encodeForm(scratch, fields);
    defer std.crypto.secureZero(u8, @constCast(body));
    const started = std.Io.Clock.awake.now(io).toMilliseconds();
    var response = try network.send(a, io, .{ .method = .POST, .uri = parsed, .body = body, .headers = &.{ .{ .name = "accept", .value = "application/json" }, .{ .name = "content-type", .value = "application/x-www-form-urlencoded" } }, .timeoutMs = 30_000 });
    defer response.deinit();
    const remaining = @max(@as(i128, 0), 30_000 - (std.Io.Clock.awake.now(io).toMilliseconds() - started));
    const wire = response.readAllWithTimeout(a, io, .unlimited, @floatFromInt(remaining)) catch |err| {
        if (err == error.OutOfMemory) return err;
        // Both readJson and the non-success response.text branch catch body
        // rejection after Fetch resolves headers, including body aborts.
        // Keep the observation locally: a host/body-task Canceled does not
        // prove the current Io task may safely call recancel.
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
    return .{ .status = response.status, .raw = text, .body = value };
}
fn trusted(a: std.mem.Allocator, raw: []const u8) !void {
    if (try @import("../../schema/formats.zig").canParseAbsoluteUrl(a, raw)) |known| if (!known) return error.UntrustedVerificationUri;
    const parsed = uri.parseHttpUrl(a, raw) catch |err| {
        if (err == error.OutOfMemory) return err;
        return error.UntrustedVerificationUri;
    };
    if (!std.ascii.eqlIgnoreCase(parsed.scheme, "http") and !std.ascii.eqlIgnoreCase(parsed.scheme, "https")) return error.UntrustedVerificationUri;
}
fn responseJson(response: http.Response) std.json.Value {
    return if (response.jsonError == null and (response.body == .object or response.body == .array)) response.body else .null;
}
fn missingFields(a: std.mem.Allocator, response: http.Response, operation: []const u8) ![]const u8 {
    return std.fmt.allocPrint(a, "Kimi Code token {s} response missing fields: {s}", .{ operation, try j.stringify(a, responseJson(response), .{}) });
}
const Poll = struct {
    allocator: std.mem.Allocator,
    network: transport.Transport,
    token_url: []const u8,
    device: []const u8,
    fn call(ctx: ?*anyopaque, io: std.Io) !polling.PollResult(auth.OAuthCredential) {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        const response = try formRequest(self.allocator, io, self.network, self.token_url, &.{ .{ .name = "client_id", .value = client_id }, .{ .name = "device_code", .value = self.device }, .{ .name = "grant_type", .value = "urn:ietf:params:oauth:grant-type:device_code" } }, null);
        if (response.status >= 500) return .{ .failed = try std.fmt.allocPrint(self.allocator, "Kimi Code device token request failed with status {d}{s}", .{ response.status, if (response.raw.len > 0) try std.fmt.allocPrint(self.allocator, ": {s}", .{response.raw}) else "" }) };
        if (response.ok() and j.field(response.body, "access_token") != null) return .{ .complete = http.credential(io, response.body, null, 0, null) catch |err| {
            if (err == error.InvalidOAuthResponse) return .{ .failed = try missingFields(self.allocator, response, "poll") };
            return err;
        } };
        const err = j.field(response.body, "error") orelse "";
        if (std.mem.eql(u8, err, "authorization_pending")) return .pending;
        if (std.mem.eql(u8, err, "slow_down")) return .{ .slow_down = if (j.get(response.body, "interval")) |v| j.number(v) else null };
        if (std.mem.eql(u8, err, "expired_token")) return .{ .failed = "Kimi Code device authorization expired. Please restart login." };
        if (std.mem.eql(u8, err, "access_denied")) return .{ .failed = "Kimi Code login was denied." };
        const suffix = if (j.field(response.body, "error")) |code| try std.fmt.allocPrint(self.allocator, ": {s}{s}", .{ code, if (j.field(response.body, "error_description")) |description| try std.fmt.allocPrint(self.allocator, ": {s}", .{description}) else "" }) else "";
        return .{ .failed = try std.fmt.allocPrint(self.allocator, "Kimi Code device token request failed (status {d}){s}", .{ response.status, suffix }) };
    }
};
fn login(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, interaction: auth.Interaction, options: auth.LoginOptions) !auth.OAuthCredential {
    var base = host(ctx);
    for ([_][]const u8{ "KIMI_CODE_OAUTH_HOST", "KIMI_OAUTH_HOST" }) |name| if (options.env.get(name)) |value| if (value.len > 0) {
        base = value;
        break;
    };
    const response = try formRequest(a, io, network, try endpoint(a, base, "/api/oauth/device_authorization"), &.{.{ .name = "client_id", .value = client_id }}, null);
    if (!response.ok()) {
        http.remember(interaction.errorMessage, try std.fmt.allocPrint(a, "Kimi Code device authorization failed with status {d}{s}", .{ response.status, if (response.raw.len > 0) try std.fmt.allocPrint(a, ": {s}", .{response.raw}) else "" }));
        return error.KimiDeviceAuthorizationFailed;
    }
    // Unlike token fields, empty device and user codes are accepted by the pin.
    const device = j.field(response.body, "device_code");
    const user = j.field(response.body, "user_code");
    const plain = j.field(response.body, "verification_uri");
    const verification = j.field(response.body, "verification_uri_complete");
    const valid_urls = if (plain != null and verification != null) valid: {
        trusted(a, verification.?) catch |err| {
            if (err == error.OutOfMemory) return err;
            break :valid false;
        };
        trusted(a, plain.?) catch |err| {
            if (err == error.OutOfMemory) return err;
            break :valid false;
        };
        break :valid true;
    } else false;
    if (device == null or user == null or !valid_urls) {
        http.remember(interaction.errorMessage, try std.fmt.allocPrint(a, "Invalid Kimi Code device authorization response: {s}", .{try j.stringify(a, responseJson(response), .{})}));
        return error.InvalidOAuthResponse;
    }
    const interval = http.positiveNumber(response.body, "interval") catch 5;
    const lifetime = http.positiveNumber(response.body, "expires_in") catch 900;
    try interaction.notify(interaction.context, io, try j.from(a, .{ .type = "device_code", .userCode = user.?, .verificationUri = verification.?, .intervalSeconds = interval, .expiresInSeconds = lifetime }));
    var poll = Poll{ .allocator = a, .network = network, .token_url = try tokenUrl(a, base), .device = device.? };
    return polling.pollOAuthDeviceCodeFlow(auth.OAuthCredential, io, .{ .context = &poll, .intervalSeconds = interval, .expiresInSeconds = lifetime, .waitBeforeFirstPoll = true, .poll = Poll.call, .errorMessage = interaction.errorMessage });
}
fn refresh(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, credential: auth.OAuthCredential) !auth.OAuthCredential {
    return refreshWithError(ctx, a, io, network, credential, null);
}
fn refreshWithError(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, credential: auth.OAuthCredential, error_message: ?*?[]const u8) !auth.OAuthCredential {
    return refreshWithContext(ctx, a, io, network, credential, null, error_message);
}
fn refreshRequest(a: std.mem.Allocator, io: std.Io, network: transport.Transport, url: []const u8, refresh_value: ?std.json.Value, body_canceled: *bool) !http.Response {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    // URLSearchParams converts the captured property during each fetch attempt.
    const text = try @import("../../utils/diagnostics.zig").formatThrownValue(temporary.allocator(), if (refresh_value) |value| .{ .value = value } else .undefined_value);
    defer if (refresh_value != null) std.crypto.secureZero(u8, @constCast(text));
    return formRequest(a, io, network, url, &.{ .{ .name = "client_id", .value = client_id }, .{ .name = "grant_type", .value = "refresh_token" }, .{ .name = "refresh_token", .value = text } }, body_canceled);
}
fn refreshWithContext(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, credential: auth.OAuthCredential, context: ?auth.AuthContext, error_message: ?*?[]const u8) !auth.OAuthCredential {
    var base = host(ctx);
    for ([_][]const u8{ "KIMI_CODE_OAUTH_HOST", "KIMI_OAUTH_HOST" }) |name| if (try @import("../../utils/provider_env.zig").getProviderEnvValue(a, io, context, name, .{})) |value| {
        base = value;
        break;
    };
    const refresh_value = try j.fieldValue(a, credential, "refresh");
    const url = try tokenUrl(a, base);
    var attempt: u3 = 0;
    var body_canceled = false;
    while (attempt <= 3) : (attempt += 1) {
        if (attempt > 0) {
            // The source's abort signal stays aborted after readJson catches
            // body rejection, so its next backoff rejects before another send.
            if (body_canceled) return error.Canceled;
            try io.sleep(.fromMilliseconds(@as(i64, 1000) << @as(u6, attempt - 1)), .awake);
        }
        io.checkCancel() catch |err| {
            http.remember(error_message, "Kimi Code token refresh aborted");
            return err;
        };
        const response = refreshRequest(a, io, network, url, refresh_value, &body_canceled) catch |err| {
            if (err == error.Canceled or err == error.OutOfMemory) return err;
            if (attempt < 3) continue;
            if (err == error.RequestTimeout) http.remember(error_message, "The operation was aborted due to timeout");
            if (err == error.CannotConvertObjectToPrimitive) http.remember(error_message, "Cannot convert object to primitive value");
            return err;
        };
        if (response.ok()) return http.credential(io, response.body, null, 0, null) catch |err| {
            if (err == error.InvalidOAuthResponse) http.remember(error_message, try missingFields(a, response, "refresh"));
            return err;
        };
        if (response.status == 401 or response.status == 403 or std.mem.eql(u8, j.field(response.body, "error") orelse "", "invalid_grant")) {
            const description = if (j.field(response.body, "error_description")) |text| try std.fmt.allocPrint(a, ": {s}", .{text}) else "";
            http.remember(error_message, try std.fmt.allocPrint(a, "Kimi Code token refresh unauthorized (status {d}){s}", .{ response.status, description }));
            return error.KimiTokenRefreshUnauthorized;
        }
        if ((response.status == 429 or response.status >= 500) and attempt < 3) continue;
        http.remember(error_message, try std.fmt.allocPrint(a, "Kimi Code token refresh failed with status {d}: {s}", .{ response.status, if (response.jsonError != null or (response.body != .object and response.body != .array)) "null" else try @import("../../json/value.zig").stringify(a, response.body, .{}) }));
        return error.KimiTokenRefreshFailed;
    }
    return error.KimiTokenRefreshFailed;
}
fn toAuth(_: ?*anyopaque, a: std.mem.Allocator, _: std.Io, credential: auth.OAuthCredential) !auth.ModelAuth {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const value = try j.fieldValue(temporary.allocator(), credential, "access");
    const text = try @import("../../utils/diagnostics.zig").formatThrownValue(temporary.allocator(), if (value) |raw| .{ .value = raw } else .undefined_value);
    defer if (value != null) std.crypto.secureZero(u8, @constCast(text));
    return .{ .headers = try a.dupe(@import("../../types.zig").ProviderHeader, &.{.{ .name = "Authorization", .value = try std.fmt.allocPrint(a, "Bearer {s}", .{text}) }}) };
}
pub const kimiCodingOAuth = auth.OAuthAuth{ .name = "Kimi Code (subscription)", .isSubscription = true, .loginLabel = "Sign in with Kimi Code", .login = login, .refresh = refresh, .refreshWithError = refreshWithError, .refreshWithContext = refreshWithContext, .toAuth = toAuth };
pub fn configured(configuration: *Configuration) auth.OAuthAuth {
    var flow = kimiCodingOAuth;
    flow.context = configuration;
    return flow;
}
