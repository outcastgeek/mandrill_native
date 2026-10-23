//! Credential profile resolution.
const std = @import("std");
const m = @import("../../models.zig");
const t = @import("../../types.zig");
const j = @import("../../json/value.zig");
const shared = @import("shared_config.zig");
const sts = @import("sts.zig");
const uri = @import("../../transport/uri.zig");
const credentials = @import("credentials.zig");
pub fn resolve(operation: m.OperationContext, options: t.StreamOptions, files: shared.Files) !?credentials.Credentials {
    var visited: std.ArrayList([]const u8) = .empty;
    defer visited.deinit(operation.allocator);
    const profile = try shared.profile(operation, options);
    return resolveProfile(operation, options, files, profile, profile, &visited, false);
}
fn profileCycle(operation: m.OperationContext, root: []const u8, visited: []const []const u8) anyerror {
    if (operation.errorMessage) |slot| {
        var temporary = std.heap.ArenaAllocator.init(operation.allocator);
        defer temporary.deinit();
        const storage = temporary.allocator();
        var record = j.map(storage);
        for (visited) |name| j.put(storage, &record, name, .{ .bool = true }) catch return error.OutOfMemory;
        const entries = j.objectEntries(storage, record.object) catch return error.OutOfMemory;
        const names = storage.alloc([]const u8, entries.len) catch return error.OutOfMemory;
        for (entries, names) |entry, *name| name.* = entry.key;
        const list = std.mem.join(storage, ", ", names) catch return error.OutOfMemory;
        slot.* = std.fmt.allocPrint(operation.allocator, "Detected a cycle attempting to resolve credentials for profile {s}. Profiles visited: {s}", .{ root, list }) catch return error.OutOfMemory;
    }
    return error.AwsProfileCycle;
}
const inheritedVisitedKey = shared.inheritedObjectName;
fn resolveProfile(operation: m.OperationContext, options: t.StreamOptions, files: shared.Files, name: []const u8, root: []const u8, visited: *std.ArrayList([]const u8), recursive: bool) anyerror!?credentials.Credentials {
    const a = operation.allocator;
    const access = try files.get(name, "aws_access_key_id");
    const secret = try files.get(name, "aws_secret_access_key");
    const static: ?credentials.Credentials = if (access != null and secret != null and try optionalString(files, name, "aws_session_token") and try optionalString(files, name, "aws_account_id")) .{ .accessKeyId = access.?, .secretAccessKey = secret.?, .sessionToken = try files.get(name, "aws_session_token") } else null;
    if (visited.items.len > 0 and static != null) return static;
    const role = try files.get(name, "role_arn");
    const source_profile = try files.get(name, "source_profile");
    const source_name = try files.get(name, "credential_source");
    const assume_profile = !recursive and role != null and try optionalString(files, name, "role_session_name") and try optionalString(files, name, "external_id") and try optionalString(files, name, "mfa_serial") and ((source_profile != null and try files.getValue(name, "credential_source") == null) or (source_name != null and try files.getValue(name, "source_profile") == null));
    if (recursive or assume_profile) {
        const raw_profile = (try files.getValue(name, "source_profile")) orelse .null;
        const lookup_profile: ?[]const u8 = if (j.truthy(raw_profile)) try profileText(operation, raw_profile) else null;
        if (lookup_profile) |candidate| {
            if (inheritedVisitedKey(candidate)) return profileCycle(operation, root, visited.items);
            for (visited.items) |previous| {
                if (std.mem.eql(u8, candidate, previous)) return profileCycle(operation, root, visited.items);
            }
        }
        const raw_source = try files.getValue(name, "credential_source");
        const source_kind: ?NamedSource = if (lookup_profile == null) try selectSource(operation, raw_source, name) else null;
        const source_result: anyerror!credentials.Credentials = resolveAssumeSource(operation, options, files, root, visited, lookup_profile, source_kind);
        if (source_result) |_| {} else |err| {
            if (err == error.Canceled or err == error.OutOfMemory) return err;
        }
        if (!j.truthy((try files.getValue(name, "role_arn")) orelse .null) and j.truthy(raw_source orelse .null)) return try source_result;
        const session_name = nonempty(try files.get(name, "role_session_name")) orelse session: {
            const timestamp = try @import("../../utils/numbers.zig").toString(a, @floatFromInt(std.Io.Clock.real.now(operation.io).toMilliseconds()));
            break :session try std.fmt.allocPrint(a, "aws-sdk-js-{s}", .{timestamp});
        };
        const external_id = try files.get(name, "external_id");
        const raw_duration = (try files.getValue(name, "duration_seconds")) orelse .null;
        const duration_text = @import("../../utils/diagnostics.zig").formatThrownValue(a, .{ .value = if (j.truthy(raw_duration)) raw_duration else j.str("3600") }) catch |err| {
            if (err == error.CannotConvertObjectToPrimitive) {
                if (operation.errorMessage) |slot| slot.* = "Cannot convert object to primitive value";
                return error.AwsProfileTypeError;
            }
            return err;
        };
        const duration = (try @import("../../utils/numbers.zig").parseInt10(a, operation.io, duration_text)) orelse std.math.nan(f64);
        const encoded_duration = try @import("../../utils/numbers.zig").toString(a, duration);
        if (nonempty(try files.get(name, "mfa_serial")) != null) {
            if (operation.errorMessage) |slot| slot.* = try std.fmt.allocPrint(a, "Profile {s} requires multi-factor authentication, but no MFA code callback was provided.", .{name});
            return error.AwsMfaCodeProviderRequired;
        }
        const source = try source_result;
        var fields: std.ArrayList(uri.Pair) = .empty;
        try fields.appendSlice(a, &.{ .{ .name = "RoleArn", .value = role orelse return error.InvalidAwsRoleProfile }, .{ .name = "RoleSessionName", .value = session_name }, .{ .name = "DurationSeconds", .value = encoded_duration } });
        if (external_id) |value| try fields.append(a, .{ .name = "ExternalId", .value = value });
        return try sts.request(operation, options, "AssumeRole", fields.items, source, try files.get(name, "region"));
    }
    if (static) |value| return value;
    if (role) |role_arn| if (try optionalString(files, name, "role_session_name") and (try files.get(name, "web_identity_token_file") != null or (if (operation.authContext) |context| context.inputs.awsWebIdentityToken != null else false))) return try webIdentity(operation, options, "", role_arn, try files.get(name, "role_session_name"));
    if (try files.get(name, "credential_process")) |command| return try processForProfile(operation, command, name);
    if (try files.get(name, "sso_session") != null or try files.get(name, "sso_start_url") != null or try files.get(name, "sso_account_id") != null or try files.get(name, "sso_region") != null or try files.get(name, "sso_role_name") != null) return try resolveSsoProfile(operation, options, files, name);
    if (try files.get(name, "login_session")) |session| return try @import("login.zig").resolve(operation, options, files, name, session);
    return null;
}
fn resolveSsoProfile(operation: m.OperationContext, options: t.StreamOptions, files: shared.Files, profile: []const u8) !credentials.Credentials {
    return @import("sso.zig").resolve(operation, options, files, profile) catch |err| {
        if (err == error.Canceled or err == error.OutOfMemory or err == error.NativeSigningDisabled) return err;
        // All escaped SSO failures are terminal at this selected-profile
        // boundary: raw session ToPropertyKey, region/provider type failures,
        // and send/query/header failures. The inner refresh catch has already
        // applied its deliberate live-token fallback before reaching here.
        // credentials.resolve preserves this marker without clearing detail.
        return error.AwsSsoSessionFailed;
    };
}
fn optionalString(files: shared.Files, profile: []const u8, key: []const u8) !bool {
    const value = (try files.getValue(profile, key)) orelse return true;
    return value == .string;
}
fn profileText(operation: m.OperationContext, value: std.json.Value) ![]const u8 {
    return @import("../../utils/diagnostics.zig").formatThrownValue(operation.allocator, .{ .value = value }) catch |err| {
        if (err == error.CannotConvertObjectToPrimitive) {
            if (operation.errorMessage) |slot| slot.* = "Cannot convert object to primitive value";
            return error.AwsProfileTypeError;
        }
        return err;
    };
}
fn resolveAssumeSource(operation: m.OperationContext, options: t.StreamOptions, files: shared.Files, root: []const u8, visited: *std.ArrayList([]const u8), source_profile: ?[]const u8, source_kind: ?NamedSource) anyerror!credentials.Credentials {
    if (source_profile) |profile| {
        try visited.append(operation.allocator, profile);
        defer _ = visited.pop();
        const inherited_source = (try files.getValue(profile, "credential_source")) orelse .null;
        const inherited_role = (try files.getValue(profile, "role_arn")) orelse .null;
        return (try resolveProfile(operation, options, files, profile, root, visited, j.truthy(inherited_source) and !j.truthy(inherited_role))) orelse {
            if (operation.errorMessage) |slot| slot.* = try std.fmt.allocPrint(operation.allocator, "Could not resolve credentials using profile: [{s}] in configuration/credentials file(s).", .{profile});
            return error.AwsSourceProfileCredentialsUnavailable;
        };
    }
    return switch (source_kind.?) {
        .environment => (try credentials.environment(operation, options)) orelse return error.AwsEnvironmentCredentialsUnavailable,
        .container => credentials.remote(operation, options, .container),
        .instance => credentials.remote(operation, options, .instance),
    };
}
fn nonempty(value: ?[]const u8) ?[]const u8 {
    return if (value) |text| if (text.len > 0) text else null else null;
}
const NamedSource = enum { environment, container, instance };
fn selectSource(operation: m.OperationContext, configured: ?std.json.Value, profile: []const u8) !NamedSource {
    const source = if (configured) |value| try profileText(operation, value) else "undefined";
    if (std.mem.eql(u8, source, "Environment")) return .environment;
    if (std.mem.eql(u8, source, "EcsContainer")) return .container;
    if (std.mem.eql(u8, source, "Ec2InstanceMetadata")) return .instance;
    if (inheritedVisitedKey(source)) {
        // resolveCredentialSource returns the inherited member. __proto__ is
        // not callable; Object/toString return non-provider values; the other
        // bare builtins reject an undefined receiver before provider creation.
        if (operation.errorMessage) |slot| slot.* = if (std.mem.eql(u8, source, "__proto__")) "resolveCredentialSource(...) is not a function" else if (std.mem.eql(u8, source, "constructor") or std.mem.eql(u8, source, "toString")) "(intermediate value) is not a function" else if (std.mem.eql(u8, source, "toLocaleString")) "Object.prototype.toLocaleString called on null or undefined" else "Cannot convert undefined or null to object";
        return error.AwsCredentialSourceTypeError;
    }
    if (operation.errorMessage) |slot| slot.* = try std.fmt.allocPrint(operation.allocator, "Unsupported credential source in profile {s}. Got {s}, expected EcsContainer or Ec2InstanceMetadata or Environment.", .{ profile, source });
    return error.UnsupportedAwsCredentialSource;
}
pub fn webIdentity(operation: m.OperationContext, options: t.StreamOptions, path: []const u8, role: []const u8, session: ?[]const u8) !credentials.Credentials {
    _ = path;
    const token = if (operation.authContext) |context| context.inputs.awsWebIdentityToken orelse return error.AwsWebIdentityTokenUnavailable else return error.AwsWebIdentityTokenUnavailable;
    return sts.request(operation, options, "AssumeRoleWithWebIdentity", &.{ .{ .name = "RoleArn", .value = role }, .{ .name = "RoleSessionName", .value = session orelse try std.fmt.allocPrint(operation.allocator, "aws-sdk-js-session-{d}", .{std.Io.Clock.real.now(operation.io).toMilliseconds()}) }, .{ .name = "WebIdentityToken", .value = token } }, null, null);
}
pub fn process(operation: m.OperationContext, options: t.StreamOptions, command: []const u8) !credentials.Credentials {
    return processForProfile(operation, command, try shared.profile(operation, options));
}
fn processFailure(operation: m.OperationContext, profile: []const u8, err: anyerror, comptime detail: []const u8) anyerror {
    if (operation.errorMessage) |slot| slot.* = std.fmt.allocPrint(operation.allocator, "Profile {s} credential_process " ++ detail, .{profile}) catch return error.OutOfMemory;
    return err;
}
fn processExpiration(operation: m.OperationContext, value: std.json.Value) !f64 {
    return @import("../../utils/datetime.zig").parseJavaScriptDateValue(operation.allocator, operation.io, value) catch |err| {
        if (err == error.CannotConvertObjectToPrimitive) if (operation.errorMessage) |slot| {
            slot.* = "Cannot convert object to primitive value";
        };
        return err;
    };
}
fn processForProfile(operation: m.OperationContext, command: []const u8, profile: []const u8) !credentials.Credentials {
    const context = operation.authContext orelse return error.AwsCredentialCommandHostRequired;
    const execute = context.credentialCommandFn orelse return error.AwsCredentialCommandHostRequired;
    const value = execute(context.context, operation.allocator, operation.io, .{ .command = command, .timeoutMs = 0, .errorMessage = operation.errorMessage }) catch |err| {
        if (err == error.CredentialCommandInvalidJson) return processFailure(operation, profile, err, "returned invalid JSON.");
        return err;
    };
    if (value == .null) {
        if (operation.errorMessage) |slot| slot.* = "Cannot read properties of null (reading 'Version')";
        return error.InvalidAwsCredentialProcessVersion;
    }
    const version = j.get(value, "Version") orelse .null;
    if (j.number(version) != 1) return processFailure(operation, profile, error.InvalidAwsCredentialProcessVersion, "did not return Version 1.");
    const access = j.get(value, "AccessKeyId") orelse return processFailure(operation, profile, error.InvalidAwsProcessCredentials, "returned invalid credentials.");
    const secret = j.get(value, "SecretAccessKey") orelse return processFailure(operation, profile, error.InvalidAwsProcessCredentials, "returned invalid credentials.");
    const raw_expiration = j.get(value, "Expiration") orelse .null;
    var expires: ?i64 = null;
    var invalid_expiration = false;
    if (j.truthy(raw_expiration)) {
        // The source constructs currentTime before converting expireTime.
        const current_time: f64 = @floatFromInt(std.Io.Clock.real.now(operation.io).toMilliseconds());
        const time = try processExpiration(operation, raw_expiration);
        if (time < current_time) return processFailure(operation, profile, error.ExpiredAwsProcessCredentials, "returned expired credentials.");
        invalid_expiration = std.math.isNan(time);
        if (!invalid_expiration) expires = @intFromFloat(time);
    }
    const raw_token = j.get(value, "SessionToken") orelse .null;
    return .{ .accessKeyId = j.text(access) orelse "", .secretAccessKey = j.text(secret) orelse "", .sessionToken = nonempty(j.text(raw_token)), .expiration = expires, .expirationInvalid = invalid_expiration, .invalidSigningKeys = access != .string or secret != .string, .invalidSigningToken = j.truthy(raw_token) and raw_token != .string };
}
