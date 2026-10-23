const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const c = @import("common.zig");
const m = @import("../models.zig");
const Machine = @import("reducer.zig").Machine;
const uri = @import("../transport/uri.zig");
const ws = @import("../transport/websocket.zig");
const sessions = @import("../transport/websocket_sessions.zig");
const responses = @import("openai_responses.zig");
pub const OpenAICodexResponsesOptions = @import("options.zig").OpenAICodexResponsesOptions;
pub const OpenAICodexWebSocketDebugStats = sessions.DebugStats;
fn cacheSession(options: t.StreamOptions) ?[]const u8 {
    return if (options.cacheRetention == .none) null else nonempty(options.sessionId);
}
fn codexModel(a: std.mem.Allocator, original: t.Model) !t.Model {
    var model = original;
    var compat = if (model.compat) |value| try j.clone(a, value) else j.map(a);
    if (j.get(compat, "supportsStrictMode") == null) try j.put(a, &compat, "supportsStrictMode", .{ .bool = true });
    model.compat = compat;
    return model;
}
pub fn buildPayload(a: std.mem.Allocator, io: std.Io, original: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !std.json.Value {
    const model = try codexModel(a, original);
    const tr = @import("../utils/transcript.zig");
    const initial = tr.getInitialSystemMessage(context.messages);
    const instructions = if (initial) |value| try @import("../utils/text.zig").getSystemMessageText(a, value) else "";
    const verbosity = c.extra(options, "textVerbosity") orelse .null;
    const tool_choice = if (c.extra(options, "toolChoice")) |choice| (if (choice == .null) j.str("auto") else choice) else j.str(if (options.toolChoice) |choice| @tagName(choice) else "auto");
    var body = try j.from(a, .{ .model = model.id, .store = false, .stream = true, .instructions = if (instructions.len > 0) instructions else "You are a helpful assistant.", .input = try responses.convertMessagesWithError(a, io, model, context, null, false, options.errorMessage), .text = .{ .verbosity = if (j.truthy(verbosity)) verbosity else j.str("low") }, .include = .{"reasoning.encrypted_content"}, .prompt_cache_key = try @import("openai_prompt_cache.zig").clampOpenAIPromptCacheKey(cacheSession(options)), .tool_choice = tool_choice, .parallel_tool_calls = true });
    if (options.temperature) |value| try j.put(a, &body, "temperature", j.num(value));
    if (c.extra(options, "serviceTier")) |value| try j.put(a, &body, "service_tier", value);
    const tools = try tr.resolveTranscriptTools(a, context.messages, c.compatBool(model, "supportsAdditionalTools", false) or c.compatBool(model, "supportsToolSearch", false));
    if (tools.requestTools.len > 0) try j.put(a, &body, "tools", try responses.convertToolsWithError(a, tools.requestTools, model, false, options.errorMessage));
    const summary = c.extra(options, "reasoningSummary") orelse .null;
    const reasoning_summary = if (summary == .null) j.str("auto") else summary;
    if (c.extra(options, "reasoningEffort")) |requested| {
        const name = j.text(requested) orelse return error.InvalidReasoningEffort;
        const mapped = if (model.thinkingLevelMap) |map| j.get(map, if (std.mem.eql(u8, name, "none")) "off" else name) else null;
        // Only explicit "none" lets a null off mapping suppress reasoning.
        // Other direct effort requests fall back to their requested value.
        const effort = if (std.mem.eql(u8, name, "none")) mapped orelse requested else if (mapped) |value| (if (value == .null) requested else value) else requested;
        if (effort != .null) try j.put(a, &body, "reasoning", try j.from(a, .{ .effort = effort, .summary = reasoning_summary }));
    } else if (options.reasoning) |level| {
        const effective = @import("../utils/thinking.zig").clampThinkingLevel(model, level);
        const mapped = if (model.thinkingLevelMap) |map| j.get(map, @tagName(effective)) else null;
        if (mapped == null or mapped.? != .null) try j.put(a, &body, "reasoning", try j.from(a, .{ .effort = mapped orelse j.str(if (effective == .off) "none" else @tagName(effective)), .summary = reasoning_summary }));
    } else if (model.reasoning) {
        const off = if (model.thinkingLevelMap) |map| j.get(map, "off") else null;
        if (off == null or off.? != .null) try j.put(a, &body, "reasoning", try j.from(a, .{ .effort = off orelse j.str("none") }));
    }
    return body;
}
pub fn endpoint(a: std.mem.Allocator, base: []const u8) !std.Uri {
    var result = try std.Uri.parse(if (std.mem.trim(u8, base, " \t\r\n").len > 0) base else "https://chatgpt.com/backend-api");
    const path = std.mem.trimEnd(u8, try result.path.toRawMaybeAlloc(a), "/");
    if (std.mem.endsWith(u8, path, "/codex/responses")) {
        result.path = .{ .raw = path };
        return result;
    }
    result.path = .{ .raw = path };
    return uri.appendPath(a, try uri.render(a, result), if (std.mem.endsWith(u8, path, "/codex")) &.{"responses"} else &.{ "codex", "responses" });
}
/// Extracts a routing claim, without verifying the token or account identity.
/// The returned string is allocator-owned as part of the decoded claim tree.
pub fn accountId(a: std.mem.Allocator, token: []const u8) ![]const u8 {
    const payload = @import("../auth/oauth/jwt.zig").payload(a, token) catch |err| {
        if (err == error.OutOfMemory) return err;
        return error.FailedToExtractAccountId;
    };
    const account = j.field(j.get(payload, "https://api.openai.com/auth") orelse return error.FailedToExtractAccountId, "chatgpt_account_id") orelse return error.FailedToExtractAccountId;
    if (account.len == 0) return error.FailedToExtractAccountId;
    return account;
}
pub fn requestHeaders(a: std.mem.Allocator, io: std.Io, model: t.Model, options: t.StreamOptions, websocket: bool) !t.ProviderHeaders {
    var generator = @import("../utils/uuid.zig").Generator{};
    const session = try @import("openai_prompt_cache.zig").clampOpenAIPromptCacheKey(cacheSession(options));
    const request_id = if (session != null and session.?.len > 0) session.? else if (websocket) try a.dupe(u8, &(try generator.uuidv7(io, null))) else null;
    return requestHeadersWithId(a, model, options, websocket, request_id);
}
fn requestHeadersWithId(a: std.mem.Allocator, model: t.Model, options: t.StreamOptions, websocket: bool, request_id: ?[]const u8) !t.ProviderHeaders {
    const token = options.apiKey orelse return error.MissingAuthentication;
    const account = try accountId(a, token);
    const defaults: t.ProviderHeaders = &.{ .{ .name = "originator", .value = "pi" }, .{ .name = "User-Agent", .value = try @import("../utils/pi_user_agent.zig").getPiUserAgent(a) } };
    const merged = try @import("../utils/headers.zig").overlay(a, &.{ defaults, try @import("../utils/headers.zig").fromValue(a, model.headers), options.headers });
    var forced: std.ArrayList(t.ProviderHeader) = .empty;
    try forced.appendSlice(a, &.{ .{ .name = "Authorization", .value = try std.fmt.allocPrint(a, "Bearer {s}", .{token}) }, .{ .name = "chatgpt-account-id", .value = account }, .{ .name = "OpenAI-Beta", .value = if (websocket) "responses_websockets=2026-02-06" else "responses=experimental" }, .{ .name = "accept", .value = if (websocket) null else "text/event-stream" }, .{ .name = "content-type", .value = if (websocket) null else "application/json" } });
    if (request_id) |id| try forced.appendSlice(a, &.{ .{ .name = "session-id", .value = id }, .{ .name = "x-client-request-id", .value = id } });
    return @import("../utils/headers.zig").merge(a, &.{ merged, forced.items });
}
fn errorCode(event: std.json.Value) ?[]const u8 {
    return j.field(event, "code") orelse j.field(j.get(event, "error") orelse .null, "code");
}
fn nonempty(value: ?[]const u8) ?[]const u8 {
    const text = value orelse return null;
    return if (text.len > 0) text else null;
}
pub fn httpFailure(a: std.mem.Allocator, io: std.Io, status: u16, status_text: []const u8, raw: []const u8) ![]const u8 {
    const fallback = nonempty(raw) orelse nonempty(status_text) orelse "Request failed";
    const parsed = @import("../json/value.zig").parseFromSliceLeaky(std.json.Value, a, raw, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |err| {
        if (err == error.OutOfMemory) return err;
        return fallback;
    };
    const failure = j.get(parsed, "error") orelse return fallback;
    if (!j.truthy(failure)) return fallback;
    const code = nonempty(j.field(failure, "code")) orelse j.field(failure, "type") orelse "";
    var usage_limit = status == 429;
    for ([_][]const u8{ "usage_limit_reached", "usage_not_included", "rate_limit_exceeded" }) |known| if (std.ascii.findIgnoreCase(code, known) != null) {
        usage_limit = true;
    };
    if (usage_limit) {
        const plan = if (nonempty(j.field(failure, "plan_type"))) |name| try std.fmt.allocPrint(a, " ({s} plan)", .{try std.ascii.allocLowerString(a, name)}) else "";
        var when: []const u8 = "";
        if (j.number(j.get(failure, "resets_at") orelse .null)) |reset| if (reset != 0 and std.math.isFinite(reset)) {
            const minutes = @max(0, @floor((reset * 1000 - @as(f64, @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds()))) / 60000 + 0.5));
            when = try std.fmt.allocPrint(a, " Try again in ~{d:.0} min.", .{minutes});
        };
        return std.fmt.allocPrint(a, "You have hit your ChatGPT usage limit{s}.{s}", .{ plan, when });
    }
    return nonempty(j.field(failure, "message")) orelse fallback;
}
pub fn retryBackoff(attempt: f64) f64 {
    return 1000 * std.math.pow(f64, 2, attempt);
}
pub fn normalizeTimeouts(a: std.mem.Allocator, options: *t.StreamOptions, error_message: ?*?[]const u8) !void {
    inline for (.{ "timeoutMs", "websocketConnectTimeoutMs" }) |name| {
        if (try j.fieldValue(a, options.*, name)) |raw| {
            const number = j.number(raw);
            if (number == null or !std.math.isFinite(number.?) or number.? < 0) {
                if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Invalid timeoutMs: {s}", .{try @import("../utils/diagnostics.zig").formatThrownValue(a, .{ .value = raw })});
                return error.InvalidCodexTimeout;
            }
            @field(options, name) = @floor(number.?);
            j.markPresent(options, name);
        }
    }
}
fn retryNumber(a: std.mem.Allocator, io: std.Io, text: []const u8) !?f64 {
    const number = (try @import("../utils/numbers.zig").number(a, io, text)) orelse return null;
    return if (std.math.isFinite(number)) number else null;
}
fn retryAfter(a: std.mem.Allocator, io: std.Io, response_headers: t.ProviderHeaders) !?f64 {
    const h = @import("../utils/headers.zig");
    if (h.get(response_headers, "retry-after-ms")) |value| if (try retryNumber(a, io, value)) |milliseconds| return @max(0, milliseconds);
    if (h.get(response_headers, "retry-after")) |value| {
        if (value.len == 0) return null;
        if (try retryNumber(a, io, value)) |seconds| return @max(0, seconds * 1000);
        const timestamp = @import("../utils/provider_retry.zig").dateMilliseconds(a, io, value) catch |err| {
            if (err == error.OutOfMemory or err == error.Canceled) return err;
            return null;
        };
        return @max(0, @as(f64, @floatFromInt(timestamp)) - @as(f64, @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds())));
    }
    return null;
}
pub fn httpRetryDelay(a: std.mem.Allocator, io: std.Io, status: u16, response_headers: t.ProviderHeaders, raw: []const u8, message: []const u8, attempt: f64, options: t.StreamOptions, failure_message: *?[]const u8) !?f64 {
    if (try @import("../utils/patterns.zig").codexHttpRetryable(a, io, status, raw)) {
        if (try retryAfter(a, io, response_headers)) |delay| {
            const maximum = options.maxRetryDelayMs orelse 60_000;
            if (maximum > 0 and delay > maximum) {
                failure_message.* = try std.fmt.allocPrint(a, "Server requested {d:.0}s retry delay (max: {d:.0}s)", .{ @ceil(delay / 1000), @ceil(maximum / 1000) });
                return error.ServerRetryDelayExceedsCap;
            }
            return delay;
        }
        return retryBackoff(attempt);
    }
    // The pinned fetch loop catches final HTTP errors too. Its usage-limit
    // message is the explicit exception to that network-style retry branch.
    return if (std.mem.find(u8, message, "usage limit") == null) retryBackoff(attempt) else null;
}
pub fn mapEvent(a: std.mem.Allocator, machine: *Machine, event: std.json.Value) !std.json.Value {
    const kind = j.field(event, "type") orelse return event;
    if (std.mem.eql(u8, kind, "error") or std.mem.eql(u8, kind, "response.failed")) {
        const source = if (std.mem.eql(u8, kind, "response.failed")) j.get(event, "response") orelse .null else event;
        const code = if (std.mem.eql(u8, kind, "response.failed")) j.field(j.get(source, "error") orelse .null, "code") else errorCode(source);
        const message = if (std.mem.eql(u8, kind, "response.failed")) j.field(j.get(source, "error") orelse .null, "message") else j.field(source, "message") orelse j.field(j.get(source, "error") orelse .null, "message");
        machine.message.errorMessage = if (std.mem.eql(u8, kind, "error")) try std.fmt.allocPrint(a, "Codex error: {s}", .{nonempty(message) orelse nonempty(code) orelse try @import("../json/value.zig").stringify(a, event, .{})}) else nonempty(message) orelse "Codex response failed";
        if (code) |value| {
            if (std.mem.eql(u8, value, "previous_response_not_found")) return error.PreviousResponseNotFound;
            if (std.mem.eql(u8, value, "websocket_connection_limit_reached")) return error.WebSocketConnectionLimitReached;
        }
        return error.CodexApiError;
    }
    if (std.mem.eql(u8, kind, "response.done") or std.mem.eql(u8, kind, "response.completed") or std.mem.eql(u8, kind, "response.incomplete")) {
        var result = try j.clone(a, event);
        try j.put(a, &result, "type", j.str("response.completed"));
        if (j.get(result, "response")) |original| if (original == .object) {
            var response = original;
            if (j.get(response, "end_turn")) |value| if (j.boolean(value)) |end| {
                machine.message.endTurn = end;
            };
            const status = j.field(response, "status");
            const known = if (status) |name| blk: {
                for ([_][]const u8{ "completed", "incomplete", "failed", "cancelled", "queued", "in_progress" }) |candidate| if (std.mem.eql(u8, name, candidate)) break :blk true;
                break :blk false;
            } else false;
            if (!known) _ = response.object.orderedRemove("status");
            try j.put(a, &result, "response", response);
        };
        return result;
    }
    return event;
}
fn jsonEqual(a: std.mem.Allocator, lhs: std.json.Value, rhs: std.json.Value) !bool {
    const left = try @import("../json/value.zig").stringify(a, lhs, .{});
    const right = try @import("../json/value.zig").stringify(a, rhs, .{});
    return std.mem.eql(u8, left, right);
}
fn withoutInput(a: std.mem.Allocator, body: std.json.Value) !std.json.Value {
    var value = try j.clone(a, body);
    _ = value.object.orderedRemove("input");
    _ = value.object.orderedRemove("previous_response_id");
    return value;
}
pub fn buildCachedWebSocketRequestBody(a: std.mem.Allocator, body: std.json.Value, continuation: ?sessions.Continuation) !std.json.Value {
    const previous = continuation orelse return body;
    if (!try jsonEqual(a, try withoutInput(a, body), try withoutInput(a, previous.lastRequestBody))) return body;
    const current = c.array(body, "input");
    const old = c.array(previous.lastRequestBody, "input");
    const output = if (previous.lastResponseItems == .array) previous.lastResponseItems.array.items else return body;
    if (current.len < old.len + output.len or previous.lastResponseId.len == 0) return body;
    for (old, current[0..old.len]) |lhs, rhs| if (!try jsonEqual(a, lhs, rhs)) return body;
    for (output, current[old.len..][0..output.len]) |lhs, rhs| if (!try jsonEqual(a, lhs, rhs)) return body;
    var result = try j.clone(a, body);
    try j.put(a, &result, "previous_response_id", j.str(previous.lastResponseId));
    try j.put(a, &result, "input", try j.from(a, current[old.len + output.len ..]));
    return result;
}
fn nextWithTimeout(socket: *ws.Session, a: std.mem.Allocator, io: std.Io, timeout: ?f64) !?ws.Frame {
    if (timeout == null or !(timeout.? > 0)) return socket.next(a);
    const Outcome = union(enum) { frame: anyerror!?ws.Frame, timeout: std.Io.Cancelable!void };
    var buffer: [2]Outcome = undefined;
    var select = std.Io.Select(Outcome).init(io, &buffer);
    defer select.cancelDiscard();
    try select.concurrent(.frame, ws.Session.next, .{ socket, a });
    try select.concurrent(.timeout, struct {
        fn wait(clock_io: std.Io, milliseconds: f64) std.Io.Cancelable!void {
            try @import("../utils/provider_retry.zig").sleep(clock_io, milliseconds);
        }
    }.wait, .{ io, timeout.? });
    return switch (try select.await()) {
        .frame => |value| value,
        .timeout => |value| {
            try value;
            return error.WebSocketIdleTimeout;
        },
    };
}
const Failure = struct { name: ?[]const u8 = null, message: ?[]const u8 = null, code: ?u16 = null };
fn process(operation: m.OperationContext, machine: *Machine, model: t.Model, body: std.json.Value, options: t.StreamOptions, request_id: []const u8, failure: *Failure) !void {
    const a = machine.allocator;
    const io = operation.io;
    const session = cacheSession(options);
    const cache = operation.transport.sessions;
    const account = try accountId(a, options.apiKey orelse return error.MissingAuthentication);
    var entry: ?*sessions.Cache.Entry = null;
    if (session != null and cache != null) entry = try cache.?.acquire(session.?, account);
    const reused = entry != null;
    var standalone: ?ws.Connection = null;
    var transient: ?*sessions.Cache.Entry = null;
    if (entry == null) {
        var url = try endpoint(a, model.baseUrl);
        url.scheme = if (std.mem.eql(u8, url.scheme, "https")) "wss" else "ws";
        var proxy_target = url;
        proxy_target.scheme = if (std.mem.eql(u8, url.scheme, "wss")) "https" else "http";
        const proxy = try @import("../transport/proxy.zig").resolveHttpProxyUrlForTargetWithError(a, io, operation.authContext, options.env, proxy_target, &failure.message);
        var connection = operation.transport.openWebSocket(if (cache) |value| value.allocator else a, io, url, try requestHeadersWithId(a, model, options, true, request_id), options.websocketConnectTimeoutMs orelse 15_000, proxy) catch |err| {
            if (err == error.WebSocketConnectTimeout) {
                failure.name = "Error";
                failure.message = try std.fmt.allocPrint(a, "WebSocket connect timeout after {d}ms", .{options.websocketConnectTimeoutMs orelse 15_000});
            }
            return err;
        };
        errdefer connection.deinit();
        if (session != null and cache != null) entry = try cache.?.insert(session.?, account, connection);
        if (entry == null) {
            if (cache) |owner| transient = try owner.trackTransient(connection) else standalone = connection;
        }
    }
    var keep = false;
    defer {
        if (entry) |value| cache.?.release(value, keep) else if (transient) |value| cache.?.release(value, false) else if (standalone) |*connection| connection.deinit();
    }
    const connection = if (entry) |value| &value.connection else if (transient) |value| &value.connection else &standalone.?;
    const socket = if (entry) |value| value.socket else if (transient) |value| value.socket else try ws.Session.open(a, io, connection.*);
    defer if (entry == null and transient == null) {
        socket.close("done");
        socket.deinit();
    };
    const cached = options.transport == .auto or options.transport == .@"websocket-cached";
    const request = if (cached and entry != null) try buildCachedWebSocketRequestBody(a, body, try cache.?.copyContinuation(entry.?, a)) else body;
    if (entry != null and j.get(request, "previous_response_id") == null) try cache.?.clearContinuation(entry.?);
    if (session != null and cache != null) try cache.?.requested(session.?, reused, cached, c.array(request, "input").len, j.field(request, "previous_response_id"));
    var frame = try j.from(a, .{ .type = "response.create" });
    try c.overlay(a, &frame, request);
    socket.send(try @import("../json/value.zig").stringify(a, frame, .{})) catch |err| return connection.ioError(err);
    while (nextWithTimeout(socket, a, io, options.timeoutMs) catch |err| {
        if (err == error.WebSocketIdleTimeout) {
            failure.name = "Error";
            failure.message = try std.fmt.allocPrint(a, "WebSocket idle timeout after {d}ms", .{options.timeoutMs orelse 0});
        }
        return connection.ioError(err);
    }) |message| {
        if (message.payload.len == 0) continue;
        var decoded: ?[]u8 = null;
        defer if (decoded) |bytes| a.free(bytes);
        const text = if (message.opcode == .binary) binary: {
            decoded = try std.fmt.allocPrint(a, "{f}", .{std.unicode.fmtUtf8(message.payload)});
            var reader = std.Io.Reader.fixed(decoded.?);
            if (std.mem.startsWith(u8, decoded.?, &.{ 0xef, 0xbb, 0xbf })) reader.toss(3);
            break :binary reader.buffered();
        } else message.payload;
        if (text.len == 0) continue;
        const event = @import("../json/value.zig").parseFromSliceLeaky(std.json.Value, a, text, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |err| {
            if (err == error.OutOfMemory) return err;
            machine.message.errorMessage = try std.fmt.allocPrint(a, "Invalid Codex WebSocket JSON: {s}", .{@errorName(err)});
            return error.CodexProtocolError;
        };
        if (event == .null) {
            machine.message.errorMessage = "Invalid Codex WebSocket JSON: Cannot read properties of null (reading 'type')";
            return error.CodexProtocolError;
        }
        if (options.callbacks.onProviderStreamEvent) |call| call(options.callbacks.context, event, model) catch |err| {
            machine.message.errorMessage = @errorName(err);
            return error.ProviderStreamEventCallbackError;
        };
        const mapped = try mapEvent(a, machine, event);
        if (j.field(mapped, "type") == null) continue;
        try machine.start();
        try machine.consume(mapped);
        if (machine.terminal) {
            applyServiceTierPricing(machine, mapped, options);
            break;
        }
    }
    if (!machine.terminal) {
        if (try socket.closeInfo(a)) |close| {
            failure.name = "WebSocketCloseError";
            failure.code = close.code;
            const reason = if (close.reason.len > 0) close.reason else if (close.code == 1009) "message too big" else "";
            failure.message = if (reason.len > 0) try std.fmt.allocPrint(a, "WebSocket closed {d} {s}", .{ close.code, reason }) else try std.fmt.allocPrint(a, "WebSocket closed {d}", .{close.code});
        } else {
            failure.name = "Error";
            failure.message = "WebSocket stream closed before response.completed";
        }
        return error.WebSocketClosedBeforeCompletion;
    }
    if (cached and entry != null and machine.message.responseId != null) {
        const output = machine.snapshot();
        const response_items = try responses.convertMessagesWithError(a, io, model, .{ .messages = &.{.{ .assistant = output }} }, null, false, options.errorMessage);
        try cache.?.setContinuation(entry.?, .{ .lastRequestBody = body, .lastResponseId = output.responseId.?, .lastResponseItems = response_items });
    }
    try machine.finish();
    keep = true;
}
pub fn tryWebSocket(operation: m.OperationContext, machine: *Machine, model: t.Model, body: std.json.Value, options: t.StreamOptions) !bool {
    const session = cacheSession(options);
    const cache = operation.transport.sessions;
    if (cache) |value| try value.beginRequest();
    defer if (cache) |value| value.endRequest();
    if (session != null and cache != null and try cache.?.fallbackActive(session.?)) {
        try cache.?.fallback(session.?);
        return false;
    }
    const clamped = try @import("openai_prompt_cache.zig").clampOpenAIPromptCacheKey(session);
    var local_generator = @import("../utils/uuid.zig").Generator{};
    const generator = if (cache) |owner| &owner.uuidGenerator else &local_generator;
    const request_id = if (clamped != null and clamped.?.len > 0) clamped.? else try machine.allocator.dupe(u8, &(try generator.uuidv7(operation.io, null)));
    var retried_previous = false;
    var retried_limit = false;
    while (true) {
        var failure = Failure{};
        process(operation, machine, model, body, options, request_id, &failure) catch |err| {
            if (err == error.Canceled or err == error.OutOfMemory or err == error.SessionCacheClosed) return err;
            if (err == error.PreviousResponseNotFound and !retried_previous) {
                retried_previous = true;
                machine.message.errorMessage = null;
                continue;
            }
            if (err == error.WebSocketConnectionLimitReached and !machine.started and !retried_limit) {
                retried_limit = true;
                machine.message.errorMessage = null;
                continue;
            }
            if (err == error.CodexApiError or err == error.CodexProtocolError or err == error.ProviderStreamEventCallbackError or err == error.PreviousResponseNotFound or (err == error.WebSocketConnectionLimitReached and machine.started)) return err;
            const a = machine.allocator;
            const message = failure.message orelse machine.message.errorMessage orelse @errorName(err);
            try @import("../utils/diagnostics.zig").appendAssistantMessageDiagnostic(a, &machine.message, .{ .type = "provider_transport_failure", .timestamp = @as(f64, @floatFromInt(std.Io.Clock.real.now(operation.io).toMilliseconds())), .@"error" = .{ .name = failure.name orelse @errorName(err), .message = message, .code = if (failure.code) |code| j.int(code) else null }, .details = try j.from(a, .{ .configuredTransport = @tagName(options.transport orelse .auto), .fallbackTransport = if (machine.started) @as(?[]const u8, null) else "sse", .eventsEmitted = machine.started, .phase = if (machine.started) "after_message_stream_start" else "before_message_stream_start", .requestBytes = (try @import("../json/value.zig").stringify(a, body, .{})).len }) });
            if (session != null and cache != null) try cache.?.failure(session.?, message);
            if (machine.started) {
                machine.message.errorMessage = message;
                return err;
            }
            machine.message.errorMessage = null;
            if (session != null and cache != null) try cache.?.fallback(session.?);
            return false;
        };
        return true;
    }
}
pub fn getOpenAICodexWebSocketDebugStats(cache: *sessions.Cache, a: std.mem.Allocator, session: []const u8) !?sessions.DebugStats {
    return cache.stats(a, session);
}
pub fn resetOpenAICodexWebSocketDebugStats(cache: *sessions.Cache, session: ?[]const u8) void {
    cache.resetStats(session);
}
pub fn closeOpenAICodexWebSocketSessions(cache: *sessions.Cache, session: ?[]const u8) void {
    cache.close(session);
}
pub fn applyServiceTierPricing(machine: *Machine, event: std.json.Value, options: t.StreamOptions) void {
    const requested = j.text(c.extra(options, "serviceTier") orelse .null);
    const returned = j.field(j.get(event, "response") orelse .null, "service_tier");
    const tier = if (returned != null and std.mem.eql(u8, returned.?, "default") and requested != null and (std.mem.eql(u8, requested.?, "flex") or std.mem.eql(u8, requested.?, "priority"))) requested else returned orelse requested;
    const name = tier orelse return;
    const multiplier: f64 = if (std.mem.eql(u8, name, "flex")) 0.5 else if (std.mem.eql(u8, name, "priority")) (if (std.mem.eql(u8, machine.model.id, "gpt-5.5")) @as(f64, 2.5) else 2) else 1;
    if (multiplier == 1) return;
    const cost = &machine.message.usage.cost;
    cost.input *= multiplier;
    cost.output *= multiplier;
    cost.cacheRead *= multiplier;
    cost.cacheWrite *= multiplier;
    cost.total = cost.input + cost.output + cost.cacheRead + cost.cacheWrite;
}

pub fn openAICodexResponsesApi() @import("../models.zig").ApiImplementation {
    return @import("driver.zig").implementation("openai-codex-responses");
}
pub fn stream(operation: @import("../models.zig").OperationContext, model: t.Model, context: t.TranscriptContext, options: OpenAICodexResponsesOptions) !*@import("../utils/event_stream.zig").AssistantMessageEventStream {
    if (!std.mem.eql(u8, model.api, "openai-codex-responses")) return error.MismatchedApi;
    var arena = std.heap.ArenaAllocator.init(operation.allocator);
    defer arena.deinit();
    return @import("driver.zig").stream(null, operation, model, context, try options.streamOptions(arena.allocator()));
}
pub fn streamSimple(operation: @import("../models.zig").OperationContext, model: t.Model, context: t.TranscriptContext, options: t.SimpleStreamOptions) !*@import("../utils/event_stream.zig").AssistantMessageEventStream {
    if (!std.mem.eql(u8, model.api, "openai-codex-responses")) return error.MismatchedApi;
    return @import("driver.zig").streamSimple(null, operation, model, context, options);
}
