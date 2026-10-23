//! Federation and token caching.
const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const transport = @import("../transport/http.zig");
pub const Config = struct { baseUrl: []const u8, organization: []const u8, rule: []const u8, identityToken: []const u8, workspace: ?[]const u8 = null, serviceAccount: ?[]const u8 = null };
const Token = struct { access: []const u8, expires: f64 };
fn nowAsSeconds(io: std.Io) f64 {
    return @floor(@as(f64, @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds())) / 1000);
}
fn remember(slot: ?*?[]const u8, message: []const u8) void {
    if (slot) |value| value.* = message;
}
fn fetchText(a: std.mem.Allocator, bytes: []const u8) ![]const u8 {
    const decoded = try std.fmt.allocPrint(a, "{f}", .{std.unicode.fmtUtf8(bytes)});
    defer a.free(decoded);
    var reader = std.Io.Reader.fixed(decoded);
    if (std.mem.startsWith(u8, decoded, &.{ 0xef, 0xbb, 0xbf })) reader.toss(3);
    return a.dupe(u8, reader.buffered());
}
fn redactValue(a: std.mem.Allocator, value: std.json.Value) anyerror!std.json.Value {
    if (value == .string) {
        const raw = value.string;
        const parsed = j.parseFromSliceLeaky(std.json.Value, a, raw, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |err| {
            if (err == error.OutOfMemory) return err;
            const units = try std.unicode.wtf8ToWtf16LeAlloc(a, raw);
            defer a.free(units);
            if (units.len <= 2000) return value;
            const prefix = try std.unicode.wtf16LeToWtf8Alloc(a, units[0..2000]);
            defer a.free(prefix);
            return .{ .string = try std.fmt.allocPrint(a, "{s}... <{d} more chars>", .{ prefix, units.len - 2000 }) };
        };
        return .{ .string = try j.stringify(a, try redactValue(a, parsed), .{}) };
    }
    var safe: std.json.Value = .null;
    if (value == .object) {
        safe = j.map(a);
        var iterator = value.object.iterator();
        while (iterator.next()) |entry| {
            for ([_][]const u8{ "error", "error_description", "error_uri" }) |key| if (std.mem.eql(u8, key, entry.key_ptr.*)) {
                try j.put(a, &safe, key, entry.value_ptr.*);
                break;
            };
        }
    }
    return safe;
}
fn redact(a: std.mem.Allocator, raw: []const u8) ![]const u8 {
    return (try redactValue(a, .{ .string = raw })).string;
}
fn invalidFields(a: std.mem.Allocator, slot: ?*?[]const u8, bytes: []const u8) !void {
    remember(slot, try std.fmt.allocPrint(a, "Token endpoint response missing required fields: {s}", .{try redact(a, bytes)}));
}
fn exchange(a: std.mem.Allocator, io: std.Io, network: transport.Transport, config: Config, error_message: ?*?[]const u8) !Token {
    const uri = @import("../transport/uri.zig");
    const base_url = std.mem.trimEnd(u8, config.baseUrl, "/");
    const base = uri.parseHttpUrl(a, base_url) catch |err| {
        if (err == error.OutOfMemory or err == error.Canceled) return err;
        remember(error_message, try std.fmt.allocPrint(a, "Invalid token endpoint base URL \"{s}\": TypeError: Invalid URL", .{base_url}));
        return error.InvalidAnthropicFederationEndpoint;
    };
    const host = try (base.host orelse {
        remember(error_message, try std.fmt.allocPrint(a, "Invalid token endpoint base URL \"{s}\": TypeError: Invalid URL", .{base_url}));
        return error.InvalidAnthropicFederationEndpoint;
    }).toRawMaybeAlloc(a);
    const bare = std.mem.trim(u8, host, "[]");
    if (!std.ascii.eqlIgnoreCase(base.scheme, "https") and !(std.ascii.eqlIgnoreCase(base.scheme, "http") and (std.ascii.eqlIgnoreCase(bare, "localhost") or std.mem.eql(u8, bare, "127.0.0.1") or std.mem.eql(u8, bare, "::1")))) {
        remember(error_message, try std.fmt.allocPrint(a, "Refusing to send credential over non-https token endpoint \"{s}\"", .{base_url}));
        return error.InsecureAnthropicFederationEndpoint;
    }
    const decoded = config.identityToken;
    const assertion = try @import("../utils/unicode.zig").trimWhitespace(a, decoded);
    defer a.free(assertion);
    if (assertion.len == 0) {
        remember(error_message, "Identity token is empty");
        return error.EmptyAnthropicIdentityToken;
    }
    const assertion_length = try @import("../utils/unicode.zig").utf16Length(assertion);
    if (assertion_length > 16 * 1024) {
        remember(error_message, try std.fmt.allocPrint(a, "Identity token is {d} KiB, exceeds the 16 KiB assertion limit", .{assertion_length / 1024 + @intFromBool(assertion_length % 1024 != 0)}));
        return error.AnthropicIdentityTokenTooLong;
    }
    const payload = try @import("../json/value.zig").stringify(a, .{
        .grant_type = "urn:ietf:params:oauth:grant-type:jwt-bearer",
        .assertion = assertion,
        .federation_rule_id = config.rule,
        .organization_id = config.organization,
        .service_account_id = if (config.serviceAccount) |value| (if (value.len > 0) value else null) else null,
        .workspace_id = if (config.workspace) |value| (if (value.len > 0) value else null) else null,
    }, .{ .emit_null_optional_fields = false });
    const endpoint = try uri.parseHttpUrl(a, try std.mem.concat(a, u8, &.{ base_url, "/v1/oauth/token" }));
    var response = network.send(a, io, .{ .method = .POST, .uri = endpoint, .body = payload, .headers = &.{
        .{ .name = "content-type", .value = "application/json" },                                      .{ .name = "anthropic-beta", .value = "oauth-2025-04-20,oidc-federation-2026-04-01" },
        .{ .name = "user-agent", .value = "anthropic-sdk-typescript/0.129.0 oidcFederationProvider" },
    } }) catch |err| {
        if (err != error.Canceled and err != error.OutOfMemory) remember(error_message, try std.fmt.allocPrint(a, "Failed to reach token endpoint {s}: {s}", .{ try @import("../transport/uri.zig").render(a, endpoint), @errorName(err) }));
        return err;
    };
    defer response.deinit();
    if (response.status < 200 or response.status >= 300) {
        const received: ?[]const u8 = response.readAll(a, .unlimited) catch |err| blk: {
            if (err == error.Canceled or err == error.OutOfMemory) return err;
            break :blk null;
        };
        defer if (received) |bytes| a.free(bytes);
        const raw = try fetchText(a, received orelse "");
        defer a.free(raw);
        const request_id = @import("../utils/headers.zig").get(response.headers, "Request-Id");
        const request = if (request_id != null and request_id.?.len > 0) try std.fmt.allocPrint(a, " (request-id {s})", .{request_id.?}) else "";
        const hint = if (response.status == 401) (if (config.workspace != null and config.workspace.?.len > 0) " Ensure your federation rule matches your identity token. View your authentication events in the Workload identity page of Claude Console for more details." else " Ensure your federation rule matches your identity token. If your federation rule is scoped to multiple workspaces, set the ANTHROPIC_WORKSPACE_ID environment variable, the 'workspace_id' config key, or the `workspaceId` option. View your authentication events in the Workload identity page of Claude Console for more details.") else "";
        remember(error_message, try std.fmt.allocPrint(a, "Token exchange failed with status {d}{s}: {s}{s}", .{ response.status, request, try redact(a, raw), hint }));
        return error.AnthropicFederationTokenExchangeFailed;
    }
    const received = response.body.readAllocShort(a, 1 << 20) catch |err| return response.readError(err);
    defer a.free(received);
    const bytes = try fetchText(a, received);
    defer a.free(bytes);
    const value = @import("../json/value.zig").parseFromSliceLeaky(std.json.Value, a, bytes, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |err| {
        if (err == error.OutOfMemory) return err;
        remember(error_message, try std.fmt.allocPrint(a, "Token endpoint returned non-JSON response (status {d})", .{response.status}));
        return error.InvalidAnthropicFederationToken;
    };
    if (value == .null) {
        remember(error_message, "Cannot read properties of null (reading 'access_token')");
        return error.InvalidAnthropicFederationToken;
    }
    const raw_access = j.get(value, "access_token");
    if (!j.truthy(raw_access orelse .null)) {
        remember(error_message, try std.fmt.allocPrint(a, "Token endpoint response missing access_token: {s}", .{try redact(a, bytes)}));
        return error.InvalidAnthropicFederationToken;
    }
    if (j.get(value, "token_type")) |kind| if (j.truthy(kind)) {
        if (kind != .string) {
            remember(error_message, "data.token_type.toLowerCase is not a function");
            return error.InvalidAnthropicFederationTokenType;
        }
        if (!std.ascii.eqlIgnoreCase(kind.string, "bearer")) {
            remember(error_message, try std.fmt.allocPrint(a, "Token endpoint response: unsupported token_type \"{s}\" (want Bearer)", .{kind.string}));
            return error.InvalidAnthropicFederationTokenType;
        }
    };
    const lifetime = try @import("../utils/numbers.zig").coerceValue(a, j.get(value, "expires_in"));
    if (!std.math.isFinite(lifetime)) {
        try invalidFields(a, error_message, bytes);
        return error.InvalidAnthropicFederationExpiry;
    }
    const access = try @import("../utils/diagnostics.zig").formatThrownValue(a, .{ .value = raw_access.? });
    return .{ .access = access, .expires = nowAsSeconds(io) + lifetime };
}
pub const Cache = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    mutex: std.Io.Mutex = .init,
    entries: std.StringHashMapUnmanaged(*Entry) = .{},
    const Entry = struct {
        owner: *Cache,
        arena: std.heap.ArenaAllocator,
        config: Config,
        network: transport.Transport,
        mutex: std.Io.Mutex = .init,
        changed: std.Io.Condition = .init,
        token: ?Token = null,
        lastAdvisoryError: f64 = 0,
        nextForce: bool = false,
        pendingJob: ?*Job = null,
        jobs: std.ArrayList(*Job) = .empty,
        const Job = struct { done: bool = false, token: ?Token = null, failure: ?anyerror = null, failureMessage: ?[]const u8 = null, references: usize = 1, arena: std.heap.ArenaAllocator, advisory: bool, task: ?std.Io.Future(void) = null };
        fn releaseJob(self: *Entry, job: *Job) void {
            job.references -= 1;
            if (job.references == 0) {
                job.arena.deinit();
                self.owner.allocator.destroy(job);
            }
        }
        fn refresh(self: *Entry, job: *Job) void {
            const io = self.owner.io;
            const result = exchange(job.arena.allocator(), io, self.network, self.config, &job.failureMessage);
            const retained: anyerror!Token = if (result) |token| blk: {
                const access = self.owner.allocator.dupe(u8, token.access) catch |err| break :blk err;
                break :blk Token{ .access = access, .expires = token.expires };
            } else |err| err;
            const protection = io.swapCancelProtection(.blocked);
            defer _ = io.swapCancelProtection(protection);
            self.mutex.lockUncancelable(io);
            defer self.mutex.unlock(io);
            self.pendingJob = null;
            if (retained) |token| {
                if (self.token) |old| self.owner.allocator.free(old.access);
                self.token = token;
                job.token = (result catch unreachable);
            } else |err| {
                job.failure = err;
                if (job.advisory) self.lastAdvisoryError = nowAsSeconds(io);
                if (err == error.Canceled) io.recancel();
            }
            job.done = true;
            self.changed.broadcast(io);
        }
        fn prune(self: *Entry) void {
            var index: usize = 0;
            while (index < self.jobs.items.len) {
                const job = self.jobs.items[index];
                if (!job.done or job.references != 1) {
                    index += 1;
                    continue;
                }
                if (job.task) |*task| task.await(self.owner.io);
                _ = self.jobs.orderedRemove(index);
                self.releaseJob(job);
            }
        }
        fn begin(self: *Entry, advisory: bool) !*Job {
            const io = self.owner.io;
            const job = try self.owner.allocator.create(Job);
            errdefer self.owner.allocator.destroy(job);
            job.* = .{ .arena = .init(self.owner.allocator), .advisory = advisory };
            try self.jobs.append(self.owner.allocator, job);
            self.pendingJob = job;
            job.task = io.concurrent(Entry.refresh, .{ self, job }) catch |err| {
                self.pendingJob = null;
                _ = self.jobs.pop();
                job.arena.deinit();
                return err;
            };
            return job;
        }
    };
    pub fn init(a: std.mem.Allocator, io: std.Io) @This() {
        return .{ .allocator = a, .io = io };
    }
    /// Request handles must be released before their transport owner.
    pub fn deinit(self: *@This()) void {
        var it = self.entries.iterator();
        while (it.next()) |item| {
            const entry = item.value_ptr.*;
            for (entry.jobs.items) |job| {
                if (job.task) |*task| task.cancel(self.io);
                entry.releaseJob(job);
            }
            entry.jobs.deinit(self.allocator);
            if (entry.token) |token| self.allocator.free(token.access);
            self.allocator.free(item.key_ptr.*);
            entry.arena.deinit();
            self.allocator.destroy(entry);
        }
        self.entries.deinit(self.allocator);
        self.* = undefined;
    }
    /// Returns a caller-owned copy, so refresh may release superseded tokens.
    pub fn get(self: *@This(), a: std.mem.Allocator, network: transport.Transport, config: Config) ![]const u8 {
        return self.getWithError(a, network, config, null);
    }
    pub fn getWithError(self: *@This(), a: std.mem.Allocator, network: transport.Transport, config: Config, error_message: ?*?[]const u8) ![]const u8 {
        var temporary = std.heap.ArenaAllocator.init(self.allocator);
        defer temporary.deinit();
        const key = try @import("../json/value.zig").stringify(temporary.allocator(), config, .{});
        const entry = blk: {
            try self.mutex.lock(self.io);
            defer self.mutex.unlock(self.io);
            if (self.entries.get(key)) |entry| break :blk entry;
            const entry = try self.allocator.create(Entry);
            errdefer self.allocator.destroy(entry);
            entry.* = .{ .owner = self, .arena = .init(self.allocator), .config = undefined, .network = network };
            errdefer entry.arena.deinit();
            entry.config = try @import("../json/value.zig").parseFromSliceLeaky(Config, entry.arena.allocator(), key, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always });
            const retained = try self.allocator.dupe(u8, key);
            errdefer self.allocator.free(retained);
            try self.entries.put(self.allocator, retained, entry);
            break :blk entry;
        };
        try entry.mutex.lock(self.io);
        defer entry.mutex.unlock(self.io);
        entry.prune();
        const force = entry.nextForce;
        entry.nextForce = false;
        const now = nowAsSeconds(self.io);
        if (!force) if (entry.token) |token| {
            const remaining = token.expires - now;
            if (remaining > 120) return a.dupe(u8, token.access);
            if (remaining > 30) {
                if (entry.pendingJob == null and now - entry.lastAdvisoryError >= 5) {
                    _ = entry.begin(true) catch {
                        entry.lastAdvisoryError = now;
                        return a.dupe(u8, token.access);
                    };
                }
                return a.dupe(u8, token.access);
            }
        };
        // A 401 forces a fresh exchange even while an older exchange is in
        // flight. Existing waiters retain their own job until it completes.
        const job = if (force) try entry.begin(false) else entry.pendingJob orelse try entry.begin(false);
        job.references += 1;
        defer entry.releaseJob(job);
        while (!job.done) try entry.changed.wait(self.io, &entry.mutex);
        if (job.failure) |err| {
            if (job.failureMessage) |message| remember(error_message, try a.dupe(u8, message));
            return err;
        }
        return a.dupe(u8, job.token.?.access);
    }
    pub fn invalidate(self: *@This(), config: Config) !void {
        const key = try @import("../json/value.zig").stringify(self.allocator, config, .{});
        defer self.allocator.free(key);
        try self.mutex.lock(self.io);
        const entry = self.entries.get(key);
        self.mutex.unlock(self.io);
        if (entry) |value| {
            try value.mutex.lock(self.io);
            defer value.mutex.unlock(self.io);
            if (value.token) |token| self.allocator.free(token.access);
            value.token = null;
            value.nextForce = true;
        }
    }
};
fn configuration(operation: @import("../models.zig").OperationContext, model: t.Model, options: t.StreamOptions, original: t.ProviderHeaders) !?Config {
    if (options.authFromClient) return null;
    if (options.apiKey) |key| if (key.len > 0) return null;
    for (original) |header| if (header.value) |value| if ((std.ascii.eqlIgnoreCase(header.name, "authorization") or std.ascii.eqlIgnoreCase(header.name, "x-api-key") or std.ascii.eqlIgnoreCase(header.name, "cf-aig-authorization")) and !try @import("../utils/unicode.zig").isBlank(value)) return null;
    if (!std.mem.eql(u8, model.provider, "anthropic")) return error.MissingAuthentication;
    const env = @import("aws/credentials.zig").env;
    return Config{
        .baseUrl = model.baseUrl,
        .rule = (try env(operation, options, "ANTHROPIC_FEDERATION_RULE_ID")) orelse return error.MissingAuthentication,
        .organization = (try env(operation, options, "ANTHROPIC_ORGANIZATION_ID")) orelse return error.MissingAuthentication,
        .identityToken = (if (operation.authContext) |context| context.inputs.anthropicIdentityToken else null) orelse (try env(operation, options, "ANTHROPIC_IDENTITY_TOKEN")) orelse return error.MissingAuthentication,
        .workspace = try env(operation, options, "ANTHROPIC_WORKSPACE_ID"),
        .serviceAccount = try env(operation, options, "ANTHROPIC_SERVICE_ACCOUNT_ID"),
    };
}
pub fn authenticate(operation: @import("../models.zig").OperationContext, model: t.Model, options: t.StreamOptions, original: t.ProviderHeaders) !t.ProviderHeaders {
    return authenticateWithError(operation, model, options, original, null);
}
pub fn authenticateWithError(operation: @import("../models.zig").OperationContext, model: t.Model, options: t.StreamOptions, original: t.ProviderHeaders, error_message: ?*?[]const u8) !t.ProviderHeaders {
    const config = (try configuration(operation, model, options, original)) orelse return original;
    return authenticatePrepared(operation, config, original, false, error_message);
}
/// Resolve the source client-construction auth decision from request headers
/// before payload callbacks; token exchange remains deferred until sending.
pub fn prepare(operation: @import("../models.zig").OperationContext, model: t.Model, options: t.StreamOptions, error_message: ?*?[]const u8) !?Config {
    return configuration(operation, model, options, options.headers) catch |err| {
        if (err == error.MissingAuthentication) remember(error_message, try std.fmt.allocPrint(operation.allocator, "No API key for provider: {s}", .{model.provider}));
        return err;
    };
}
pub fn authenticatePrepared(operation: @import("../models.zig").OperationContext, prepared: ?Config, original: t.ProviderHeaders, omit_authorization: bool, error_message: ?*?[]const u8) !t.ProviderHeaders {
    const config = prepared orelse return original;
    const access = if (operation.transport.anthropicFederationCache) |cache| try cache.getWithError(operation.allocator, operation.transport, config, error_message) else (try exchange(operation.allocator, operation.io, operation.transport, config, error_message)).access;
    const a = operation.allocator;
    const headers = @import("../utils/headers.zig");
    var result = try headers.merge(a, &.{ &.{.{ .name = "authorization", .value = try std.fmt.allocPrint(a, "Bearer {s}", .{access}) }}, original, if (omit_authorization) &.{.{ .name = "authorization", .value = null }} else &.{} });
    // SDK prepareRequest appends the token-auth beta after caller headers,
    // including when an explicit null removed an earlier beta header.
    var existing: ?[]const u8 = null;
    for (result) |header| if (std.ascii.eqlIgnoreCase(header.name, "anthropic-beta")) {
        existing = header.value;
    };
    var features: std.ArrayList([]const u8) = .empty;
    defer features.deinit(a);
    var contains = false;
    if (existing) |value| {
        var split = std.mem.splitScalar(u8, value, ',');
        while (split.next()) |part| {
            const feature = try @import("../utils/unicode.zig").trimWhitespace(a, part);
            try features.append(a, feature);
            if (std.mem.eql(u8, feature, "oauth-2025-04-20")) contains = true;
        }
    }
    if (!contains) {
        try features.append(a, "oauth-2025-04-20");
        result = try headers.merge(a, &.{ result, &.{.{ .name = "anthropic-beta", .value = try std.mem.join(a, ",", features.items) }} });
    }
    return result;
}
pub fn invalidate(operation: @import("../models.zig").OperationContext, model: t.Model, options: t.StreamOptions, original: t.ProviderHeaders) !void {
    const config = (try configuration(operation, model, options, original)) orelse return;
    return invalidatePrepared(operation, config);
}
pub fn invalidatePrepared(operation: @import("../models.zig").OperationContext, prepared: ?Config) !void {
    const config = prepared orelse return;
    if (operation.transport.anthropicFederationCache) |cache| try cache.invalidate(config);
}
