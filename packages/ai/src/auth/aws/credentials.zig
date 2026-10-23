const std = @import("std");
const t = @import("../../types.zig");
const m = @import("../../models.zig");
const j = @import("../../json/value.zig");
const c = @import("../../api/common.zig");
const http = @import("../oauth/http.zig");
const uri = @import("../../transport/uri.zig");
const headers = @import("../../utils/headers.zig");
pub const Credentials = struct {
    accessKeyId: []const u8,
    secretAccessKey: []const u8,
    sessionToken: ?[]const u8 = null,
    expiration: ?i64 = null,
    /// A legacy SDK Date can be present with NaN time. Keep that state distinct
    /// from an omitted expiration without forcing it into an integer timestamp.
    expirationInvalid: bool = false,
    /// Process providers validate property presence; the Smithy signer later
    /// rejects present non-string keys without advancing the provider chain.
    invalidSigningKeys: bool = false,
    /// A truthy non-string process SessionToken reaches canonical-header trim.
    invalidSigningToken: bool = false,
    pub fn validateSigning(self: @This(), operation: m.OperationContext) !void {
        if (self.invalidSigningKeys) {
            if (operation.errorMessage) |slot| slot.* = "Resolved credential object is not valid";
            return error.InvalidAwsSigningCredentials;
        }
    }
    pub fn validateSigningToken(self: @This(), operation: m.OperationContext) !void {
        if (self.invalidSigningToken) {
            if (operation.errorMessage) |slot| slot.* = "headers[headerName].trim is not a function";
            return error.InvalidAwsSigningToken;
        }
    }
};
/// Owns initial default-provider refresh. Models may retain its worker beyond
/// this producer; request-specific transports keep request-owned cancellation.
/// Captured credential inputs/options and diagnostics have a private arena.
/// Opaque contexts borrow the selected owner's documented lifetime. Allocators
/// must support concurrent use, as for the enclosing stream producer.
pub const PassiveRefresh = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    frame: ?*Frame = null,
    future: ?std.Io.Future(void) = null,
    started: bool = false,
    const Frame = struct {
        allocator: std.mem.Allocator,
        arena: std.heap.ArenaAllocator,
        operation: m.OperationContext,
        options: t.StreamOptions,
        detail: ?[]const u8 = null,
        done: std.atomic.Value(bool) = .init(false),
        fn release(context: *anyopaque) void {
            const self: *@This() = @ptrCast(@alignCast(context));
            const a = self.allocator;
            self.arena.deinit();
            a.destroy(self);
        }
        fn run(self: *@This()) void {
            defer self.done.store(true, .release);
            // memoizeChain's passive chain rejection is deliberately ignored.
            // Its result cannot replace this command's already-selected identity.
            _ = resolve(self.operation, self.options) catch return;
        }
    };
    pub fn init(a: std.mem.Allocator, io: std.Io) @This() {
        return .{ .allocator = a, .io = io };
    }
    pub fn deinit(self: *@This()) void {
        if (self.future) |*future| future.cancel(self.io);
        if (self.frame) |frame| Frame.release(frame);
        self.* = undefined;
    }
    fn start(self: *@This(), operation: m.OperationContext, options: t.StreamOptions) !void {
        if (self.started) return;
        const background = if (options.httpTransport == null) operation.awsBackgroundOwner else null;
        const a = if (background) |owner| owner.allocator else self.allocator;
        const frame = try a.create(Frame);
        frame.* = .{ .allocator = a, .arena = std.heap.ArenaAllocator.init(a), .operation = operation, .options = .{} };
        errdefer Frame.release(frame);
        const owned = frame.arena.allocator();
        frame.operation.allocator = owned;
        frame.operation.io = self.io;
        frame.operation.errorMessage = &frame.detail;
        frame.operation.errorStorage = null;
        frame.operation.googleState = null;
        frame.operation.googleToolCallIds = null;
        if (operation.awsRegion) |region_name| frame.operation.awsRegion = try owned.dupe(u8, region_name);
        if (frame.operation.authContext) |*context| {
            const Inputs = @import("../types.zig").CredentialInputs;
            var inputs: Inputs = .{};
            inline for (.{ "awsConfig", "awsCredentials", "awsSsoSessions", "awsServices", "awsSsoTokens", "awsLoginTokens" }) |name| {
                if (@field(context.inputs, name)) |value| @field(inputs, name) = try j.clone(owned, value);
            }
            inline for (.{ "awsWebIdentityToken", "awsContainerAuthorizationToken" }) |name| {
                if (@field(context.inputs, name)) |text| @field(inputs, name) = try owned.dupe(u8, text);
            }
            context.inputs = inputs;
            context.vertexAdcExistsCache = null;
        }
        for (options.env.keys(), options.env.values()) |name, text| try frame.options.env.put(owned, try owned.dupe(u8, name), try owned.dupe(u8, text));
        if (c.extra(options, "profile")) |profile_value| {
            var extra = j.map(owned);
            try j.put(owned, &extra, "profile", try j.clone(owned, profile_value));
            frame.options.extra = extra;
        }
        frame.options.maxRetries = options.maxRetries;
        frame.options.maxRetryDelayMs = options.maxRetryDelayMs;
        frame.options.timeoutMs = options.timeoutMs;
        frame.options.errorMessage = &frame.detail;
        var future = try self.io.concurrent(Frame.run, .{frame});
        errdefer future.cancel(self.io);
        if (background) |owner| {
            try owner.retainBackground(self.io, future, frame, Frame.release, &frame.done);
        } else {
            self.frame = frame;
            self.future = future;
        }
        self.started = true;
    }
    pub fn resolveInitial(self: *@This(), operation: m.OperationContext, options: t.StreamOptions) !Credentials {
        var selected = try resolve(operation, options);
        while (true) {
            if (selected.expiration) |expiration| {
                const expiry: f64 = @floatFromInt(expiration);
                // The source memoizer recursively reacquires an already-expired
                // first result, then checks the passive window on its next read.
                if (expiry < @as(f64, @floatFromInt(std.Io.Clock.real.now(operation.io).toMilliseconds()))) {
                    selected = try resolve(operation, options);
                    continue;
                }
                if (expiry - @as(f64, @floatFromInt(std.Io.Clock.real.now(operation.io).toMilliseconds())) < 300_000) try self.start(operation, options);
            }
            return selected;
        }
    }
};
pub const env = @import("shared_config.zig").env;
fn nonempty(value: ?[]const u8) ?[]const u8 {
    const text = value orelse return null;
    return if (text.len > 0) text else null;
}
pub fn bearerToken(operation: m.OperationContext, options: t.StreamOptions) !?[]const u8 {
    return nonempty(if (c.extra(options, "bearerToken")) |value| j.text(value) else null) orelse nonempty(options.apiKey) orelse try env(operation, options, "AWS_BEARER_TOKEN_BEDROCK");
}
pub fn usesSigV4(operation: m.OperationContext, options: t.StreamOptions) !bool {
    return std.mem.eql(u8, (try env(operation, options, "AWS_BEDROCK_SKIP_AUTH")) orelse "", "1") or try bearerToken(operation, options) == null;
}
pub fn sharedFilesPresent(operation: m.OperationContext, options: t.StreamOptions) !bool {
    var files = try @import("shared_config.zig").Files.init(operation, options);
    defer files.deinit();
    return files.config != null or files.credentials != null;
}
pub fn environment(operation: m.OperationContext, options: t.StreamOptions) !?Credentials {
    _ = options;
    const shared = @import("shared_config.zig");
    const access = nonempty(try shared.sdkEnv(operation, "AWS_ACCESS_KEY_ID"));
    const secret = nonempty(try shared.sdkEnv(operation, "AWS_SECRET_ACCESS_KEY"));
    if (access == null or secret == null) return null;
    const expiration_text = nonempty(try shared.sdkEnv(operation, "AWS_CREDENTIAL_EXPIRATION"));
    const expiry = if (expiration_text) |text| try @import("../../utils/datetime.zig").parseJavaScriptDate(operation.allocator, operation.io, text) else std.math.nan(f64);
    return .{ .accessKeyId = access.?, .secretAccessKey = secret.?, .sessionToken = nonempty(try shared.sdkEnv(operation, "AWS_SESSION_TOKEN")), .expiration = if (std.math.isNan(expiry)) null else @intFromFloat(expiry), .expirationInvalid = expiration_text != null and std.math.isNan(expiry) };
}
pub fn resolve(operation: m.OperationContext, options: t.StreamOptions) !Credentials {
    const shared = @import("shared_config.zig");
    const profiles = @import("profiles.zig");
    const explicit_profile = try shared.explicitProfile(operation, options);
    // Pi supplies static credentials directly unless the profile came from its
    // explicit options/scoped credential. Ambient AWS_PROFILE alone does not
    // suppress that explicit SDK credential value.
    if (explicit_profile == null) {
        const access = nonempty(try env(operation, options, "AWS_ACCESS_KEY_ID"));
        const secret = nonempty(try env(operation, options, "AWS_SECRET_ACCESS_KEY"));
        if (access != null and secret != null) return .{ .accessKeyId = access.?, .secretAccessKey = secret.?, .sessionToken = nonempty(try env(operation, options, "AWS_SESSION_TOKEN")) };
    }
    var files = try shared.Files.init(operation, options);
    defer files.deinit();
    // Selected SSO profiles translate escaped failures to the terminal
    // AwsSsoSessionFailed marker before this default-chain decision.
    const profiled = profiles.resolve(operation, options, files) catch |err| blk: {
        if (terminalMetadataResponse(err)) return err;
        switch (err) {
            error.Canceled, error.OutOfMemory, error.AwsIniCodecRequired, error.AwsStsXmlCodecRequired, error.AwsMfaCodeProviderRequired, error.InvalidAwsSsoProfile, error.ExpiredAwsSsoSession, error.AwsSsoRoleCredentialsFailed, error.InvalidAwsSsoCredentials, error.AwsLoginSessionInvalid, error.AwsLoginAccessDenied, error.UnsupportedAwsLoginKey, error.AwsSsoSessionFailed, error.AwsCredentialSourceTypeError, error.AwsProfileTypeError, error.AwsContainerMetadataEndpointRejected, error.AwsContainerRequestPathInvalid, error.AwsContainerUrlTypeError, error.AwsMetadataUrlTypeError, error.AwsMetadataRequestPathInvalid, error.AwsMetadataRequestHeaderTypeError, error.AwsMetadataRequestProtocolTypeError, error.InvalidAwsRegion => return err,
            else => {
                if (operation.errorMessage) |slot| slot.* = null;
                break :blk null;
            },
        }
    };
    if (profiled) |value| return value;
    if (try files.get(try shared.profile(operation, options), "credential_process")) |command| {
        if (profiles.process(operation, options, command)) |value| return value else |err| {
            if (err == error.Canceled or err == error.OutOfMemory) return err;
            if (operation.errorMessage) |slot| slot.* = null;
        }
    }
    if (if (operation.authContext) |context| context.inputs.awsWebIdentityToken != null else false) if (nonempty(try shared.sdkEnv(operation, "AWS_ROLE_ARN"))) |role| {
        if (profiles.webIdentity(operation, options, "", role, try shared.sdkEnv(operation, "AWS_ROLE_SESSION_NAME"))) |value| return value else |err| {
            if (err == error.Canceled or err == error.OutOfMemory or err == error.AwsStsXmlCodecRequired or err == error.InvalidAwsRegion) return err;
            if (operation.errorMessage) |slot| slot.* = null;
        }
    };
    return remote(operation, options, .automatic) catch |err| switch (err) {
        // These metadata failures carry ProviderError.tryNextLink in the
        // source. The default chain's final provider supplies the public error.
        // Direct credential_source calls retain their provider-specific errors.
        error.AwsCredentialsUnavailable, error.AwsMetadataRoleFailed, error.AwsMetadataCredentialsFailed, error.AwsMetadataTokenFailed, error.AwsMetadataV1Disabled, error.InvalidAwsMetadataCredentials, error.AwsContainerCredentialsFailed, error.AwsHttpCredentialsUrlRejected, error.AwsMetadataTransportProviderError => {
            if (operation.errorMessage) |slot| slot.* = "Could not load credentials from any providers";
            return error.AwsCredentialsUnavailable;
        },
        else => return err,
    };
}
pub const Remote = enum { automatic, container, instance };
fn remoteFailure(operation: m.OperationContext, err: anyerror, comptime format: []const u8, args: anytype) anyerror {
    if (operation.errorMessage) |slot| slot.* = std.fmt.allocPrint(operation.allocator, format, args) catch return error.OutOfMemory;
    return err;
}
fn terminalMetadataResponse(err: anyerror) bool {
    return switch (err) {
        error.AwsMetadataJsonSyntaxError, error.AwsContainerResponseTypeError, error.AwsContainerResponseDateError, error.AwsContainerResponseReadError => true,
        else => false,
    };
}
fn credentialJson(operation: m.OperationContext, text: []const u8) !std.json.Value {
    return j.parseFromSliceLeaky(std.json.Value, operation.allocator, text, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |err| {
        if (err == error.Canceled or err == error.OutOfMemory) return err;
        return remoteFailure(operation, error.AwsMetadataJsonSyntaxError, "{s}", .{@errorName(err)});
    };
}
fn containerUrl(operation: m.OperationContext, text: []const u8) !std.Uri {
    return uri.parseHttpUrl(operation.allocator, text) catch |err| switch (err) {
        error.UnexpectedCharacter, error.InvalidFormat, error.InvalidPort, error.InvalidHostName => return remoteFailure(operation, error.AwsContainerUrlTypeError, "Invalid URL", .{}),
        else => return err,
    };
}
fn legacyRequestUri(operation: m.OperationContext, url: std.Uri, path: []const u8, invalid_path_error: anyerror) !std.Uri {
    const a = operation.allocator;
    var invalid = try @import("../../regex/regexp.zig").Regex.init(a, operation.io, "[^\\u0021-\\u00ff]", .{});
    defer invalid.deinit();
    if (try invalid.exec(a, path, 0)) |found| {
        var match = found;
        defer match.deinit();
        return remoteFailure(operation, invalid_path_error, "Request path contains unescaped characters", .{});
    }
    var target = url;
    target.path = .{ .percent_encoded = try @import("../../transport/http.zig").httpFieldBytes(a, path) };
    target.query = null;
    target.fragment = null;
    return target;
}
fn metadataUrl(operation: m.OperationContext, text: []const u8) !std.Uri {
    // @smithy/core parseUrl delegates string input to new URL before returning
    // its typed endpoint components. Preserve that boundary before path edits.
    return uri.parseHttpUrl(operation.allocator, text) catch |err| switch (err) {
        error.UnexpectedCharacter, error.InvalidFormat, error.InvalidPort, error.InvalidHostName => return remoteFailure(operation, error.AwsMetadataUrlTypeError, "Invalid URL", .{}),
        else => return err,
    };
}
pub fn remote(operation: m.OperationContext, options: t.StreamOptions, selection: Remote) !Credentials {
    const a = operation.allocator;
    const io = operation.io;
    const prior_message = if (operation.errorMessage) |slot| slot.* else null;
    var endpoint: ?std.Uri = null;
    if (selection != .instance) {
        if (nonempty(try @import("shared_config.zig").sdkEnv(operation, "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI"))) |relative| {
            endpoint = try containerUrl(operation, try std.fmt.allocPrint(a, "http://169.254.170.2{s}", .{relative}));
        } else if (nonempty(try @import("shared_config.zig").sdkEnv(operation, "AWS_CONTAINER_CREDENTIALS_FULL_URI"))) |url| endpoint = try containerUrl(operation, url);
    }
    if (selection != .instance) if (endpoint) |url| {
        try validateModernContainerEndpoint(operation, url);
        var attempts: usize = 0;
        while (attempts <= 3) : (attempts += 1) {
            const result = containerCredentials(operation, options, url, true);
            if (result) |credentials| return credentials else |err| {
                if (err == error.Canceled or err == error.OutOfMemory) return err;
                // retryWrapper retries every rejection, but chain only tries
                // legacy metadata when the final error is a ProviderError.
                if (attempts == 3 and terminalMetadataResponse(err)) return err;
                if (operation.errorMessage) |slot| slot.* = prior_message;
            }
            if (attempts < 3) try io.sleep(.fromMilliseconds(1000), .awake);
        }
        // The pinned default chain next tries its older container provider.
        const relative = nonempty(try @import("shared_config.zig").sdkEnv(operation, "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI"));
        if (relative) |path| {
            // Node receives the environment value as its literal request
            // path with a fixed host, independent of URL parsing.
            return containerCredentials(operation, options, try legacyRequestUri(operation, .{ .scheme = "http", .host = .{ .raw = "169.254.170.2" } }, path, error.AwsContainerRequestPathInvalid), false);
        }
        const host = try (url.host orelse return error.InvalidAwsContainerEndpoint).toRawMaybeAlloc(a);
        if (!std.mem.eql(u8, host, "localhost") and !std.mem.eql(u8, host, "127.0.0.1")) return remoteFailure(operation, error.AwsContainerMetadataEndpointRejected, "{s} is not a valid container metadata service hostname", .{host});
        if (!std.ascii.eqlIgnoreCase(url.scheme, "http") and !std.ascii.eqlIgnoreCase(url.scheme, "https")) return remoteFailure(operation, error.AwsContainerMetadataEndpointRejected, "{s}: is not a valid container metadata service protocol", .{url.scheme});
        return containerCredentials(operation, options, url, false);
    };
    if (selection == .container) return remoteFailure(operation, error.AwsContainerCredentialsUnavailable, "No HTTP credential provider host provided.\nSet AWS_CONTAINER_CREDENTIALS_FULL_URI or AWS_CONTAINER_CREDENTIALS_RELATIVE_URI.", .{});
    if (selection == .automatic) if (try @import("shared_config.zig").sdkEnv(operation, "AWS_EC2_METADATA_DISABLED")) |disabled| if (disabled.len > 0 and !std.mem.eql(u8, disabled, "false")) return remoteFailure(operation, error.AwsCredentialsUnavailable, "EC2 Instance Metadata Service access disabled", .{});
    const shared = @import("shared_config.zig");
    const configured_endpoint = (try shared.sdkEnv(operation, "AWS_EC2_METADATA_SERVICE_ENDPOINT")) orelse (try shared.setting(operation, options, "ec2_metadata_service_endpoint"));
    const base = nonempty(configured_endpoint) orelse resolve_metadata: {
        const mode = (try shared.sdkEnv(operation, "AWS_EC2_METADATA_SERVICE_ENDPOINT_MODE")) orelse (try shared.setting(operation, options, "ec2_metadata_service_endpoint_mode")) orelse "IPv4";
        if (std.mem.eql(u8, mode, "IPv4")) break :resolve_metadata "http://169.254.169.254";
        if (std.mem.eql(u8, mode, "IPv6")) break :resolve_metadata "http://[fd00:ec2::254]";
        return remoteFailure(operation, error.InvalidAwsMetadataEndpointMode, "Unsupported endpoint mode: {s}. Select from IPv4,IPv6", .{mode});
    };
    const base_uri = try metadataUrl(operation, base);
    var token_uri = base_uri;
    token_uri.path = .{ .raw = "/latest/api/token" };
    token_uri.query = null;
    token_uri.fragment = null;
    const token_response = metadataRequest(operation, .PUT, token_uri, &.{.{ .name = "x-aws-ec2-metadata-token-ttl-seconds", .value = "21600" }}, .legacy) catch |err| blk: {
        if (err == error.Canceled or err == error.OutOfMemory) return err;
        if (operation.errorMessage) |slot| slot.* = prior_message;
        break :blk null;
    };
    defer if (token_response) |value| a.free(value.body);
    var metadata_headers: t.ProviderHeaders = &.{};
    if (token_response) |token_result| {
        if (token_result.status == 400) return remoteFailure(operation, error.AwsMetadataTokenFailed, "EC2 Metadata token request returned error", .{});
        if (token_result.status >= 200 and token_result.status < 300) metadata_headers = try a.dupe(t.ProviderHeader, &.{.{ .name = "x-aws-ec2-metadata-token", .value = token_result.body }});
        if (token_result.status < 200 or token_result.status >= 300) if (operation.errorMessage) |slot| {
            slot.* = prior_message;
        };
    }
    if (metadata_headers.len == 0) {
        const process_setting = try shared.sdkEnv(operation, "AWS_EC2_METADATA_V1_DISABLED");
        const blocked = if (process_setting) |value| value.len > 0 and !std.mem.eql(u8, value, "false") else if (try shared.settingValue(operation, options, "ec2_metadata_v1_disabled")) |value| j.truthy(value) and !(value == .string and std.mem.eql(u8, value.string, "false")) else false;
        if (blocked) return remoteFailure(operation, error.AwsMetadataV1Disabled, "AWS EC2 Metadata v1 fallback has been blocked by AWS SDK configuration in the following: [{s}].", .{if (process_setting != null) "process environment variable (AWS_EC2_METADATA_V1_DISABLED)" else "config file profile (ec2_metadata_v1_disabled)"});
    }
    var role_uri = base_uri;
    role_uri.path = .{ .raw = "/latest/meta-data/iam/security-credentials/" };
    role_uri.query = null;
    role_uri.fragment = null;
    const role_response = try metadataRequest(operation, .GET, role_uri, metadata_headers, .legacy);
    defer a.free(role_response.body);
    if (role_response.status < 200 or role_response.status >= 300) return error.AwsMetadataRoleFailed;
    const role = try @import("../../utils/unicode.zig").trimWhitespace(a, role_response.body);
    defer a.free(role);
    const role_path = try std.fmt.allocPrint(a, "/latest/meta-data/iam/security-credentials/{s}", .{role});
    defer a.free(role_path);
    const credential_uri = try legacyRequestUri(operation, base_uri, role_path, error.AwsMetadataRequestPathInvalid);
    const response = try metadataRequest(operation, .GET, credential_uri, metadata_headers, .legacy);
    defer a.free(response.body);
    if (response.status < 200 or response.status >= 300) return error.AwsMetadataCredentialsFailed;
    var resolved_credentials = try parse(operation, try credentialJson(operation, response.body), .legacy);
    if (resolved_credentials.expiration) |expiry| if (expiry < std.Io.Clock.real.now(io).toMilliseconds()) {
        var random: [8]u8 = undefined;
        try io.randomSecure(&random);
        const jitter = std.mem.readInt(u64, &random, .little) % 300;
        resolved_credentials.expiration = try @import("../../utils/datetime.zig").expiresAfter(io, @floatFromInt(300 + jitter));
    };
    return resolved_credentials;
}
const MetadataResponse = struct { status: u16, body: []const u8 };
const MetadataKind = enum { legacy, modern };
fn metadataIoFailure(operation: m.OperationContext, kind: MetadataKind, err: anyerror) anyerror {
    if (kind == .modern or err == error.Canceled or err == error.OutOfMemory or err == error.ConcurrencyUnavailable) return err;
    return remoteFailure(operation, error.AwsMetadataTransportProviderError, "{s}", .{if (err == error.RequestTimeout or err == error.ConnectionTimedOut) "TimeoutError from instance metadata service" else "Unable to connect to instance metadata service"});
}
fn validateMetadataHeaders(operation: m.OperationContext, extra: t.ProviderHeaders, kind: MetadataKind) !void {
    if (extra.len == 0) return;
    var invalid = try @import("../../regex/regexp.zig").Regex.init(operation.allocator, operation.io, "[^\\t\\x20-\\x7e\\x80-\\xff]", .{});
    defer invalid.deinit();
    for (extra) |header| if (header.value) |text| if (try invalid.execSensitive(operation.allocator, text, 0)) |found| {
        var match = found;
        defer match.deinit();
        return remoteFailure(operation, if (kind == .legacy) error.AwsMetadataRequestHeaderTypeError else error.AwsContainerHeaderProviderError, "Invalid character in header content [\"{s}\"]", .{header.name});
    };
}
fn metadataRequest(operation: m.OperationContext, method: std.http.Method, url: std.Uri, extra: t.ProviderHeaders, kind: MetadataKind) !MetadataResponse {
    const a = operation.allocator;
    // Legacy httpRequest always calls node:http, including configured HTTPS
    // endpoints. Its protocol error precedes header construction/network I/O.
    if (kind == .legacy and !std.ascii.eqlIgnoreCase(url.scheme, "http")) return remoteFailure(operation, error.AwsMetadataRequestProtocolTypeError, "Protocol \"{s}:\" not supported. Expected \"http:\"", .{url.scheme});
    try validateMetadataHeaders(operation, extra, kind);
    var target = url;
    target.user = null;
    target.password = null;
    target.fragment = null;
    // Node request options select the protocol default port for numeric zero.
    if (target.port == 0) target.port = null;
    var response = operation.transport.send(a, operation.io, .{ .method = method, .uri = target, .headers = extra, .timeoutMs = if (kind == .legacy) 1000 else null, .connectionTimeoutMs = if (kind == .modern) 1000 else null, .proxyResolved = true, .followRedirects = false, .decompressResponse = false, .preserveUriComponents = true }) catch |err| return metadataIoFailure(operation, kind, err);
    var released = false;
    defer if (!released) response.deinit();
    if (kind == .legacy and (response.status < 200 or response.status >= 300)) {
        const status = response.status;
        response.abort();
        released = true;
        if (operation.errorMessage) |slot| slot.* = "Error response received from instance metadata service";
        return .{ .status = status, .body = "" };
    }
    const bytes = (if (kind == .modern) response.readAll(a, .unlimited) else response.readAllWithIdleTimeout(a, operation.io, 1000)) catch |err| {
        if (kind == .modern and err != error.Canceled and err != error.OutOfMemory) return remoteFailure(operation, error.AwsContainerResponseReadError, "{s}", .{@errorName(err)});
        return metadataIoFailure(operation, kind, err);
    };
    defer a.free(bytes);
    return .{ .status = response.status, .body = try std.fmt.allocPrint(a, "{f}", .{std.unicode.fmtUtf8(bytes)}) };
}
fn validateModernContainerEndpoint(operation: m.OperationContext, url: std.Uri) !void {
    const host = if (url.host) |value| try value.toRawMaybeAlloc(operation.allocator) else "";
    if (!std.ascii.eqlIgnoreCase(url.scheme, "https")) {
        const bare = std.mem.trim(u8, host, "[]");
        const loopback = if (std.Io.net.Ip4Address.parse(bare, 0)) |ip| ip.bytes[0] == 127 else |_| false;
        const ip6 = std.Io.net.Ip6Address.parse(bare, 0) catch null;
        const ipv6_loopback = if (ip6) |ip| std.mem.eql(u8, &ip.bytes, &std.Io.net.Ip6Address.loopback(0).bytes) else false;
        if (!loopback and !ipv6_loopback and !std.mem.eql(u8, bare, "localhost") and !std.mem.eql(u8, bare, "169.254.170.2") and !std.mem.eql(u8, bare, "169.254.170.23") and !std.mem.eql(u8, bare, "fd00:ec2::23")) return remoteFailure(operation, error.AwsHttpCredentialsUrlRejected, "URL not accepted. It must either be HTTPS or match one of the following:\n  - loopback CIDR 127.0.0.0/8 or [::1/128]\n  - ECS container host 169.254.170.2\n  - EKS container host 169.254.170.23 or [fd00:ec2::23]", .{});
    }
}
fn modernContainerUri(a: std.mem.Allocator, url: std.Uri) !std.Uri {
    var result = url;
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    if (url.query) |query| {
        var record = j.map(scratch);
        for (try uri.decodeFormPairs(scratch, query)) |pair| {
            // Assignment of a string to Object.prototype.__proto__ creates no own
            // property in the source's ordinary-object reduction.
            if (!std.mem.eql(u8, pair.name, "__proto__")) try j.put(scratch, &record, pair.name, j.str(pair.value));
        }
        const Entry = struct { pair: uri.Pair, units: []const u16 };
        const entries = try scratch.alloc(Entry, record.object.count());
        var iterator = record.object.iterator();
        var index: usize = 0;
        while (iterator.next()) |entry| : (index += 1) entries[index] = .{ .pair = .{ .name = entry.key_ptr.*, .value = entry.value_ptr.string }, .units = try std.unicode.wtf8ToWtf16LeAlloc(scratch, entry.key_ptr.*) };
        std.mem.sort(Entry, entries, {}, struct {
            fn less(_: void, left: Entry, right: Entry) bool {
                for (0..@min(left.units.len, right.units.len)) |unit_index| {
                    const l = std.mem.littleToNative(u16, left.units[unit_index]);
                    const r = std.mem.littleToNative(u16, right.units[unit_index]);
                    if (l != r) return l < r;
                }
                return left.units.len < right.units.len;
            }
        }.less);
        const pairs = try scratch.alloc(uri.Pair, entries.len);
        for (entries, pairs) |entry, *pair| pair.* = entry.pair;
        const encoded = try uri.encode(scratch, pairs);
        result.query = if (encoded.len > 0) .{ .percent_encoded = encoded } else null;
    }
    if (url.fragment) |fragment| if (!fragment.isEmpty()) {
        // createGetRequest retains URL.hash's leading '#'; NodeHttpHandler
        // appends its own '#'. Compose that target with the standard URI writer
        // before putting it in the standard client's encoded path component.
        var hash = std.Io.Writer.Allocating.init(scratch);
        defer hash.deinit();
        try hash.writer.writeByte('#');
        try fragment.formatFragment(&hash.writer);
        var components = result;
        components.fragment = .{ .percent_encoded = hash.written() };
        var target = std.Io.Writer.Allocating.init(scratch);
        defer target.deinit();
        try components.writeToStream(&target.writer, .{ .path = true, .query = true, .fragment = true });
        result.path = .{ .percent_encoded = try a.dupe(u8, target.written()) };
        result.query = null;
    };
    if (result.query) |query| result.query = try t.cloneInto(std.Uri.Component, a, query);
    result.fragment = null;
    return result;
}
fn containerCredentials(operation: m.OperationContext, options: t.StreamOptions, url: std.Uri, modern: bool) !Credentials {
    _ = options;
    const a = operation.allocator;
    if (!std.ascii.eqlIgnoreCase(url.scheme, "http") and !std.ascii.eqlIgnoreCase(url.scheme, "https")) return error.InvalidAwsContainerEndpoint;
    var token = nonempty(try @import("shared_config.zig").sdkEnv(operation, "AWS_CONTAINER_AUTHORIZATION_TOKEN"));
    if (modern) if (operation.authContext) |context| if (context.inputs.awsContainerAuthorizationToken) |value| {
        token = value;
    };
    if (modern) if (token) |value| if (std.mem.find(u8, value, "\r\n") != null) return remoteFailure(operation, error.InvalidAwsContainerAuthorizationToken, "Authorization token contains invalid \\r\\n sequence.", .{});
    const extra: t.ProviderHeaders = if (token) |value| try a.dupe(t.ProviderHeader, &.{.{ .name = "Authorization", .value = value }}) else &.{};
    const kind: MetadataKind = if (modern) .modern else .legacy;
    const response = try metadataRequest(operation, .GET, if (modern) try modernContainerUri(a, url) else url, extra, kind);
    defer a.free(response.body);
    if (modern and response.status != 200) {
        if (response.status >= 400 and response.status < 500) {
            var temporary = std.heap.ArenaAllocator.init(a);
            defer temporary.deinit();
            const value = @import("../../json/value.zig").parseFromSliceLeaky(std.json.Value, temporary.allocator(), response.body, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |err| parsed: {
                if (err == error.OutOfMemory) return err;
                break :parsed j.map(temporary.allocator());
            };
            if (value == .null) return remoteFailure(operation, error.AwsContainerResponseTypeError, "Cannot read properties of null (reading 'Code')", .{});
        }
        return remoteFailure(operation, error.AwsContainerCredentialsFailed, "Server responded with status: {d}", .{response.status});
    }
    if (response.status < 200 or response.status >= 300) return error.AwsContainerCredentialsFailed;
    return parse(operation, try credentialJson(operation, response.body), kind);
}
fn invalidMetadataShape(operation: m.OperationContext, kind: MetadataKind) anyerror {
    return remoteFailure(operation, error.InvalidAwsMetadataCredentials, "{s}", .{if (kind == .modern) "HTTP credential provider response not of the required format, an object matching: { AccessKeyId: string, SecretAccessKey: string, Token: string, Expiration: string(rfc3339) }" else "Invalid response received from instance metadata service."});
}
fn parse(operation: m.OperationContext, value: std.json.Value, kind: MetadataKind) !Credentials {
    if (kind == .modern and value == .null) return remoteFailure(operation, error.AwsContainerResponseTypeError, "Cannot read properties of null (reading 'AccessKeyId')", .{});
    const access = j.field(value, "AccessKeyId") orelse return invalidMetadataShape(operation, kind);
    const secret = j.field(value, "SecretAccessKey") orelse return invalidMetadataShape(operation, kind);
    const token = j.field(value, "Token") orelse return invalidMetadataShape(operation, kind);
    const expiration = j.field(value, "Expiration") orelse return invalidMetadataShape(operation, kind);
    if (kind == .modern) return .{ .accessKeyId = access, .secretAccessKey = secret, .sessionToken = token, .expiration = @import("../../utils/datetime.zig").parseSmithyRfc3339(operation.allocator, expiration, operation.errorMessage) catch |err| {
        if (err == error.Canceled or err == error.OutOfMemory) return err;
        return error.AwsContainerResponseDateError;
    } };
    const milliseconds = try @import("../../utils/datetime.zig").parseJavaScriptDate(operation.allocator, operation.io, expiration);
    return .{ .accessKeyId = access, .secretAccessKey = secret, .sessionToken = token, .expiration = if (std.math.isNan(milliseconds)) null else @intFromFloat(milliseconds), .expirationInvalid = std.math.isNan(milliseconds) };
}
fn inputRegion(operation: m.OperationContext, model: t.Model, options: t.StreamOptions) ![]const u8 {
    var arn = try @import("../../regex/regexp.zig").Regex.init(operation.allocator, operation.io, "^arn:aws(?:-[a-z0-9-]+)?:bedrock:([a-z0-9-]+):", .{});
    defer arn.deinit();
    if (try arn.exec(operation.allocator, model.id, 0)) |found| {
        var selected = found;
        defer selected.deinit();
        return (try selected.text(operation.allocator, 1)).?;
    }
    if (c.extra(options, "region")) |value| if (j.text(value)) |r| if (r.len > 0) return r;
    if (try env(operation, options, "AWS_REGION")) |r| return r;
    if (try env(operation, options, "AWS_DEFAULT_REGION")) |r| return r;
    if (try ambientProfile(operation)) {
        const shared = @import("shared_config.zig");
        var files = try shared.Files.init(operation, options);
        defer files.deinit();
        if (try files.get(try shared.profile(operation, options), "region")) |value| return value;
        if (try instanceMetadataRegion(operation)) |value| return value;
        if (operation.errorMessage) |slot| slot.* = "Region is missing";
        return error.AwsRegionMissing;
    }
    return (try standardEndpointRegion(operation.allocator, model.baseUrl)) orelse "us-east-1";
}
const isFipsRegion = @import("region.zig").isFipsRegion;
const realRegion = @import("region.zig").realRegion;
pub fn region(operation: m.OperationContext, model: t.Model, options: t.StreamOptions) ![]const u8 {
    return realRegion(operation, try inputRegion(operation, model, options));
}
fn regionMetadataAttempt(operation: m.OperationContext, method: std.http.Method, target: std.Uri, extra: t.ProviderHeaders) ![]const u8 {
    var response = try operation.transport.send(operation.allocator, operation.io, .{ .method = method, .uri = target, .headers = extra, .proxyResolved = true, .followRedirects = false, .decompressResponse = false });
    var completed = false;
    defer if (completed) response.deinit() else response.abort();
    if (response.status < 200 or response.status >= 300) return error.AwsMetadataRegionFailed;
    const bytes = try response.readAll(operation.allocator, .unlimited);
    defer operation.allocator.free(bytes);
    const text = try std.fmt.allocPrint(operation.allocator, "{f}", .{std.unicode.fmtUtf8(bytes)});
    completed = true;
    return text;
}
fn regionMetadataTimeout(io: std.Io) std.Io.Cancelable!void {
    try io.sleep(.fromMilliseconds(1000), .awake);
}
fn regionMetadataRequest(operation: m.OperationContext, method: std.http.Method, target: std.Uri, extra: t.ProviderHeaders) ![]const u8 {
    const Outcome = union(enum) { response: anyerror![]const u8, timeout: std.Io.Cancelable!void };
    var buffer: [2]Outcome = undefined;
    var select = std.Io.Select(Outcome).init(operation.io, &buffer);
    defer while (select.cancel()) |outcome| switch (outcome) {
        .response => |result| {
            if (result) |text| operation.allocator.free(text) else |_| {}
        },
        .timeout => {},
    };
    try select.concurrent(.response, regionMetadataAttempt, .{ operation, method, target, extra });
    try select.concurrent(.timeout, regionMetadataTimeout, .{operation.io});
    return switch (try select.await()) {
        .response => |result| result,
        .timeout => |result| {
            try result;
            return error.RequestTimeout;
        },
    };
}
fn discoverInstanceMetadataRegion(operation: m.OperationContext) !?[]const u8 {
    const shared = @import("shared_config.zig");
    const a = operation.allocator;
    var target = if (nonempty(try shared.sdkEnv(operation, "AWS_EC2_METADATA_SERVICE_ENDPOINT"))) |value| try uri.parseHttpUrl(a, value) else try std.Uri.parse(if (std.mem.eql(u8, (try shared.sdkEnv(operation, "AWS_EC2_METADATA_SERVICE_ENDPOINT_MODE")) orelse "", "IPv6")) "http://[fd00:ec2::254]" else "http://169.254.169.254");
    if (target.host == null) return error.InvalidAwsMetadataEndpoint;
    // The pinned region helper always uses node:http, even for an HTTPS
    // override URL; only its parsed hostname and explicit port are forwarded.
    target.scheme = "http";
    target.user = null;
    target.password = null;
    target.query = null;
    target.fragment = null;
    target.path = .{ .raw = "/latest/api/token" };
    const token = try regionMetadataRequest(operation, .PUT, target, &.{.{ .name = "x-aws-ec2-metadata-token-ttl-seconds", .value = "21600" }});
    defer a.free(token);
    target.path = .{ .raw = "/latest/meta-data/placement/region" };
    const response = try regionMetadataRequest(operation, .GET, target, &.{.{ .name = "x-aws-ec2-metadata-token", .value = token }});
    defer a.free(response);
    const value = try @import("../../utils/unicode.zig").trimWhitespace(a, response);
    if (value.len == 0) {
        a.free(value);
        return null;
    }
    return value;
}
fn instanceMetadataRegion(operation: m.OperationContext) !?[]const u8 {
    const shared = @import("shared_config.zig");
    if (nonempty(try shared.sdkEnv(operation, "AWS_EC2_METADATA_DISABLED")) != null) return null;
    const cache = if (operation.authContext) |context| context.awsImdsRegionNegativeCache else null;
    if (cache) |deadline| if (std.Io.Clock.real.now(operation.io).toMilliseconds() < deadline.load(.acquire)) return null;
    const found = discoverInstanceMetadataRegion(operation) catch |err| blk: {
        if (err == error.Canceled or err == error.OutOfMemory) return err;
        break :blk null;
    };
    if (found == null) if (cache) |deadline| deadline.store(std.math.add(i64, std.Io.Clock.real.now(operation.io).toMilliseconds(), 60_000) catch std.math.maxInt(i64), .release);
    return found;
}
fn standardEndpointRegion(a: std.mem.Allocator, base_url: []const u8) !?[]const u8 {
    const endpoint = uri.parseHttpUrl(a, base_url) catch |err| {
        if (err == error.OutOfMemory or err == error.Canceled) return err;
        return null;
    };
    const host = try (endpoint.host orelse return null).toRawMaybeAlloc(a);
    var regex = try @import("../../regex/regexp.zig").Regex.init(a, null, "^bedrock-runtime(?:-fips)?\\.([a-z0-9-]+)\\.amazonaws\\.com(?:\\.cn)?$", .{ .ignoreCase = true });
    defer regex.deinit();
    var selected = (try regex.exec(a, host, 0)) orelse return null;
    defer selected.deinit();
    return try std.ascii.allocLowerString(a, (try selected.text(a, 1)).?);
}
fn ambientProfile(operation: m.OperationContext) !bool {
    const context = operation.authContext orelse return false;
    return nonempty(try context.env(operation.allocator, operation.io, "AWS_PROFILE")) != null;
}
pub const ResolvedEndpoint = struct { uri: std.Uri, signingRegion: []const u8 };
pub fn resolveRequestEndpoint(operation: m.OperationContext, model: t.Model, options: t.StreamOptions, payload: std.json.Value) !ResolvedEndpoint {
    const configured = try env(operation, options, "AWS_REGION") != null or try env(operation, options, "AWS_DEFAULT_REGION") != null or nonempty(if (c.extra(options, "region")) |value| j.text(value) else null) != null;
    const explicit: ?[]const u8 = if (try standardEndpointRegion(operation.allocator, model.baseUrl) == null or (!configured and !try ambientProfile(operation))) model.baseUrl else null;
    const region_input = try inputRegion(operation, model, options);
    const signing_region = try realRegion(operation, region_input);
    const base = try @import("endpoints.zig").resolveBedrock(operation, options, signing_region, explicit, isFipsRegion(region_input));
    return .{ .uri = try @import("../../api/bedrock_converse_stream.zig").endpoint(operation.allocator, base, payload, operation.errorMessage), .signingRegion = signing_region };
}
pub const Prepared = struct { uri: std.Uri, headers: t.ProviderHeaders, requestSentAt: ?i64 = null, credentials: ?Credentials = null };
pub fn prepareRequest(operation: m.OperationContext, model: t.Model, options: t.StreamOptions, original_uri: std.Uri, original_headers: t.ProviderHeaders, body: []const u8) !Prepared {
    return prepareRequestWithClock(operation, model, options, original_uri, original_headers, body, 0);
}
pub fn prepareRequestWithClock(operation: m.OperationContext, model: t.Model, options: t.StreamOptions, original_uri: std.Uri, original_headers: t.ProviderHeaders, body: []const u8, clock_offset_ms: f64) !Prepared {
    return prepareRequestWithCredentials(operation, model, options, original_uri, original_headers, body, clock_offset_ms, null);
}
pub fn prepareRequestWithCredentials(operation: m.OperationContext, model: t.Model, options: t.StreamOptions, original_uri: std.Uri, original_headers: t.ProviderHeaders, body: []const u8, clock_offset_ms: f64, resolved_credentials: ?Credentials) !Prepared {
    return prepareRequestWithRegion(operation, model, options, original_uri, original_headers, body, clock_offset_ms, resolved_credentials, null);
}
pub fn prepareRequestWithRegion(operation: m.OperationContext, model: t.Model, options: t.StreamOptions, original_uri: std.Uri, original_headers: t.ProviderHeaders, body: []const u8, clock_offset_ms: f64, resolved_credentials: ?Credentials, resolved_region: ?[]const u8) !Prepared {
    return prepareRequestWithRegionAndRefresh(operation, model, options, original_uri, original_headers, body, clock_offset_ms, resolved_credentials, resolved_region, null);
}
pub fn prepareRequestWithRegionAndRefresh(operation: m.OperationContext, model: t.Model, options: t.StreamOptions, original_uri: std.Uri, original_headers: t.ProviderHeaders, body: []const u8, clock_offset_ms: f64, resolved_credentials: ?Credentials, resolved_region: ?[]const u8, refresh: ?*PassiveRefresh) !Prepared {
    const a = operation.allocator;
    const io = operation.io;
    const signing_region = resolved_region orelse try region(operation, model, options);
    const request_uri = original_uri;
    var filtered: std.ArrayList(t.ProviderHeader) = .empty;
    for (original_headers) |header| {
        const lower = try std.ascii.allocLowerString(a, header.name);
        if (std.mem.eql(u8, lower, "host") or std.mem.eql(u8, lower, "authorization") or std.mem.startsWith(u8, lower, "x-amz-")) continue;
        try filtered.append(a, header);
    }
    const bearer = try bearerToken(operation, options);
    const skip = if (try env(operation, options, "AWS_BEDROCK_SKIP_AUTH")) |v| std.mem.eql(u8, v, "1") else false;
    if (bearer != null and !skip) {
        try filtered.append(a, .{ .name = "authorization", .value = try std.fmt.allocPrint(a, "Bearer {s}", .{bearer.?}) });
        return .{ .uri = request_uri, .headers = filtered.items };
    }
    const credentials = if (skip) Credentials{ .accessKeyId = "dummy-access-key", .secretAccessKey = "dummy-secret-key" } else resolved_credentials orelse if (refresh) |owner| try owner.resolveInitial(operation, options) else try resolve(operation, options);
    try credentials.validateSigning(operation);
    // Smithy's prepareRequest removes legacy Date before signing/sending.
    var header_index: usize = 0;
    while (header_index < filtered.items.len) {
        if (std.ascii.eqlIgnoreCase(filtered.items[header_index].name, "date")) _ = filtered.orderedRemove(header_index) else header_index += 1;
    }
    const sent_at = std.Io.Clock.real.now(io).toMilliseconds();
    const timestamp = @floor((@as(f64, @floatFromInt(sent_at)) + clock_offset_ms) / 1000);
    if (!std.math.isFinite(timestamp) or timestamp < 0 or timestamp >= @as(f64, @floatFromInt(std.math.maxInt(u64)))) return error.InvalidAwsSigningTimestamp;
    const timestamp_seconds: u64 = @intFromFloat(timestamp);
    try credentials.validateSigningToken(operation);
    const signing = @import("../signing.zig");
    var pairs: std.ArrayList(signing.Pair) = .empty;
    for (filtered.items) |header| if (header.value) |value| {
        try pairs.append(a, .{ .name = signing.view(header.name), .value = signing.view(value) });
    };
    var output = try signing.aws(a, io, .{ .method = signing.view("POST"), .uri = signing.view(try uri.render(a, request_uri)), .body = signing.view(body), .headers = signing.Pairs.fromSlice(pairs.items), .region = signing.view(signing_region), .service = signing.view("bedrock"), .access_key = signing.view(credentials.accessKeyId), .secret_key = signing.view(credentials.secretAccessKey), .session_token = signing.view(credentials.sessionToken orelse ""), .timestamp_seconds = timestamp_seconds });
    defer output.deinit();
    for (try output.headers.slice()) |header| try filtered.append(a, .{ .name = try a.dupe(u8, try header.name.slice()), .value = try a.dupe(u8, try header.value.slice()) });
    return .{ .uri = request_uri, .headers = try headers.merge(a, &.{filtered.items}), .requestSentAt = sent_at, .credentials = credentials };
}
