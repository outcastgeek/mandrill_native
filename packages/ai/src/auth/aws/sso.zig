const std = @import("std");
const m = @import("../../models.zig");
const t = @import("../../types.zig");
const j = @import("../../json/value.zig");
const shared = @import("shared_config.zig");
const http = @import("../oauth/http.zig");
const aws_http = @import("json_http.zig");
const uri = @import("../../transport/uri.zig");
const datetime = @import("../../utils/datetime.zig");
const Credentials = @import("credentials.zig").Credentials;
const refresh_message = "To refresh this SSO session run 'aws sso login' with the corresponding profile.";
const credential_refresh_message = "To refresh this SSO session run aws sso login with the corresponding profile.";
fn nowMilliseconds(io: std.Io) f64 {
    return @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds());
}
fn dateValue(operation: m.OperationContext, value: ?std.json.Value) !f64 {
    return datetime.parseJavaScriptDateValue(operation.allocator, operation.io, value) catch |err| {
        if (err == error.CannotConvertObjectToPrimitive) if (operation.errorMessage) |slot| {
            slot.* = "Cannot convert object to primitive value";
        };
        return err;
    };
}
fn stringSchemaText(a: std.mem.Allocator, value: std.json.Value) ![]u8 {
    return switch (value) {
        .string => |text| try a.dupe(u8, text),
        .object, .array => try j.stringify(a, value, .{ .whitespace = .indent_2 }),
        else => @constCast(try @import("../../utils/diagnostics.zig").formatThrownValue(a, .{ .value = value })),
    };
}
fn roleTokenText(operation: m.OperationContext, value: ?std.json.Value) !?[]u8 {
    const selected = value orelse return null;
    if (selected == .null) return null;
    const a = operation.allocator;
    const text = try stringSchemaText(a, selected);
    errdefer {
        std.crypto.secureZero(u8, text);
        a.free(text);
    }
    var invalid = try @import("../../regex/regexp.zig").Regex.init(a, operation.io, "[^\\t\\x20-\\x7e\\x80-\\xff]", .{});
    defer invalid.deinit();
    if (try invalid.execSensitive(a, text, 0)) |found| {
        var match = found;
        defer match.deinit();
        return fail(operation, error.InvalidAwsSsoAccessTokenHeader, "Invalid character in header content [\"x-amz-sso_bearer_token\"]", .{});
    }
    return text;
}
fn fail(operation: m.OperationContext, err: anyerror, comptime format: []const u8, args: anytype) anyerror {
    if (operation.errorMessage) |slot| slot.* = std.fmt.allocPrint(operation.allocator, format, args) catch return error.OutOfMemory;
    return err;
}
fn requiredTokenKey(operation: m.OperationContext, value: std.json.Value, key: []const u8, refreshing: bool) !void {
    if (value == .null) return fail(operation, error.AwsSsoSessionFailed, "Cannot read properties of null (reading '{s}')", .{key});
    if (j.get(value, key) == null) return fail(operation, error.AwsSsoRefreshPropertyMissing, "Value not present for '{s}' in SSO Token{s}. {s}", .{ key, if (refreshing) ". Cannot refresh" else "", refresh_message });
}
fn invalidProfile(operation: m.OperationContext, files: shared.Files, profile: []const u8, session: bool) anyerror {
    const names = files.profilePropertyNames(profile, if (session) &.{ "sso_region", "sso_start_url" } else &.{}) catch |err| return err;
    defer operation.allocator.free(names);
    const keys = std.mem.join(operation.allocator, ", ", names) catch |err| return err;
    defer operation.allocator.free(keys);
    return fail(operation, error.InvalidAwsSsoProfile, "Profile is configured with invalid SSO credentials. Required parameters \"sso_account_id\", \"sso_region\", \"sso_role_name\", \"sso_start_url\". Got {s}\nReference: https://docs.aws.amazon.com/cli/latest/userguide/cli-configure-sso.html", .{keys});
}
fn profileKey(operation: m.OperationContext, value: std.json.Value) ![]const u8 {
    return @import("../../utils/diagnostics.zig").formatThrownValue(operation.allocator, .{ .value = value }) catch |err| {
        if (err == error.CannotConvertObjectToPrimitive) if (operation.errorMessage) |slot| {
            slot.* = "Cannot convert object to primitive value";
        };
        return err;
    };
}
fn regionText(operation: m.OperationContext, value: std.json.Value) ![]const u8 {
    if (value == .string) return value.string;
    // Smithy's region provider checks the retained value before the endpoint
    // UseFIPS provider attempts to call a non-string configured region.
    const text = profileKey(operation, value) catch |err| {
        if (err == error.CannotConvertObjectToPrimitive) if (operation.errorMessage) |slot| {
            slot.* = "TypeError: Cannot convert object to primitive value";
        };
        return err;
    };
    @import("region.zig").checkRegion(operation, text, false) catch |err| {
        if (err == error.InvalidAwsRegion) if (operation.errorMessage) |slot| if (slot.*) |detail| {
            slot.* = try std.fmt.allocPrint(operation.allocator, "Error: {s}", .{detail});
        };
        return err;
    };
    return fail(operation, error.AwsSsoRegionTypeError, "TypeError: region is not a function", .{});
}
fn serviceEndpoint(operation: m.OperationContext, options: t.StreamOptions, service: @import("endpoints.zig").Service, region: std.json.Value) ![]const u8 {
    const text = try regionText(operation, region);
    return @import("endpoints.zig").resolve(operation, options, service, text) catch |err| {
        // SSO's send catch passes the Error object to CredentialsProviderError,
        // whose message String conversion retains its Error name prefix.
        if (err == error.InvalidAwsRegion) if (operation.errorMessage) |slot| if (slot.*) |detail| {
            slot.* = try std.fmt.allocPrint(operation.allocator, "Error: {s}", .{detail});
        };
        return err;
    };
}
pub fn resolve(operation: m.OperationContext, options: t.StreamOptions, files: shared.Files, profile: []const u8) !Credentials {
    const a = operation.allocator;
    const configured_session = (try files.getValue(profile, "sso_session")) orelse .null;
    const session: ?[]const u8 = if (j.truthy(configured_session)) try profileKey(operation, configured_session) else null;
    var sso_region = (try files.getValue(profile, "sso_region")) orelse .null;
    var start_url = (try files.getValue(profile, "sso_start_url")) orelse .null;
    if (session) |name| {
        const selected = if (files.config) |config| j.get(config.sessions, name) else null;
        // The loader reduces into an ordinary object. Its inherited members
        // have no consumed SSO fields, rather than being absent map entries.
        const record = selected orelse if (shared.inheritedObjectName(name)) j.map(a) else return fail(operation, error.InvalidAwsSsoProfile, "Cannot read properties of undefined (reading 'sso_region')", .{});
        if (record == .null) return fail(operation, error.InvalidAwsSsoProfile, "Cannot read properties of null (reading 'sso_region')", .{});
        sso_region = j.get(record, "sso_region") orelse .null;
        start_url = j.get(record, "sso_start_url") orelse .null;
    }
    const raw_account = (try files.getValue(profile, "sso_account_id")) orelse .null;
    const raw_role = (try files.getValue(profile, "sso_role_name")) orelse .null;
    if (!j.truthy(sso_region) or !j.truthy(start_url) or !j.truthy(raw_account) or !j.truthy(raw_role)) return invalidProfile(operation, files, profile, session != null);
    // Cache hashing accepts a string identifier, without String coercion.
    // Non-string retained identifiers fail inside the source token-load catch.
    if (session != null and configured_session != .string) return fail(operation, error.AwsSsoSessionFailed, "The SSO session token associated with profile={s} was not found or is invalid. {s}", .{ profile, refresh_message });
    if (session == null and start_url != .string) return fail(operation, error.AwsSsoSessionFailed, "The SSO session associated with this profile is invalid. {s}", .{credential_refresh_message});
    const token_id = session orelse start_url.string;
    const path = tokenPath(operation, options, token_id) catch |err| blk: {
        if (err != error.AwsHomeDirectoryUnavailable) return err;
        break :blk null;
    };
    const token = readToken(operation, token_id) catch |err| {
        if (err == error.Canceled or err == error.OutOfMemory) return err;
        if (session != null) return fail(operation, error.AwsSsoSessionFailed, "The SSO session token associated with profile={s} was not found or is invalid. {s}", .{ profile, refresh_message });
        return fail(operation, error.AwsSsoSessionFailed, "The SSO session associated with this profile is invalid. {s}", .{credential_refresh_message});
    };
    if (session != null) {
        try requiredTokenKey(operation, token, "accessToken", false);
        try requiredTokenKey(operation, token, "expiresAt", false);
    } else if (token == .null) return fail(operation, error.AwsSsoSessionFailed, "Cannot read properties of null (reading 'expiresAt')", .{});
    var expiry = try dateValue(operation, j.get(token, "expiresAt"));
    var access: ?std.json.Value = j.get(token, "accessToken");
    const now = nowMilliseconds(operation.io);
    if (session != null and !(expiry - now > 300_000)) {
        const ready = if (operation.awsState) |state| try state.ssoRefreshReady(operation.io, session.?) else true;
        if (ready) {
            // Required refresh properties are checked before the SDK's fallback
            // catch; a malformed session does not silently become another source.
            for ([_][]const u8{ "clientId", "clientSecret", "refreshToken" }) |key| try requiredTokenKey(operation, token, key, true);
            const acquired = if (operation.awsState) |state| try state.beginSsoRefresh(operation.io, session.?) else true;
            const refreshed = if (acquired) refresh(operation, options, sso_region, token) catch |err| blk: {
                if (err == error.Canceled or err == error.OutOfMemory) return err;
                if (expiry < nowMilliseconds(operation.io)) return fail(operation, error.ExpiredAwsSsoSession, "Token is expired. {s}", .{refresh_message});
                if (operation.errorMessage) |slot| slot.* = null;
                break :blk null;
            } else null;
            if (refreshed) |value| {
                expiry = value.expiration;
                access = value.access;
                if (value.cache) |record| {
                    // Source cache-write failures are ignored, but a failed
                    // write must not replace the snapshot seen by later loads.
                    const saved = if (path) |output| persisted: {
                        saveToken(operation, output, record) catch |err| {
                            if (err == error.Canceled or err == error.OutOfMemory) return err;
                            break :persisted false;
                        };
                        break :persisted true;
                    } else true;
                    if (saved) if (operation.awsState) |state| try state.putToken(operation.io, try std.fmt.allocPrint(a, "sso:{s}", .{token_id}), record);
                }
            } else if (expiry < nowMilliseconds(operation.io)) return fail(operation, error.ExpiredAwsSsoSession, "Token is expired. {s}", .{refresh_message});
        } else if (expiry < nowMilliseconds(operation.io)) return fail(operation, error.ExpiredAwsSsoSession, "Token is expired. {s}", .{refresh_message});
    }
    // resolveSSOCredentials serializes session-token expiration to ISO before
    // its legacy expiry check; an invalid Date fails at that conversion.
    if (session != null and std.math.isNan(expiry)) return fail(operation, error.AwsSsoSessionFailed, "Invalid time value", .{});
    if (expiry <= nowMilliseconds(operation.io)) return fail(operation, error.ExpiredAwsSsoSession, "The SSO session associated with this profile has expired. {s}", .{credential_refresh_message});
    const endpoint = try serviceEndpoint(operation, options, .sso, sso_region);
    const role = try stringSchemaText(a, raw_role);
    defer {
        std.crypto.secureZero(u8, role);
        a.free(role);
    }
    const account = try stringSchemaText(a, raw_account);
    defer {
        std.crypto.secureZero(u8, account);
        a.free(account);
    }
    var url = try uri.appendPath(a, endpoint, &.{ "federation", "credentials" });
    url.query = .{ .percent_encoded = try uri.encode(a, &.{ .{ .name = "role_name", .value = role }, .{ .name = "account_id", .value = account } }) };
    const token_text = try roleTokenText(operation, access);
    defer if (token_text) |text| {
        std.crypto.secureZero(u8, text);
        a.free(text);
    };
    const token_headers: t.ProviderHeaders = if (token_text) |text| try a.dupe(t.ProviderHeader, &.{.{ .name = "x-amz-sso_bearer_token", .value = text }}) else &.{};
    const response = try aws_http.request(operation, options, .GET, try uri.render(a, url), null, token_headers);
    if (!response.ok()) return error.AwsSsoRoleCredentialsFailed;
    const value = j.get(response.body, "roleCredentials") orelse .null;
    const access_key = j.get(value, "accessKeyId") orelse .null;
    const secret_key = j.get(value, "secretAccessKey") orelse .null;
    const session_token = j.get(value, "sessionToken") orelse .null;
    const raw_expiration = aws_http.numericSchemaValue(j.get(value, "expiration") orelse .null);
    if (!j.truthy(access_key) or !j.truthy(secret_key) or !j.truthy(session_token) or !j.truthy(raw_expiration)) return fail(operation, error.InvalidAwsSsoCredentials, "SSO returns an invalid temporary credential.", .{});
    const expiration = try dateValue(operation, raw_expiration);
    return .{ .accessKeyId = j.text(access_key) orelse "", .secretAccessKey = j.text(secret_key) orelse "", .sessionToken = j.text(session_token), .expiration = if (std.math.isNan(expiration)) null else @intFromFloat(expiration), .expirationInvalid = std.math.isNan(expiration), .invalidSigningKeys = access_key != .string or secret_key != .string, .invalidSigningToken = session_token != .string };
}
const RefreshedToken = struct { access: std.json.Value, expiration: f64, cache: ?std.json.Value };
fn refresh(operation: m.OperationContext, options: t.StreamOptions, region: std.json.Value, previous: std.json.Value) !?RefreshedToken {
    const a = operation.allocator;
    var body = j.map(a);
    for ([_][]const u8{ "clientId", "clientSecret" }) |key| {
        const value = j.get(previous, key) orelse return error.AwsSsoRefreshPropertyMissing;
        if (value != .null) try j.put(a, &body, key, try aws_http.stringSchemaMember(a, value));
    }
    try j.put(a, &body, "grantType", j.str("refresh_token"));
    const refresh_token = j.get(previous, "refreshToken") orelse return error.AwsSsoRefreshPropertyMissing;
    if (refresh_token != .null) try j.put(a, &body, "refreshToken", try aws_http.stringSchemaMember(a, refresh_token));
    const endpoint = try serviceEndpoint(operation, options, .sso_oidc, region);
    var url = try uri.appendPath(a, endpoint, &.{"token"});
    url.query = null;
    const response = try aws_http.request(operation, options, .POST, try uri.render(a, url), body, &.{});
    if (!response.ok()) return error.AwsSsoTokenRefreshFailed;
    // Project only the CreateToken fields consumed below, following the
    // protocol's struct null omission and numeric-schema special strings.
    var decoded = j.map(a);
    for ([_][]const u8{ "accessToken", "expiresIn", "refreshToken" }) |key| if (aws_http.structField(response.body, key)) |value| {
        try j.put(a, &decoded, key, if (std.mem.eql(u8, key, "expiresIn")) aws_http.numericSchemaValue(value) else value);
    };
    try requiredTokenKey(operation, decoded, "accessToken", false);
    try requiredTokenKey(operation, decoded, "expiresIn", false);
    const access = j.get(decoded, "accessToken").?;
    const started = nowMilliseconds(operation.io);
    const seconds = try @import("../../utils/numbers.zig").coerceValue(a, j.get(decoded, "expiresIn"));
    const expiration = try dateValue(operation, j.num(started + seconds * 1000));
    // The source catches toISOString failure inside its cache-write try, then
    // still returns the new invalid Date. Its caller's ISO conversion fails;
    // this must not fall back to the old token as a failed refresh request.
    if (std.math.isNan(expiration)) return .{ .access = access, .expiration = expiration, .cache = null };
    var updated = try j.clone(a, previous);
    try j.put(a, &updated, "accessToken", try j.clone(a, access));
    try j.put(a, &updated, "expiresAt", j.str(try datetime.formatJavaScriptIsoUtc(a, @intFromFloat(@trunc(expiration)))));
    if (j.get(decoded, "refreshToken")) |value| try j.put(a, &updated, "refreshToken", try j.clone(a, value)) else _ = updated.object.swapRemove("refreshToken");
    // Cache rereads observe JSON.stringify's stored values, while this request
    // still uses the raw newly returned token (for example numeric Infinity).
    return .{ .access = access, .expiration = expiration, .cache = try j.stringifyTree(a, updated) };
}
pub fn tokenPath(operation: m.OperationContext, options: t.StreamOptions, id: []const u8) ![]const u8 {
    const digest = try shared.tokenCacheDigest(std.crypto.hash.Sha1, operation.allocator, id);
    _ = options;
    return @import("../../utils/node_path.zig").join(operation.allocator, &.{ (try shared.sdkHome(operation)) orelse return error.AwsHomeDirectoryUnavailable, ".aws", "sso", "cache", try std.fmt.allocPrint(operation.allocator, "{s}.json", .{std.fmt.bytesToHex(&digest, .lower)}) });
}
pub fn readToken(operation: m.OperationContext, path: []const u8) !std.json.Value {
    try operation.io.checkCancel();
    if (operation.awsState) |state| if (try state.token(operation.allocator, operation.io, try std.fmt.allocPrint(operation.allocator, "sso:{s}", .{path}))) |value| return value;
    const inputs = if (operation.authContext) |context| context.inputs.awsSsoTokens else null;
    return j.clone(operation.allocator, j.get(inputs orelse .null, path) orelse return error.AwsSsoTokenUnavailable);
}
pub fn saveToken(operation: m.OperationContext, path: []const u8, value: std.json.Value) !void {
    const text = try j.stringify(operation.allocator, value, .{ .whitespace = .indent_2 });
    defer {
        std.crypto.secureZero(u8, @constCast(text));
        operation.allocator.free(text);
    }
    try std.Io.Dir.cwd().writeFile(operation.io, .{ .sub_path = path, .data = text });
}
