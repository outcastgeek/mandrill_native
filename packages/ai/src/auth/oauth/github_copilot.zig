const std = @import("std");
const auth = @import("../types.zig");
const t = @import("../../types.zig");
const transport = @import("../../transport/http.zig");
const uri = @import("../../transport/uri.zig");
const j = @import("../../json/value.zig");
const http = @import("http.zig");
const polling = @import("device_code.zig");
const client_id = "Iv1.b507a08c87ecfe98";
pub const copilotHeaders: t.ProviderHeaders = &.{ .{ .name = "User-Agent", .value = "GitHubCopilotChat/0.35.0" }, .{ .name = "Editor-Version", .value = "vscode/1.107.0" }, .{ .name = "Editor-Plugin-Version", .value = "copilot-chat/0.35.0" }, .{ .name = "Copilot-Integration-Id", .value = "vscode-chat" } };
fn domain(a: std.mem.Allocator, input: []const u8) !?[]const u8 {
    var arena = std.heap.ArenaAllocator.init(a);
    defer arena.deinit();
    const scratch = arena.allocator();
    const value = try @import("../../utils/unicode.zig").trimWhitespace(scratch, input);
    if (value.len == 0) return null;
    const url = if (std.mem.find(u8, value, "://") != null) value else try std.fmt.allocPrint(scratch, "https://{s}", .{value});
    if (try @import("../../schema/formats.zig").canParseAbsoluteUrl(scratch, url) == false) return error.InvalidEnterpriseDomain;
    const parsed = uri.parseHttpUrl(scratch, url) catch |err| {
        if (err == error.OutOfMemory) return err;
        return error.InvalidEnterpriseDomain;
    };
    const host = parsed.host orelse return error.InvalidEnterpriseDomain;
    return try a.dupe(u8, try host.toRawMaybeAlloc(scratch));
}
fn enterprise(a: std.mem.Allocator, previous: auth.OAuthCredential) !?[]const u8 {
    const raw = j.field(previous.metadata orelse .null, "enterpriseUrl") orelse return null;
    return domain(a, raw) catch |err| {
        if (err == error.OutOfMemory) return err;
        return null;
    };
}
fn endpoint(a: std.mem.Allocator, host: []const u8, path: []const u8) ![]const u8 {
    return uri.render(a, .{ .scheme = "https", .host = .{ .raw = host }, .path = .{ .raw = path } });
}
/// Match the source token regex, including a match inside another attribute.
/// Its proxy endpoint is returned as a string until the request URL is built.
pub fn getGitHubCopilotBaseUrl(a: std.mem.Allocator, token: ?[]const u8, enterprise_domain: ?[]const u8) ![]const u8 {
    if (token) |bytes| {
        var regex = try @import("../../regex/regexp.zig").Regex.init(a, null, "proxy-ep=([^;]+)", .{});
        defer regex.deinit();
        if (try regex.exec(a, bytes, 0)) |found| {
            var capture = found;
            defer capture.deinit();
            const proxy = (try capture.text(a, 1)).?;
            defer a.free(proxy);
            return if (std.mem.startsWith(u8, proxy, "proxy.")) std.fmt.allocPrint(a, "https://api.{s}", .{proxy["proxy.".len..]}) else std.fmt.allocPrint(a, "https://{s}", .{proxy});
        }
    }
    return if (enterprise_domain) |host| if (host.len > 0) std.fmt.allocPrint(a, "https://copilot-api.{s}", .{host}) else a.dupe(u8, "https://api.individual.githubcopilot.com") else a.dupe(u8, "https://api.individual.githubcopilot.com");
}
fn rateLimitDelay(a: std.mem.Allocator, io: std.Io, headers: t.ProviderHeaders, retry: u32) !?f64 {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    const record = try @import("../../utils/headers.zig").headersToRecord(scratch, headers);
    const retry_after = j.field(record, "retry-after");
    var delay = 500 * std.math.pow(f64, 2, @floatFromInt(retry));
    if (retry_after) |text| if (text.len > 0) {
        const seconds = try @import("../../utils/numbers.zig").parseFloat(scratch, io, text);
        delay = if (seconds) |value| value * 1000 else (try @import("../../utils/datetime.zig").parseJavaScriptDate(scratch, io, text)) - @as(f64, @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds()));
        if (!std.math.isFinite(delay)) return null;
    };
    return @max(0, delay);
}
fn request(a: std.mem.Allocator, io: std.Io, network: transport.Transport, method: std.http.Method, url: []const u8, body: ?[]const u8, headers: t.ProviderHeaders, max_retries: u32, read_body: enum { always, rate_limit }) !http.Response {
    const started = std.Io.Clock.awake.now(io).toMilliseconds();
    var retry: u32 = 0;
    const retry_deadline = if (max_retries > 0) std.Io.Clock.real.now(io).toMilliseconds() + 5000 else null;
    while (true) {
        const attempt_started = std.Io.Clock.awake.now(io).toMilliseconds();
        const remaining_budget = if (max_retries > 0) 5000 - (attempt_started - started) else 5000;
        if (remaining_budget <= 0) return error.RequestTimeout;
        const timeout: u64 = @intCast(@min(5000, remaining_budget));
        const h = @import("../../utils/headers.zig");
        const request_headers = if (h.get(headers, "accept") == null) try h.merge(a, &.{ &.{.{ .name = "accept", .value = "*/*" }}, headers }) else headers;
        var response = try network.send(a, io, .{ .method = method, .uri = try uri.parseHttpUrl(a, url), .body = body, .headers = request_headers, .timeoutMs = @floatFromInt(timeout) });
        var released = false;
        defer if (!released) response.deinit();
        if (response.status == 429 and retry < max_retries) {
            if (try rateLimitDelay(a, io, response.headers, retry)) |delay| {
                const remaining = @as(f64, @floatFromInt(retry_deadline.? - std.Io.Clock.real.now(io).toMilliseconds()));
                if (delay < remaining) {
                    response.deinit();
                    released = true;
                    retry += 1;
                    // Date.now controls the retry decision, while the live
                    // timeout signal uses a relative timer across the sleep.
                    const active_remaining = 5000 - (std.Io.Clock.awake.now(io).toMilliseconds() - started);
                    if (active_remaining <= 0) return error.RequestTimeout;
                    const timer = @import("../../utils/provider_retry.zig");
                    if (timer.timerMilliseconds(delay) >= active_remaining) {
                        try io.sleep(.fromMilliseconds(@intCast(active_remaining)), .awake);
                        return error.RequestTimeout;
                    }
                    try timer.sleep(io, delay);
                    continue;
                }
            }
        }
        if (read_body == .rate_limit and response.status != 429) return .{ .status = response.status, .statusText = try a.dupe(u8, response.statusText), .raw = "", .body = j.map(a) };
        const remaining = @as(i128, timeout) - (std.Io.Clock.awake.now(io).toMilliseconds() - attempt_started);
        if (remaining <= 0) {
            if (read_body == .rate_limit and response.status == 429) return .{ .status = 429, .statusText = try a.dupe(u8, response.statusText), .raw = "", .body = j.map(a) };
            return error.RequestTimeout;
        }
        const wire = response.readAllWithTimeout(a, io, .unlimited, @floatFromInt(remaining)) catch |err| {
            if (err == error.Canceled or err == error.OutOfMemory) return err;
            // A terminal policy 429 stops the outer enable-model loop even
            // when response.text() fails. Fetch/send failures stay retryable
            // by that loop's source catch/continue behavior.
            if (read_body == .rate_limit and response.status == 429) return .{ .status = 429, .statusText = try a.dupe(u8, response.statusText), .raw = "", .body = j.map(a) };
            return err;
        };
        defer a.free(wire);
        const raw = try http.responseText(a, wire, .fetch);
        errdefer a.free(raw);
        const status_text = try a.dupe(u8, response.statusText);
        errdefer a.free(status_text);
        if (read_body == .rate_limit or response.status < 200 or response.status >= 300) return .{ .status = response.status, .statusText = status_text, .raw = raw, .body = j.map(a) };
        const parsed = @import("../../json/value.zig").parseFromSliceLeaky(std.json.Value, a, raw, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |err| {
            if (err == error.OutOfMemory) return err;
            return .{ .status = response.status, .statusText = status_text, .raw = raw, .body = j.map(a), .jsonError = err };
        };
        return .{ .status = response.status, .statusText = status_text, .raw = raw, .body = parsed };
    }
}
fn bearerHeaders(a: std.mem.Allocator, token: []const u8, extra: t.ProviderHeaders) !t.ProviderHeaders {
    return @import("../../utils/headers.zig").merge(a, &.{ copilotHeaders, &.{.{ .name = "Authorization", .value = try std.fmt.allocPrint(a, "Bearer {s}", .{token}) }}, extra });
}
pub const ModelCatalog = struct { availableModelIds: []const []const u8, policyModelIds: []const []const u8 };
pub fn parseGitHubCopilotModelCatalog(a: std.mem.Allocator, raw: std.json.Value, allow_fallback: bool) !ModelCatalog {
    const data = j.get(raw, "data") orelse return error.InvalidCopilotModelsResponse;
    if (data != .array) return error.InvalidCopilotModelsResponse;
    var picker: std.ArrayList([]const u8) = .empty;
    var enabled: std.ArrayList([]const u8) = .empty;
    var policy: std.ArrayList([]const u8) = .empty;
    for (data.array.items) |item| {
        const id = j.field(item, "id") orelse continue;
        const supports = j.get(j.get(item, "capabilities") orelse .null, "supports") orelse .null;
        if (j.get(supports, "tool_calls")) |value| if (value == .bool and !value.bool) continue;
        const picked = if (j.get(item, "model_picker_enabled")) |value| value == .bool and value.bool else false;
        const state = j.field(j.get(item, "policy") orelse .null, "state") orelse "";
        if (picked and !std.mem.eql(u8, state, "disabled")) try picker.append(a, id);
        if (std.mem.eql(u8, state, "enabled")) try enabled.append(a, id);
    }
    const fallback = allow_fallback and picker.items.len == 0;
    var known = try @import("../../model_catalog.zig").Catalog.decode(a, @embedFile("../../../data/github-copilot.json"));
    defer known.deinit();
    for (data.array.items) |item| {
        const id = j.field(item, "id") orelse continue;
        if (known.get(.chat, id) == null) continue;
        const supports = j.get(j.get(item, "capabilities") orelse .null, "supports") orelse .null;
        if (j.get(supports, "tool_calls")) |value| if (value == .bool and !value.bool) continue;
        const picked = if (j.get(item, "model_picker_enabled")) |value| value == .bool and value.bool else false;
        const state = j.field(j.get(item, "policy") orelse .null, "state") orelse "";
        if (std.mem.eql(u8, state, "unconfigured") and (picked or fallback)) try policy.append(a, id);
    }
    return .{ .availableModelIds = if (fallback) try enabled.toOwnedSlice(a) else try picker.toOwnedSlice(a), .policyModelIds = try policy.toOwnedSlice(a) };
}
fn fetchModels(a: std.mem.Allocator, io: std.Io, network: transport.Transport, token: []const u8, host: ?[]const u8, retries: u32, error_message: ?*?[]const u8) !ModelCatalog {
    const base = try getGitHubCopilotBaseUrl(a, token, host);
    const response = try request(a, io, network, .GET, try std.fmt.allocPrint(a, "{s}/models", .{base}), null, try bearerHeaders(a, token, &.{ .{ .name = "accept", .value = "application/json" }, .{ .name = "X-GitHub-Api-Version", .value = "2026-06-01" } }), retries, .always);
    if (!response.ok()) {
        http.remember(error_message, try std.fmt.allocPrint(a, "{d} {s}: {s}", .{ response.status, response.statusText, response.raw }));
        return error.CopilotModelsRequestFailed;
    }
    if (response.jsonError) |err| return err;
    return parseGitHubCopilotModelCatalog(a, response.body, std.mem.eql(u8, base, "https://api.individual.githubcopilot.com")) catch |err| {
        if (err != error.OutOfMemory) http.remember(error_message, "Invalid Copilot models response");
        return err;
    };
}
fn refreshAccess(a: std.mem.Allocator, io: std.Io, network: transport.Transport, token: []const u8, host: ?[]const u8, error_message: ?*?[]const u8) !auth.OAuthCredential {
    const api_host = try std.fmt.allocPrint(a, "api.{s}", .{host orelse "github.com"});
    const response = try http.request(a, io, network, .GET, try endpoint(a, api_host, "/copilot_internal/v2/token"), null, null, try bearerHeaders(a, token, &.{}));
    if (!response.ok()) {
        http.remember(error_message, try std.fmt.allocPrint(a, "{d} {s}: {s}", .{ response.status, response.statusText, response.raw }));
        return error.CopilotTokenRequestFailed;
    }
    if (response.jsonError) |err| return err;
    if (response.body != .object and response.body != .array) {
        http.remember(error_message, "Invalid Copilot token response");
        return error.InvalidOAuthResponse;
    }
    const access = j.field(response.body, "token");
    const expiry = if (j.get(response.body, "expires_at")) |value| j.number(value) else null;
    if (access == null or expiry == null) {
        http.remember(error_message, "Invalid Copilot token response fields");
        return error.InvalidOAuthResponse;
    }
    var metadata = j.map(a);
    if (host) |value| try j.put(a, &metadata, "enterpriseUrl", j.str(value));
    return .{ .access = access.?, .refresh = token, .expires = expiry.? * 1000 - 300_000, .metadata = metadata };
}
const Poll = struct {
    a: std.mem.Allocator,
    network: transport.Transport,
    host: []const u8,
    device: []const u8,
    fn call(ctx: ?*anyopaque, io: std.Io) !polling.PollResult([]const u8) {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        const response = try http.request(self.a, io, self.network, .POST, try endpoint(self.a, self.host, "/login/oauth/access_token"), try uri.encodeForm(self.a, &.{ .{ .name = "client_id", .value = client_id }, .{ .name = "device_code", .value = self.device }, .{ .name = "grant_type", .value = "urn:ietf:params:oauth:grant-type:device_code" } }), "application/x-www-form-urlencoded", &.{.{ .name = "User-Agent", .value = "GitHubCopilotChat/0.35.0" }});
        if (!response.ok()) return .{ .failed = try std.fmt.allocPrint(self.a, "{d} {s}: {s}", .{ response.status, response.statusText, response.raw }) };
        if (response.jsonError) |err| return err;
        if (j.field(response.body, "access_token")) |token| return .{ .complete = token };
        const err = j.field(response.body, "error") orelse return .{ .failed = "Invalid device token response" };
        if (std.mem.eql(u8, err, "authorization_pending")) return .pending;
        if (std.mem.eql(u8, err, "slow_down")) return .{ .slow_down = if (j.get(response.body, "interval")) |value| j.number(value) else null };
        const description = j.get(response.body, "error_description") orelse .null;
        return .{ .failed = try std.fmt.allocPrint(self.a, "Device flow failed: {s}{s}", .{ err, if (j.truthy(description)) try std.fmt.allocPrint(self.a, ": {s}", .{try @import("../../utils/diagnostics.zig").formatThrownValue(self.a, .{ .value = description })}) else "" }) };
    }
};
fn login(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, interaction: auth.Interaction, _: auth.LoginOptions) !auth.OAuthCredential {
    const input = try interaction.prompt(interaction.context, a, io, try j.from(a, .{ .type = "text", .message = "GitHub Enterprise URL/domain (blank for github.com)", .placeholder = "company.ghe.com" }));
    io.checkCancel() catch |err| {
        http.remember(interaction.errorMessage, "Login cancelled");
        return err;
    };
    const host = domain(a, input) catch |err| {
        if (err != error.OutOfMemory) http.remember(interaction.errorMessage, "Invalid GitHub Enterprise URL/domain");
        return err;
    };
    const response = try http.request(a, io, network, .POST, try endpoint(a, host orelse "github.com", "/login/device/code"), try uri.encodeForm(a, &.{ .{ .name = "client_id", .value = client_id }, .{ .name = "scope", .value = "read:user" } }), "application/x-www-form-urlencoded", &.{.{ .name = "User-Agent", .value = "GitHubCopilotChat/0.35.0" }});
    if (!response.ok()) {
        http.remember(interaction.errorMessage, try std.fmt.allocPrint(a, "{d} {s}: {s}", .{ response.status, response.statusText, response.raw }));
        return error.GitHubDeviceAuthorizationFailed;
    }
    if (response.jsonError) |err| return err;
    if (response.body != .object and response.body != .array) {
        http.remember(interaction.errorMessage, "Invalid device code response");
        return error.InvalidOAuthResponse;
    }
    const device = j.field(response.body, "device_code");
    const user = j.field(response.body, "user_code");
    const verification = j.field(response.body, "verification_uri");
    const raw_interval = j.get(response.body, "interval");
    const interval = if (raw_interval) |value| j.number(value) else null;
    const lifetime = if (j.get(response.body, "expires_in")) |value| j.number(value) else null;
    if (device == null or user == null or verification == null or (raw_interval != null and interval == null) or lifetime == null) {
        http.remember(interaction.errorMessage, "Invalid device code response fields");
        return error.InvalidOAuthResponse;
    }
    if (try @import("../../schema/formats.zig").canParseAbsoluteUrl(a, verification.?) == false) {
        http.remember(interaction.errorMessage, "Untrusted verification_uri in device code response");
        return error.UntrustedVerificationUri;
    }
    const parsed = uri.parseHttpUrl(a, verification.?) catch |err| {
        if (err == error.OutOfMemory) return err;
        http.remember(interaction.errorMessage, "Untrusted verification_uri in device code response");
        return error.UntrustedVerificationUri;
    };
    if (!std.ascii.eqlIgnoreCase(parsed.scheme, "https") and !std.ascii.eqlIgnoreCase(parsed.scheme, "http")) {
        http.remember(interaction.errorMessage, "Untrusted verification_uri in device code response");
        return error.UntrustedVerificationUri;
    }
    var notification = try j.from(a, .{ .type = "device_code", .userCode = user.?, .verificationUri = try uri.render(a, parsed), .expiresInSeconds = lifetime.? });
    if (raw_interval) |value| try j.put(a, &notification, "intervalSeconds", value);
    try interaction.notify(interaction.context, io, notification);
    var poll = Poll{ .a = a, .network = network, .host = host orelse "github.com", .device = device.? };
    const token = try polling.pollOAuthDeviceCodeFlow([]const u8, io, .{ .context = &poll, .intervalSeconds = interval, .expiresInSeconds = lifetime.?, .waitBeforeFirstPoll = true, .poll = Poll.call, .errorMessage = interaction.errorMessage });
    var result = try refreshAccess(a, io, network, token, host, interaction.errorMessage);
    const models = try fetchModels(a, io, network, result.access, host, 2, interaction.errorMessage);
    var available: std.StringArrayHashMapUnmanaged(void) = .{};
    for (models.availableModelIds) |id| try available.put(a, id, {});
    if (models.policyModelIds.len > 0) try interaction.notify(interaction.context, io, try j.from(a, .{ .type = "progress", .message = "Enabling models..." }));
    const base = try getGitHubCopilotBaseUrl(a, result.access, host);
    for (models.policyModelIds) |id| {
        const url = try std.fmt.allocPrint(a, "{s}/models/{s}/policy", .{ base, id });
        const policy = request(a, io, network, .POST, url, try @import("../../json/value.zig").stringify(a, .{ .state = "enabled" }, .{}), try bearerHeaders(a, result.access, &.{ .{ .name = "Content-Type", .value = "application/json" }, .{ .name = "openai-intent", .value = "chat-policy" }, .{ .name = "x-interaction-type", .value = "chat-policy" } }), 2, .rate_limit) catch |err| {
            if (err == error.Canceled or err == error.OutOfMemory) return err;
            continue;
        };
        if (policy.status == 429) break;
        if (policy.ok()) try available.put(a, id, {});
    }
    var metadata = result.metadata orelse j.map(a);
    try j.put(a, &metadata, "availableModelIds", try j.from(a, available.keys()));
    result.metadata = metadata;
    return result;
}
fn refresh(context: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, previous: auth.OAuthCredential) !auth.OAuthCredential {
    return refreshWithError(context, a, io, network, previous, null);
}
fn refreshWithError(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: transport.Transport, previous: auth.OAuthCredential, error_message: ?*?[]const u8) !auth.OAuthCredential {
    const host = try enterprise(a, previous);
    var result = try refreshAccess(a, io, network, previous.refresh, host, error_message);
    const models = try fetchModels(a, io, network, result.access, host, 0, error_message);
    var metadata = result.metadata orelse j.map(a);
    try j.put(a, &metadata, "availableModelIds", try j.from(a, models.availableModelIds));
    result.metadata = metadata;
    return result;
}
fn toAuth(_: ?*anyopaque, a: std.mem.Allocator, _: std.Io, previous: auth.OAuthCredential) !auth.ModelAuth {
    return .{ .apiKey = previous.access, .baseUrl = try getGitHubCopilotBaseUrl(a, previous.access, try enterprise(a, previous)) };
}
pub const githubCopilotOAuth = auth.OAuthAuth{ .name = "GitHub Copilot", .isSubscription = true, .login = login, .refresh = refresh, .refreshWithError = refreshWithError, .toAuth = toAuth };
