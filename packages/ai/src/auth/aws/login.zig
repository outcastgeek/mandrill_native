//! AWS login profiles. JSON cache and proof documents stay in std.json;
//! PEM loading and request signatures use native standard-library crypto.
const std = @import("std");
const m = @import("../../models.zig");
const t = @import("../../types.zig");
const j = @import("../../json/value.zig");
const http = @import("../oauth/http.zig");
const shared = @import("shared_config.zig");
const sso = @import("sso.zig");
const datetime = @import("../../utils/datetime.zig");
const Credentials = @import("credentials.zig").Credentials;
const uri = @import("../../transport/uri.zig");
const json_http = @import("json_http.zig");
const cacheJoin = @import("../../utils/node_path.zig").join;
pub fn resolve(operation: m.OperationContext, options: t.StreamOptions, files: shared.Files, profile: []const u8, session: []const u8) !Credentials {
    const a = operation.allocator;
    const directory = (try shared.sdkEnv(operation, "AWS_LOGIN_CACHE_DIRECTORY")) orelse if (try shared.osHome(operation)) |home| try cacheJoin(a, &.{ home, ".aws", "login", "cache" }) else null;
    const digest = try shared.tokenCacheDigest(std.crypto.hash.sha2.Sha256, a, session);
    const path = if (directory) |output| try cacheJoin(a, &.{ output, try std.fmt.allocPrint(a, "{s}.json", .{std.fmt.bytesToHex(&digest, .lower)}) }) else null;
    const token = try load(operation, session);
    const now: f64 = @floatFromInt(std.Io.Clock.real.now(operation.io).toMilliseconds());
    const current = try toCredentials(operation, token);
    if (current.expiration) |expiry| if (@as(f64, @floatFromInt(expiry)) - now <= 300_000) return refresh(operation, options, files, profile, path, session, token, current);
    return current;
}
fn loadFailure(operation: m.OperationContext, path: []const u8, cause: []const u8) anyerror {
    if (operation.errorMessage) |slot| slot.* = std.fmt.allocPrint(operation.allocator, "Failed to load token from {s}: {s}", .{ path, cause }) catch return error.OutOfMemory;
    return error.AwsLoginSessionInvalid;
}
fn load(operation: m.OperationContext, path: []const u8) !std.json.Value {
    try operation.io.checkCancel();
    const cache_key = try std.fmt.allocPrint(operation.allocator, "login:{s}", .{path});
    const cached = if (operation.awsState) |state| try state.token(operation.allocator, operation.io, cache_key) else null;
    const inputs = if (operation.authContext) |context| context.inputs.awsLoginTokens else null;
    const token = cached orelse try j.clone(operation.allocator, j.get(inputs orelse .null, path) orelse return loadFailure(operation, path, "in-memory login token unavailable"));
    if (token == .null) return loadFailure(operation, path, "TypeError: Cannot read properties of null (reading 'accessToken')");
    var missing: [5][]const u8 = undefined;
    var count: usize = 0;
    for ([_][]const u8{ "accessToken", "clientId", "refreshToken", "dpopKey" }) |key| if (!j.truthy(j.get(token, key) orelse .null)) {
        missing[count] = key;
        count += 1;
    };
    const access = j.get(token, "accessToken") orelse .null;
    if (!j.truthy(j.get(access, "accountId") orelse .null)) {
        missing[count] = "accountId";
        count += 1;
    }
    if (count > 0) {
        if (operation.errorMessage) |slot| {
            const fields = try std.mem.join(operation.allocator, ", ", missing[0..count]);
            defer operation.allocator.free(fields);
            slot.* = try std.fmt.allocPrint(operation.allocator, "Failed to load token from {s}: CredentialsProviderError: Token validation failed, missing fields: {s}", .{ path, fields });
        }
        return error.AwsLoginSessionInvalid;
    }
    return token;
}
fn toCredentials(operation: m.OperationContext, token: std.json.Value) !Credentials {
    const value = j.get(token, "accessToken") orelse return error.AwsLoginSessionInvalid;
    const expiration = try tokenExpiration(operation, value);
    const access = j.get(value, "accessKeyId") orelse .null;
    const secret = j.get(value, "secretAccessKey") orelse .null;
    const session = j.get(value, "sessionToken") orelse .null;
    return .{ .accessKeyId = j.text(access) orelse "", .secretAccessKey = j.text(secret) orelse "", .sessionToken = if (j.truthy(session)) j.text(session) else null, .expiration = if (std.math.isNan(expiration)) null else @intFromFloat(expiration), .expirationInvalid = std.math.isNan(expiration), .invalidSigningKeys = access != .string or secret != .string, .invalidSigningToken = j.truthy(session) and session != .string };
}
fn tokenExpiration(operation: m.OperationContext, access_token: std.json.Value) !f64 {
    return datetime.parseJavaScriptDateValue(operation.allocator, operation.io, j.get(access_token, "expiresAt")) catch |err| {
        if (err == error.CannotConvertObjectToPrimitive) if (operation.errorMessage) |slot| {
            slot.* = "Cannot convert object to primitive value";
        };
        return err;
    };
}
fn refresh(operation: m.OperationContext, options: t.StreamOptions, files: shared.Files, profile: []const u8, path: ?[]const u8, session: []const u8, original_previous: std.json.Value, original_current: Credentials) !Credentials {
    const a = operation.allocator;
    const prior_message = if (operation.errorMessage) |slot| slot.* else null;
    const disk = load(operation, session) catch |err| fallback: {
        if (err == error.Canceled or err == error.OutOfMemory) return err;
        if (operation.errorMessage) |slot| slot.* = prior_message;
        break :fallback original_previous;
    };
    const now: f64 = @floatFromInt(std.Io.Clock.real.now(operation.io).toMilliseconds());
    const disk_expiry = try tokenExpiration(operation, j.get(disk, "accessToken") orelse .null);
    const original_expiry = try tokenExpiration(operation, j.get(original_previous, "accessToken") orelse .null);
    const keep_original = disk_expiry <= now and original_expiry > now;
    const previous = if (keep_original) original_previous else disk;
    const current = if (keep_original) original_current else try toCredentials(operation, disk);
    const fresh_expiry = try tokenExpiration(operation, j.get(previous, "accessToken") orelse .null);
    if (fresh_expiry - @as(f64, @floatFromInt(std.Io.Clock.real.now(operation.io).toMilliseconds())) > 300_000) return current;
    return refreshRequest(operation, options, files, profile, path, session, previous) catch |err| {
        if (err == error.Canceled or err == error.OutOfMemory or err == error.AwsLoginAccessDenied or err == error.NativeSigningDisabled) return err;
        if (current.expiration) |expiry| if (expiry > std.Io.Clock.real.now(operation.io).toMilliseconds()) {
            if (operation.errorMessage) |slot| slot.* = null;
            return current;
        };
        if (operation.errorMessage) |slot| slot.* = try std.fmt.allocPrint(a, "Failed to refresh token: {s}. Please re-authenticate using aws login", .{slot.* orelse @errorName(err)});
        return error.AwsLoginRefreshFailed;
    };
}
fn refreshRequest(operation: m.OperationContext, options: t.StreamOptions, files: shared.Files, profile: []const u8, path: ?[]const u8, session: []const u8, previous: std.json.Value) !Credentials {
    const a = operation.allocator;
    const region = (try files.get(profile, "region")) orelse operation.awsRegion orelse (try shared.sdkEnv(operation, "AWS_REGION")) orelse {
        if (operation.errorMessage) |slot| slot.* = "Error: Region is missing";
        return error.AwsLoginRegionRequired;
    };
    const endpoint = try @import("endpoints.zig").resolve(operation, options, .signin, region);
    var url = try uri.appendPath(a, endpoint, &.{ "v1", "token" });
    url.query = null;
    url.fragment = null;
    url = try uri.parseHttpUrl(a, try uri.render(a, url));
    if (url.port == 0) url.port = null;
    // tokenInput has the httpPayload trait, so the wire document is this flat
    // schema projection in clientId/grantType/refreshToken iteration order.
    var body = j.map(a);
    if (j.getNonNull(previous, "clientId")) |value| try j.put(a, &body, "clientId", try json_http.stringSchemaMember(a, value));
    try j.put(a, &body, "grantType", j.str("refresh_token"));
    if (j.getNonNull(previous, "refreshToken")) |value| try j.put(a, &body, "refreshToken", try json_http.stringSchemaMember(a, value));
    return exchange(operation, options, path, session, previous, try uri.render(a, url), body);
}
fn exchange(operation: m.OperationContext, options: t.StreamOptions, path: ?[]const u8, session: []const u8, previous: std.json.Value, rendered: []const u8, body: std.json.Value) !Credentials {
    const a = operation.allocator;
    var proof = Proof{ .session = session, .url = try proofEndpoint(a, rendered) };
    const response = try json_http.requestWithFactory(operation, options, .POST, rendered, body, &.{}, .{ .context = &proof, .call = Proof.headers });
    if (!response.ok()) {
        if (isAccessDenied(response)) {
            const kind = j.field(response.body, "error") orelse @as([]const u8, "");
            const message = j.field(response.body, "message") orelse j.field(response.body, "Message") orelse @as([]const u8, "");
            if (operation.errorMessage) |slot| slot.* = if (std.mem.eql(u8, kind, "TOKEN_EXPIRED")) "Your session has expired. Please reauthenticate." else if (std.mem.eql(u8, kind, "USER_CREDENTIALS_CHANGED")) "Unable to refresh credentials because of a change in your password. Please reauthenticate with your new password." else if (std.mem.eql(u8, kind, "INSUFFICIENT_PERMISSIONS")) "Unable to refresh credentials due to insufficient permissions. You may be missing permission for the 'CreateOAuth2Token' action." else if (message.len > 0) try std.fmt.allocPrint(a, "Failed to refresh token: AccessDeniedException: {s}. Please re-authenticate using `aws login`", .{message}) else try std.fmt.allocPrint(a, "Failed to refresh token: {s}. Please re-authenticate using `aws login`", .{response.raw});
            return error.AwsLoginAccessDenied;
        }
        return error.AwsLoginRefreshFailed;
    }
    const access = json_http.structField(response.body, "accessToken") orelse .null;
    const refresh_token = json_http.structField(response.body, "refreshToken") orelse .null;
    const access_key = json_http.structField(access, "accessKeyId") orelse .null;
    const secret_key = json_http.structField(access, "secretAccessKey") orelse .null;
    const session_token = json_http.structField(access, "sessionToken") orelse .null;
    if (!j.truthy(refresh_token) or !j.truthy(access_key) or !j.truthy(secret_key) or !j.truthy(session_token)) {
        if (operation.errorMessage) |slot| slot.* = "CredentialsProviderError: Token refresh response missing required fields";
        return error.InvalidAwsLoginRefreshResponse;
    }
    const lifetime = if (json_http.structField(response.body, "expiresIn")) |value| if (value == .null) j.int(900) else json_http.numericSchemaValue(value) else j.int(900);
    // Signin computes the duration before sampling Date.now, unlike SSO OIDC.
    const duration = (try @import("../../utils/numbers.zig").coerceValue(a, lifetime)) * 1000;
    const expiration = try datetime.parseJavaScriptDateValue(a, operation.io, j.num(@as(f64, @floatFromInt(std.Io.Clock.real.now(operation.io).toMilliseconds())) + duration));
    if (std.math.isNan(expiration)) {
        if (operation.errorMessage) |slot| slot.* = "RangeError: Invalid time value";
        return error.InvalidAwsLoginExpiration;
    }
    var updated = try j.clone(a, previous);
    var updated_access = try j.clone(a, j.get(previous, "accessToken").?);
    try j.put(a, &updated_access, "accessKeyId", try j.clone(a, access_key));
    try j.put(a, &updated_access, "secretAccessKey", try j.clone(a, secret_key));
    try j.put(a, &updated_access, "sessionToken", try j.clone(a, session_token));
    try j.put(a, &updated_access, "expiresAt", j.str(try datetime.formatJavaScriptIsoUtc(a, @intFromFloat(expiration))));
    try j.put(a, &updated, "accessToken", updated_access);
    try j.put(a, &updated, "refreshToken", try j.clone(a, refresh_token));
    if (path) |output| if (std.fs.path.dirname(output)) |directory| std.Io.Dir.cwd().createDirPath(operation.io, directory) catch |err| {
        if (err == error.Canceled) return err;
    };
    if (path) |output| try sso.saveToken(operation, output, updated);
    if (operation.awsState) |state| try state.putToken(operation.io, try std.fmt.allocPrint(a, "login:{s}", .{session}), try j.stringifyTree(a, updated));
    return toCredentials(operation, updated);
}
const Proof = struct {
    session: []const u8,
    url: []const u8,
    fn headers(context: *anyopaque, operation: m.OperationContext) !t.ProviderHeaders {
        const self: *Proof = @ptrCast(@alignCast(context));
        // The source rereads/validates the token before its crypto try on each
        // interceptor invocation. The command body retains its earlier token.
        const token = try load(operation, self.session);
        const value = dpopFromToken(operation, token, "POST", self.url) catch |err| {
            if (err == error.Canceled or err == error.OutOfMemory or err == error.NativeSigningDisabled) return err;
            if (operation.errorMessage) |slot| {
                const detail = if (err == error.InvalidAwsLoginKeyInput) slot.* orelse "Invalid AWS login DPoP private key input" else @errorName(err);
                slot.* = try std.fmt.allocPrint(operation.allocator, "CredentialsProviderError: Failed to generate Dpop proof: {s}", .{detail});
            }
            return err;
        };
        errdefer operation.allocator.free(value);
        const result = try operation.allocator.alloc(t.ProviderHeader, 1);
        result[0] = .{ .name = "DPoP", .value = value };
        return result;
    }
};
fn proofEndpoint(a: std.mem.Allocator, raw: []const u8) ![]const u8 {
    var parsed = try uri.parseHttpUrl(a, raw);
    if (parsed.port == 0) parsed.port = null;
    var writer = std.Io.Writer.Allocating.init(a);
    defer writer.deinit();
    try parsed.writeToStream(&writer.writer, .{ .scheme = true, .authority = true, .authentication = false, .path = true, .query = false, .fragment = false });
    return writer.toOwnedSlice();
}
fn invalidDpopKey(operation: m.OperationContext, value: ?std.json.Value) anyerror {
    const a = operation.allocator;
    var direct_detail: ?[]const u8 = null;
    const received: ?[]const u8 = if (value) |raw| switch (raw) {
        .null => "null",
        .array => "an instance of Array",
        .bool => |boolean| if (boolean) "type boolean (true)" else "type boolean (false)",
        .integer, .float, .number_string => numeric: {
            const number = j.number(raw) orelse break :numeric null;
            const text = if (number == 0 and std.math.signbit(number)) a.dupe(u8, "-0") catch |err| return err else @import("../../utils/numbers.zig").toString(a, number) catch |err| return err;
            defer a.free(text);
            break :numeric std.fmt.allocPrint(a, "type number ({s})", .{text}) catch |err| return err;
        },
        .object => object: {
            const constructor = j.get(raw, "constructor") orelse break :object "an instance of Object";
            if (!j.truthy(constructor)) break :object "[Object]";
            if (constructor != .object and constructor != .array) {
                const text = @import("../../utils/diagnostics.zig").formatThrownValue(a, .{ .value = constructor }) catch |err| return err;
                defer a.free(text);
                direct_detail = std.fmt.allocPrint(a, "Cannot use 'in' operator to search for 'name' in {s}", .{text}) catch |err| return err;
                break :object null;
            }
            if (j.get(constructor, "name")) |name| {
                const text = @import("../../utils/diagnostics.zig").formatThrownValue(a, .{ .value = name }) catch |err| {
                    if (err != error.CannotConvertObjectToPrimitive) return err;
                    direct_detail = "Cannot convert object to primitive value";
                    break :object null;
                };
                defer a.free(text);
                break :object std.fmt.allocPrint(a, "an instance of {s}", .{text}) catch |err| return err;
            }
            // Parsed JSON retains the ordinary Object prototype. inspect at
            // depth -1 reduces this fallback to [Object], even with a
            // non-function constructor data property; no general formatter.
            break :object "[Object]";
        },
        else => null,
    } else "undefined";
    if (operation.errorMessage) |slot| {
        slot.* = direct_detail orelse if (received) |description| std.fmt.allocPrint(a, "The \"key.key\" property must be of type string or an instance of ArrayBuffer, Buffer, TypedArray, or DataView. Received {s}", .{description}) catch |err| return err else "Invalid AWS login DPoP private key input";
    }
    return error.InvalidAwsLoginKeyInput;
}
fn dpopFromToken(operation: m.OperationContext, token: std.json.Value, method: []const u8, url: []const u8) ![]const u8 {
    const pem = j.field(token, "dpopKey") orelse return invalidDpopKey(operation, j.get(token, "dpopKey"));
    return dpop(operation, pem, method, url);
}
fn isAccessDenied(response: http.Response) bool {
    const code = @import("../../utils/headers.zig").get(response.headers, "x-amzn-errortype") orelse j.field(response.body, "__type") orelse j.field(response.body, "code") orelse j.field(response.body, "Code") orelse return false;
    return std.mem.eql(u8, code, "AccessDeniedException") or std.mem.endsWith(u8, code, "#AccessDeniedException") or std.mem.startsWith(u8, code, "AccessDeniedException:");
}
fn base64(a: std.mem.Allocator, bytes: []const u8) ![]const u8 {
    const output = try a.alloc(u8, std.base64.url_safe_no_pad.Encoder.calcSize(bytes.len));
    return std.base64.url_safe_no_pad.Encoder.encode(output, bytes);
}
fn dpop(operation: m.OperationContext, pem: []const u8, method: []const u8, url: []const u8) ![]const u8 {
    const a = operation.allocator;
    var key = try @import("login_signing.zig").Key.open(a, operation.io, pem);
    defer key.deinit();
    const documents = try proofDocuments(a, operation.io, try key.x(), try key.y(), method, url);
    defer a.free(documents.header);
    defer a.free(documents.payload);
    const header = try base64(a, documents.header);
    defer a.free(header);
    const payload = try base64(a, documents.payload);
    defer a.free(payload);
    var writer = std.Io.Writer.Allocating.init(a);
    defer writer.deinit();
    try writer.writer.writeAll(header);
    try writer.writer.writeByte('.');
    try writer.writer.writeAll(payload);
    const signature = try key.sign(operation.io, writer.written());
    const encoded_signature = try base64(a, &signature);
    defer a.free(encoded_signature);
    try writer.writer.writeByte('.');
    try writer.writer.writeAll(encoded_signature);
    return writer.toOwnedSlice();
}
pub const ProofDocuments = struct { header: []const u8, payload: []const u8 };
pub fn proofDocuments(a: std.mem.Allocator, io: std.Io, x: []const u8, y: []const u8, method: []const u8, url: []const u8) !ProofDocuments {
    const encoded_x = try base64(a, x);
    defer a.free(encoded_x);
    const encoded_y = try base64(a, y);
    defer a.free(encoded_y);
    const header = try @import("../../json/value.zig").stringify(a, .{ .alg = "ES256", .typ = "dpop+jwt", .jwk = .{ .kty = "EC", .crv = "P-256", .x = encoded_x, .y = encoded_y } }, .{});
    errdefer a.free(header);
    return .{
        .header = header,
        .payload = try @import("../../json/value.zig").stringify(a, .{ .jti = try @import("../../utils/uuid.zig").randomV4(io), .htm = method, .htu = url, .iat = @floor(@as(f64, @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds())) / 1000) }, .{}),
    };
}
