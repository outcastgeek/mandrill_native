//! Provider request lifecycle. JSON is serialized once after callbacks, and the
//! exact resulting body is passed to signing and transport unchanged.
const std = @import("std");
const t = @import("../types.zig");
const m = @import("../models.zig");
const j = @import("../json/value.zig");
const c = @import("common.zig");
const http = @import("../transport/http.zig");
const uri = @import("../transport/uri.zig");
const headers = @import("../utils/headers.zig");
const events = @import("../utils/event_stream.zig");
const Machine = @import("reducer.zig").Machine;
const Value = std.json.Value;
fn observesAllResponses(api_id: []const u8) bool {
    return std.mem.eql(u8, api_id, "pi-messages") or std.mem.eql(u8, api_id, "mistral-conversations") or
        std.mem.eql(u8, api_id, "openai-codex-responses") or std.mem.eql(u8, api_id, "bedrock-converse-stream");
}
pub fn implementation(comptime api: []const u8) m.ApiImplementation {
    return .{ .api = api, .stream = stream, .streamSimple = streamSimple, .complete = directComplete, .completeSimple = directCompleteSimple };
}
pub fn streamSimple(ctx: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !*events.AssistantMessageEventStream {
    var arena = std.heap.ArenaAllocator.init(operation.allocator);
    defer arena.deinit();
    return stream(ctx, operation, model, context, try prepareSimpleOptions(arena.allocator(), operation, model, context, options));
}
pub fn directCompleteSimple(_: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !t.Owned(t.AssistantMessage) {
    var arena = std.heap.ArenaAllocator.init(operation.allocator);
    defer arena.deinit();
    return complete(operation, model, context, try prepareSimpleOptions(arena.allocator(), operation, model, context, options));
}
fn prepareSimpleOptions(a: std.mem.Allocator, operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !t.StreamOptions {
    var detail: ?[]const u8 = null;
    var input = options;
    input.errorMessage = &detail;
    if (std.mem.eql(u8, model.api, "anthropic-messages")) {
        var auth_operation = operation;
        auth_operation.allocator = a;
        _ = @import("../auth/anthropic_federation.zig").prepare(auth_operation, model, input, &detail) catch |err| {
            if (operation.errorMessage orelse options.errorMessage) |slot| {
                slot.* = if (detail != null and operation.errorStorage != null) try operation.errorStorage.?.copy(detail.?) else @errorName(err);
            }
            return err;
        };
    }
    const google_key = std.mem.eql(u8, model.api, "google-generative-ai");
    const openai_key = std.mem.eql(u8, model.api, "openai-completions") or std.mem.eql(u8, model.api, "openai-responses");
    if ((google_key or (openai_key and !hasHeaderAuthentication(options.headers))) and (options.apiKey == null or options.apiKey.?.len == 0)) {
        if (operation.errorMessage orelse options.errorMessage) |slot| {
            slot.* = if (operation.errorStorage) |storage| try storage.copy(try std.fmt.allocPrint(a, "No API key for provider: {s}", .{model.provider})) else @errorName(error.MissingAuthentication);
        }
        return error.MissingAuthentication;
    }
    var result = @import("simple_options.zig").forApi(a, model, context, input) catch |err| {
        if (operation.errorMessage orelse options.errorMessage) |slot| {
            // A raw slot transfers no allocation ownership. Retain a dynamic
            // detail only through an explicit owner before `a` is destroyed.
            slot.* = if (detail != null and operation.errorStorage != null) try operation.errorStorage.?.copy(detail.?) else @errorName(err);
        }
        return err;
    };
    result.errorMessage = options.errorMessage;
    return result;
}
fn directComplete(_: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !t.Owned(t.AssistantMessage) {
    return complete(operation, model, context, options);
}
const RequestState = struct {
    operation: m.OperationContext,
    model: t.Model,
    context: t.TranscriptContext,
    options: t.StreamOptions,
    handle: *events.AssistantMessageEventStream,
    disablePreferredStrictTools: bool = false,
    anthropicPrepared: ?struct { config: ?@import("../auth/anthropic_federation.zig").Config, headers: t.ProviderHeaders } = null,
};
pub fn cloneOptions(a: std.mem.Allocator, original: t.StreamOptions) !t.StreamOptions {
    var options = original;
    if (original.apiKey) |value| options.apiKey = try a.dupe(u8, value);
    if (original.sessionId) |value| options.sessionId = try a.dupe(u8, value);
    const copied_headers = try a.alloc(t.ProviderHeader, original.headers.len);
    for (copied_headers, original.headers) |*out, in| out.* = .{ .name = try a.dupe(u8, in.name), .value = if (in.value) |v| try a.dupe(u8, v) else null };
    options.headers = copied_headers;
    options.env = .{};
    for (original.env.keys(), original.env.values()) |key, value| try options.env.put(a, try a.dupe(u8, key), try a.dupe(u8, value));
    if (original.samplingParams) |v| options.samplingParams = try j.clone(a, v);
    if (original.extra) |v| options.extra = try j.clone(a, v);
    if (original.metadata) |v| options.metadata = try j.clone(a, v);
    if (original.deferred) |v| options.deferred = try j.clone(a, v);
    options.jsonFields = try j.clone(a, original.jsonFields);
    if (original.thinkingBudgets) |value| options.thinkingBudgets = try t.cloneInto(t.ThinkingBudgets, a, value);
    return options;
}
pub fn stream(_: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !*events.AssistantMessageEventStream {
    const handle = try events.AssistantMessageEventStream.create(operation.allocator, operation.io);
    errdefer handle.destroy();
    try handle.prepareModel(model);
    const a = handle.arena.allocator();
    const request = try a.create(RequestState);
    request.* = .{ .operation = operation, .model = (try t.Owned(t.Model).clone(a, model)).value(), .context = (try t.Owned(t.TranscriptContext).clone(a, context)).value(), .options = try cloneOptions(a, options), .handle = handle };
    handle.producer = try operation.io.concurrent(produce, .{request});
    return handle;
}
/// Direct completion drives the same reducer without a producer task. A chosen
/// transport or timeout may independently require backend concurrency.
pub fn complete(operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !t.Owned(t.AssistantMessage) {
    const handle = try events.AssistantMessageEventStream.create(operation.allocator, operation.io);
    defer handle.destroy();
    try handle.prepareModel(model);
    var request = RequestState{ .operation = operation, .model = model, .context = context, .options = options, .handle = handle };
    produce(&request);
    return t.Owned(t.AssistantMessage).clone(operation.allocator, try handle.result());
}
fn produce(request: *RequestState) void {
    var machine = Machine.init(request.handle.arena.allocator(), request.operation.io, request.model, request.handle);
    machine.googleToolCallIds = request.operation.googleToolCallIds;
    execute(request, &machine) catch |err| {
        machine.prepareFailure();
        var canceled = err == error.Canceled;
        const mistral = std.mem.eql(u8, request.model.api, "mistral-conversations");
        if (std.mem.eql(u8, request.model.api, "pi-messages") or mistral) request.operation.io.checkCancel() catch {
            canceled = true;
        };
        if (mistral) @import("mistral_conversations.zig").prepareFailure(&machine, err, canceled);
        const protection = request.operation.io.swapCancelProtection(.blocked);
        defer _ = request.operation.io.swapCancelProtection(protection);
        machine.piAborted = canceled;
        machine.fail(err);
        if (canceled) request.operation.io.recancel();
    };
}
pub fn buildPayload(a: std.mem.Allocator, io: std.Io, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !Value {
    if (std.mem.eql(u8, model.api, "openai-completions")) {
        return @import("openai_completions.zig").buildPayload(a, io, model, context, options);
    }
    if (std.mem.eql(u8, model.api, "openai-responses")) return @import("openai_responses.zig").buildPayload(a, io, model, context, options);
    if (std.mem.eql(u8, model.api, "azure-openai-responses")) return @import("azure_openai_responses.zig").buildPayload(a, io, model, context, options);
    if (std.mem.eql(u8, model.api, "openai-codex-responses")) return @import("openai_codex_responses.zig").buildPayload(a, io, model, context, options);
    if (std.mem.eql(u8, model.api, "anthropic-messages")) return @import("anthropic_messages.zig").buildPayload(a, io, model, context, options);
    if (std.mem.eql(u8, model.api, "google-generative-ai") or std.mem.eql(u8, model.api, "google-vertex")) return @import("google_shared.zig").buildSdkPayload(a, io, model, context, options);
    if (std.mem.eql(u8, model.api, "mistral-conversations")) return @import("mistral_conversations.zig").buildPayload(a, io, model, context, options);
    if (std.mem.eql(u8, model.api, "bedrock-converse-stream")) return @import("bedrock_converse_stream.zig").buildPayload(a, io, model, context, options);
    if (std.mem.eql(u8, model.api, "pi-messages")) return @import("pi_messages.zig").buildPayload(a, io, model, context, options);
    return error.UnsupportedApi;
}
fn execute(request: *RequestState, machine: *Machine) !void {
    const a = machine.allocator;
    const io = request.operation.io;
    const model = request.model;
    var options = request.options;
    if (std.mem.eql(u8, model.api, "openai-responses")) if (@import("openai_responses.zig").getConfigurationUpdateEffortValue(model, options)) |effort| {
        if (j.text(effort)) |value| machine.message.providerThinkingLevel = value else try j.retainField(a, &machine.message, "providerThinkingLevel", effort);
    };
    const google_sdk = std.mem.eql(u8, model.api, "google-generative-ai") or std.mem.eql(u8, model.api, "google-vertex");
    if (google_sdk and options.httpTransport != null) {
        machine.message.errorMessage = if (std.mem.eql(u8, model.api, "google-vertex")) "Custom fetch is not supported by the Google Vertex adapter" else "Custom fetch is not supported by the Google Generative AI adapter";
        return error.GoogleCustomFetchUnsupported;
    }
    const cache_retention: []const u8 = options.env.get("PI_CACHE_RETENTION") orelse "";
    if (options.cacheRetention == null and cache_retention.len == 0) if (request.operation.authContext) |auth_context| {
        if (try auth_context.env(a, io, "PI_CACHE_RETENTION")) |value| {
            if (std.mem.eql(u8, value, "long")) options.cacheRetention = .long else if (!std.mem.eql(u8, model.api, "pi-messages")) options.cacheRetention = .short;
        }
    };
    var operation = request.operation;
    operation.allocator = a;
    operation.transport = options.httpTransport orelse operation.transport;
    operation.errorMessage = &machine.message.errorMessage;
    options.errorMessage = operation.errorMessage;
    options.env = try hydrateEndpointEnv(a, io, operation.authContext, options.env, model);
    if (std.mem.eql(u8, model.api, "google-vertex")) {
        options.apiKey = try @import("google_vertex.zig").resolveApiKey(a, options);
        try @import("google_vertex.zig").validateClient(a, options);
    }
    if (std.mem.eql(u8, model.api, "google-generative-ai") and (options.apiKey == null or options.apiKey.?.len == 0)) return error.MissingAuthentication;
    if ((std.mem.eql(u8, model.api, "mistral-conversations") or std.mem.eql(u8, model.api, "azure-openai-responses") or std.mem.eql(u8, model.api, "pi-messages")) and (options.apiKey == null or options.apiKey.?.len == 0)) return error.MissingAuthentication;
    if ((std.mem.eql(u8, model.api, "openai-completions") or std.mem.eql(u8, model.api, "openai-responses")) and (options.apiKey == null or options.apiKey.?.len == 0) and !hasHeaderAuthentication(options.headers)) {
        machine.message.errorMessage = try std.fmt.allocPrint(a, "No API key for provider: {s}", .{model.provider});
        return error.MissingAuthentication;
    }
    const prepared_endpoint: ?std.Uri = if (std.mem.eql(u8, model.api, "azure-openai-responses")) try @import("azure_openai_responses.zig").endpoint(a, model, options) else if (std.mem.eql(u8, model.api, "pi-messages")) try @import("pi_messages.zig").endpoint(a, model.baseUrl, j.truthy(c.extra(options, "debug") orelse .null)) else null;
    const anthropic_config = if (request.anthropicPrepared) |prepared| prepared.config else if (std.mem.eql(u8, model.api, "anthropic-messages")) try @import("../auth/anthropic_federation.zig").prepare(operation, model, options, &machine.message.errorMessage) else null;
    if (std.mem.eql(u8, model.api, "anthropic-messages")) {
        machine.anthropicOAuth = !options.authFromClient and !c.is(model, "github-copilot") and (if (options.apiKey) |key| std.mem.find(u8, key, "sk-ant-oat") != null else false);
        machine.tools = try @import("../utils/transcript.zig").getCurrentTools(a, request.context.messages);
    }
    if (std.mem.eql(u8, model.api, "openai-completions") or std.mem.eql(u8, model.api, "openai-responses") or std.mem.eql(u8, model.api, "azure-openai-responses") or std.mem.eql(u8, model.api, "openai-codex-responses")) {
        const grammar_supported = if (std.mem.eql(u8, model.api, "openai-completions")) j.truthy(j.getNonNull(model.compat orelse .null, "supportsOpenAIGrammarTools") orelse .{ .bool = false }) else if (std.mem.eql(u8, model.api, "openai-responses")) @import("openai_responses.zig").compatFlag(model, "supportsOpenAIGrammarTools", false) else c.compatBool(model, "supportsOpenAIGrammarTools", false);
        const grammar_context = if (std.mem.eql(u8, model.api, "openai-completions") or std.mem.eql(u8, model.api, "openai-responses")) try @import("../utils/transcript.zig").resolveTranscript(a, request.context, j.truthy(j.getNonNull(model.compat orelse .null, "supportsMidConvoSystemMessages") orelse .{ .bool = false })) else request.context;
        machine.grammarToolInputProperties = try @import("../schema/constrained.zig").createGrammarToolInputPropertiesWithError(a, try @import("../utils/transcript.zig").getDeclaredTools(a, grammar_context.messages), grammar_supported, operation.errorMessage);
    }
    const openai_sdk = std.mem.eql(u8, model.api, "openai-completions") or std.mem.eql(u8, model.api, "openai-responses") or std.mem.eql(u8, model.api, "azure-openai-responses");
    const client_headers = openai_sdk or (std.mem.eql(u8, model.api, "anthropic-messages") and !options.authFromClient);
    const prepared_headers: ?t.ProviderHeaders = if (request.anthropicPrepared) |prepared| prepared.headers else if (google_sdk) try @import("google_shared.zig").clientHeaders(a, model, options) else if (client_headers) try requestHeadersWithEnvironment(a, io, model, request.context, options, operation.authContext) else null;
    if (std.mem.eql(u8, model.api, "anthropic-messages") and !options.authFromClient and request.anthropicPrepared == null) request.anthropicPrepared = .{ .config = anthropic_config, .headers = prepared_headers.? };
    try io.checkCancel();
    const anthropic_payload: ?@import("anthropic_messages.zig").BuiltPayload = if (std.mem.eql(u8, model.api, "anthropic-messages")) try @import("anthropic_messages.zig").buildPayloadDetails(a, io, model, request.context, options, request.disablePreferredStrictTools) else null;
    var payload = if (anthropic_payload) |built| built.payload else try buildPayload(a, io, model, request.context, options);
    if (anthropic_payload) |built| if (built.providerThinkingLevel) |effort| if (j.truthy(effort)) {
        if (j.text(effort)) |value| machine.message.providerThinkingLevel = value else try j.retainField(a, &machine.message, "providerThinkingLevel", effort);
    };
    const has_preferred_strict = if (anthropic_payload) |built| built.hasPreferredStrictTools else false;
    if (options.callbacks.onPayload) |callback| if (try callback(options.callbacks.context, a, &payload, model)) |replacement| {
        if (std.mem.eql(u8, model.api, "anthropic-messages")) {
            payload = try j.objectSpread(a, replacement);
            try j.put(a, &payload, "stream", .{ .bool = true });
        } else payload = try j.clone(a, replacement);
    };
    if (payload == .null and (std.mem.eql(u8, model.api, "openai-completions") or std.mem.eql(u8, model.api, "openai-responses") or std.mem.eql(u8, model.api, "azure-openai-responses"))) {
        machine.message.errorMessage = "Cannot read properties of null (reading 'stream')";
        return error.InvalidOpenAIRequestPayload;
    }
    if (std.mem.eql(u8, model.api, "openai-codex-responses")) if (options.transport != .sse) {
        try @import("openai_codex_responses.zig").normalizeTimeouts(a, &options, &machine.message.errorMessage);
        if (try @import("openai_codex_responses.zig").tryWebSocket(operation, machine, model, payload, options)) return;
    };
    if (std.mem.eql(u8, model.api, "openai-codex-responses") and options.transport == .sse) try @import("openai_codex_responses.zig").normalizeTimeouts(a, &options, &machine.message.errorMessage);
    const google_wire_payload: ?Value = if (google_sdk) try @import("google_shared.zig").toWirePayloadWithError(a, std.mem.eql(u8, model.api, "google-vertex"), payload, if (std.mem.eql(u8, model.api, "google-vertex")) try @import("google_vertex.zig").resources(a, options) else .{}, operation.errorMessage) else null;
    const bedrock_endpoint: ?@import("../auth/aws/credentials.zig").ResolvedEndpoint = if (std.mem.eql(u8, model.api, "bedrock-converse-stream")) try @import("../auth/aws/credentials.zig").resolveRequestEndpoint(operation, model, options, payload) else null;
    const endpoint = prepared_endpoint orelse if (bedrock_endpoint) |resolved| resolved.uri else try endpointFor(a, model, options, payload);
    var request_headers = prepared_headers orelse try requestHeadersWithEnvironment(a, io, model, request.context, options, operation.authContext);
    var omit_federation_authorization = false;
    if (anthropic_config != null) for (try headers.spread(a, &.{ try headers.fromValue(a, model.headers), options.headers })) |header| {
        if (std.ascii.eqlIgnoreCase(header.name, "authorization")) omit_federation_authorization = header.value == null;
    };
    if (google_sdk) request_headers = try @import("google_shared.zig").requestHeadersWithError(a, payload, request_headers, options.apiKey, options.errorMessage);
    if (std.mem.eql(u8, model.api, "mistral-conversations")) try @import("mistral_conversations.zig").validateHeaderTimeout(a, options, &machine.message.errorMessage);
    var body_payload = google_wire_payload orelse payload;
    if (std.mem.eql(u8, model.api, "anthropic-messages")) {
        body_payload = try j.clone(a, payload);
        if (body_payload.object.fetchSwapRemove("betas")) |entry| {
            if (entry.value != .null) {
                const value = try @import("../utils/diagnostics.zig").formatThrownValue(a, .{ .value = entry.value });
                request_headers = try headers.merge(a, &.{ request_headers, &.{.{ .name = "anthropic-beta", .value = value }} });
            }
        }
        inline for (.{ .{ "user_profile_id", "anthropic-user-profile-id" }, .{ "workspace_id", "anthropic-workspace-id" } }) |field| {
            if (body_payload.object.fetchSwapRemove(field[0])) |entry| if (entry.value != .null) {
                const value = try @import("../utils/diagnostics.zig").formatThrownValue(a, .{ .value = entry.value });
                request_headers = try headers.merge(a, &.{ request_headers, &.{.{ .name = field[1], .value = value }} });
            };
        }
    }
    if (std.mem.eql(u8, model.api, "bedrock-converse-stream")) body_payload = try @import("bedrock_converse_stream.zig").toWirePayload(a, payload);
    if (std.mem.eql(u8, model.api, "mistral-conversations")) body_payload = try @import("mistral_conversations.zig").toMistralWirePayloadWithError(a, payload, &machine.message.errorMessage);
    const body = try @import("../json/value.zig").stringify(a, body_payload, .{ .emit_null_optional_fields = false });
    if (openai_sdk or (std.mem.eql(u8, model.api, "anthropic-messages") and !options.authFromClient)) try @import("sdk_request_headers.zig").validateTimeout(options, &machine.message.errorMessage);
    if (std.mem.eql(u8, model.api, "google-vertex")) request_headers = try @import("../auth/google/credentials.zig").requestHeaders(operation, model, options, request_headers);
    if (std.mem.eql(u8, model.api, "bedrock-converse-stream")) operation.awsRegion = try @import("../auth/aws/credentials.zig").region(operation, model, options);
    var bedrock_retry: ?@import("../auth/aws/retry.zig").Policy = if (std.mem.eql(u8, model.api, "bedrock-converse-stream")) try @import("../auth/aws/retry.zig").Policy.init(operation, options) else null;
    var bedrock_credentials: ?@import("../auth/aws/credentials.zig").Credentials = null;
    var bedrock_refresh = @import("../auth/aws/credentials.zig").PassiveRefresh.init(request.operation.allocator, io);
    defer bedrock_refresh.deinit();
    var attempt: f64 = 0;
    const max_retries = if (std.mem.eql(u8, model.api, "mistral-conversations") or std.mem.eql(u8, model.api, "pi-messages")) @as(f64, 0) else options.maxRetries orelse @as(f64, 0);
    const codex_api = std.mem.eql(u8, model.api, "openai-codex-responses");
    const google_api = std.mem.eql(u8, model.api, "google-generative-ai") or std.mem.eql(u8, model.api, "google-vertex");
    const google_timeout = if (google_api) try @import("google_shared.zig").httpTimeoutMs(a, payload) else null;
    var google_retry = if (google_api) try @import("google_http.zig").Policy.init(a, io, payload) else null;
    while (true) {
        var attempt_endpoint = endpoint;
        var attempt_headers = request_headers;
        if (std.mem.eql(u8, model.api, "anthropic-messages")) attempt_headers = try @import("../auth/anthropic_federation.zig").authenticatePrepared(operation, anthropic_config, attempt_headers, omit_federation_authorization, &machine.message.errorMessage);
        if (std.mem.eql(u8, model.api, "bedrock-converse-stream")) {
            machine.bedrockRequestId = null;
            machine.bedrockErrorCode = null;
            machine.bedrockFailureStatus = null;
            machine.bedrockServerTime = null;
            attempt_headers = try bedrock_retry.?.requestHeaders(a, attempt_headers);
            const prepared = try @import("../auth/aws/credentials.zig").prepareRequestWithRegionAndRefresh(operation, model, options, endpoint, attempt_headers, body, if (bedrock_retry.?.disableClockCorrection) 0 else bedrock_retry.?.clockOffsetMs, bedrock_credentials, bedrock_endpoint.?.signingRegion, &bedrock_refresh);
            bedrock_credentials = prepared.credentials;
            if (prepared.requestSentAt) |sent_at| bedrock_retry.?.requestSentAt = sent_at;
            attempt_endpoint = prepared.uri;
            attempt_headers = prepared.headers;
        }
        const proxy = try @import("../transport/proxy.zig").resolveHttpProxyUrlForTargetWithError(a, io, operation.authContext, options.env, attempt_endpoint, &machine.message.errorMessage);
        const timeout: ?f64 = if (google_api) google_timeout else requestTimeoutMs(model, options);
        const sdk_transport = std.mem.eql(u8, model.api, "openai-completions") or std.mem.eql(u8, model.api, "openai-responses") or std.mem.eql(u8, model.api, "azure-openai-responses") or (std.mem.eql(u8, model.api, "anthropic-messages") and !options.authFromClient);
        const deadline: ?i128 = if (google_timeout) |milliseconds| @as(i128, std.Io.Clock.awake.now(io).toMilliseconds()) + @import("../utils/provider_retry.zig").timerMilliseconds(milliseconds) else null;
        var response = operation.transport.send(a, io, .{ .method = .POST, .uri = attempt_endpoint, .headers = attempt_headers, .body = body, .timeoutMs = timeout, .proxy = proxy, .proxyResolved = true, .followRedirects = bedrock_retry == null, .decompressResponse = bedrock_retry == null }) catch |err| {
            if (err == error.Canceled or err == error.OutOfMemory) {
                if (err == error.Canceled and sdk_transport) machine.message.errorMessage = "Request was aborted.";
                return err;
            }
            if (bedrock_retry) |*policy| {
                if (try policy.waitForRetry(@import("../auth/aws/retry.zig").transportError(err), &.{})) {
                    attempt += 1;
                    continue;
                }
                return err;
            }
            if (codex_api) {
                if (attempt < max_retries) {
                    try @import("../utils/provider_retry.zig").sleep(io, @import("openai_codex_responses.zig").retryBackoff(attempt));
                    attempt += 1;
                    continue;
                }
                if (err == error.RequestTimeout) machine.message.errorMessage = try std.fmt.allocPrint(a, "Codex SSE response headers timed out after {d}ms", .{timeout orelse 0});
                return err;
            }
            if (google_api) {
                const mapped = if (err == error.RequestTimeout) error.GoogleRequestTimeout else err;
                try retryGoogleFailure(a, io, payload, options, machine, &attempt, &google_retry, .{ .err = mapped, .message = if (mapped == error.GoogleRequestTimeout) "This operation was aborted" else @errorName(mapped) }, true);
                continue;
            }
            if (attempt >= max_retries) {
                if (sdk_transport) machine.message.errorMessage = if (err == error.RequestTimeout) "Request timed out." else "Connection error.";
                if (std.mem.eql(u8, model.api, "mistral-conversations") and err == error.RequestTimeout) machine.message.errorMessage = try std.fmt.allocPrint(a, "Mistral response headers timed out after {s}ms", .{try @import("../utils/numbers.zig").toString(a, timeout orelse 60_000)});
                return err;
            }
            try @import("../utils/provider_retry.zig").sleep(io, try @import("../utils/provider_retry.zig").delay(a, io, &.{}, attempt, options.maxRetryDelayMs));
            attempt += 1;
            continue;
        };
        defer response.deinit();
        // Bedrock's raw deserialize observer also sees non-success responses,
        // before error decoding/retry decisions. Callback errors leave here.
        const expose_error_response = observesAllResponses(model.api);
        if (expose_error_response) if (options.callbacks.onResponse) |callback| try callback(options.callbacks.context, .{ .status = response.status, .headers = try headers.fromValue(a, try headers.headersToRecord(a, response.headers)) }, model);
        if (response.status < 200 or response.status >= 300) {
            if (response.status == 401 and std.mem.eql(u8, model.api, "anthropic-messages")) try @import("../auth/anthropic_federation.zig").invalidatePrepared(operation, anthropic_config);
            const bytes = beforeDeadline([]u8, io, deadline, http.Response.readAll, .{ response, a, std.Io.Limit.unlimited }) catch |err| {
                if (google_api and err != error.Canceled and err != error.OutOfMemory) {
                    try retryGoogleFailure(a, io, payload, options, machine, &attempt, &google_retry, .{ .err = err, .message = if (err == error.GoogleRequestTimeout) "This operation was aborted" else @errorName(err) }, if (google_retry) |policy| policy.retryable(response.status) else false);
                    continue;
                }
                if (codex_api and err != error.Canceled and err != error.OutOfMemory and attempt < max_retries) {
                    try @import("../utils/provider_retry.zig").sleep(io, @import("openai_codex_responses.zig").retryBackoff(attempt));
                    attempt += 1;
                    continue;
                }
                return err;
            };
            if (google_api) {
                const eligible = if (google_retry) |policy| policy.retryable(response.status) else false;
                const message = @import("sdk_http_errors.zig").google(a, response.status, response.statusText, response.headers, bytes) catch |err| {
                    if (err == error.Canceled or err == error.OutOfMemory) return err;
                    try retryGoogleFailure(a, io, payload, options, machine, &attempt, &google_retry, .{ .err = err, .message = @errorName(err) }, eligible);
                    continue;
                };
                try retryGoogleFailure(a, io, payload, options, machine, &attempt, &google_retry, .{ .err = error.ProviderHttpError, .status = if (response.status >= 400 and response.status < 600) response.status else null, .message = message }, eligible);
                continue;
            }
            const error_message = if (std.mem.eql(u8, model.api, "mistral-conversations")) try @import("mistral_conversations.zig").httpFailure(a, response.status, response.statusText, bytes) else if (std.mem.eql(u8, model.api, "bedrock-converse-stream")) try @import("bedrock_converse_stream.zig").httpFailure(machine, response.status, response.headers, bytes) else if (std.mem.eql(u8, model.api, "openai-completions") or std.mem.eql(u8, model.api, "openai-responses") or std.mem.eql(u8, model.api, "azure-openai-responses")) try @import("sdk_http_errors.zig").openAI(a, model, response.status, bytes) else if (std.mem.eql(u8, model.api, "anthropic-messages")) try @import("sdk_http_errors.zig").anthropic(a, response.status, bytes) else if (std.mem.eql(u8, model.api, "google-generative-ai") or std.mem.eql(u8, model.api, "google-vertex")) try @import("sdk_http_errors.zig").google(a, response.status, response.statusText, response.headers, bytes) else if (std.mem.eql(u8, model.api, "openai-codex-responses")) try @import("openai_codex_responses.zig").httpFailure(a, io, response.status, response.statusText, bytes) else try @import("../utils/error_body.zig").format(a, response.status, bytes);
            if (has_preferred_strict and response.status == 400 and isStrictToolLimitError(a, bytes)) {
                request.disablePreferredStrictTools = true;
                try @import("../utils/diagnostics.zig").appendAssistantMessageDiagnostic(a, &machine.message, .{
                    .type = "anthropic_strict_tools_fallback",
                    .timestamp = @as(f64, @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds())),
                    .@"error" = .{ .message = error_message },
                    .details = try j.from(a, .{ .reason = "strict_tool_limits_exceeded" }),
                });
                return execute(request, machine);
            }
            if (bedrock_retry) |*policy| {
                const skew_corrected = try policy.responseClock(response.headers, machine.bedrockServerTime);
                const kind = @import("../auth/aws/retry.zig").classify(response.status, machine.bedrockErrorCode, error_message);
                if (try policy.waitForRetry(if (skew_corrected and kind == .other) .transient else kind, response.headers)) {
                    attempt += 1;
                    continue;
                }
            } else if (codex_api) {
                if (attempt < max_retries) if (try @import("openai_codex_responses.zig").httpRetryDelay(a, io, response.status, response.headers, bytes, error_message, attempt, options, &machine.message.errorMessage)) |delay| {
                    try @import("../utils/provider_retry.zig").sleep(io, delay);
                    attempt += 1;
                    continue;
                };
            } else if (attempt < max_retries and @import("../utils/provider_retry.zig").retryable(response.status, if (google_api) &.{} else response.headers)) {
                const retry_message = if (std.mem.eql(u8, model.api, "openai-completions") or std.mem.eql(u8, model.api, "openai-responses") or std.mem.eql(u8, model.api, "azure-openai-responses"))
                    try @import("sdk_http_errors.zig").openAIRequestMessage(a, response.status, bytes)
                else
                    error_message;
                const delay = try @import("../utils/provider_retry.zig").delayWithFailure(a, io, if (google_api) &.{} else response.headers, attempt, options.maxRetryDelayMs, retry_message, &machine.message.errorMessage);
                try @import("../utils/provider_retry.zig").sleep(io, delay);
                attempt += 1;
                continue;
            }
            if (std.mem.eql(u8, model.api, "pi-messages")) {
                const failure = try @import("pi_messages.zig").createPiMessagesResponseError(a, io, model, attempt_endpoint, response.status, response.statusText, bytes);
                machine.message.errorMessage = failure.message;
                try @import("../utils/diagnostics.zig").appendAssistantMessageDiagnostic(a, &machine.message, .{ .type = "pi_messages_response_failure", .timestamp = @as(f64, @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds())), .@"error" = .{ .name = "PiMessagesResponseError", .message = failure.message, .code = if (failure.code) |code| j.str(code) else null }, .details = failure.diagnosticDetails });
            } else machine.message.errorMessage = error_message;
            return error.ProviderHttpError;
        }
        if (bedrock_retry) |*policy| _ = try policy.responseClock(response.headers, null);
        try beforeDeadline(void, io, deadline, consumeResponse, .{ machine, model, options, response });
        return;
    }
}
fn retryGoogleFailure(a: std.mem.Allocator, io: std.Io, payload: Value, options: t.StreamOptions, machine: *Machine, attempt: *f64, policy: *?@import("google_http.zig").Policy, failure: @import("google_http.zig").Failure, allow_sdk: bool) !void {
    var selected = failure;
    if (allow_sdk) if (policy.*) |*sdk| {
        if (try sdk.wait(io, failure)) return;
        selected = sdk.main.?;
    };
    if (selected.status != null and attempt.* < (options.maxRetries orelse 0) and @import("../utils/provider_retry.zig").retryable(selected.status, &.{})) {
        const delay = try @import("../utils/provider_retry.zig").delayWithFailure(a, io, &.{}, attempt.*, options.maxRetryDelayMs, selected.message, &machine.message.errorMessage);
        try @import("../utils/provider_retry.zig").sleep(io, delay);
        attempt.* += 1;
        policy.* = try @import("google_http.zig").Policy.init(a, io, payload);
        return;
    }
    machine.message.errorMessage = selected.message;
    return selected.err;
}
fn beforeDeadline(comptime Result: type, io: std.Io, deadline: ?i128, comptime call: anytype, args: anytype) !Result {
    const at = deadline orelse return @call(.auto, call, args);
    const remaining = at - std.Io.Clock.awake.now(io).toMilliseconds();
    if (remaining <= 0) return error.GoogleRequestTimeout;
    const Outcome = union(enum) { result: anyerror!Result, timeout: std.Io.Cancelable!void };
    var buffer: [2]Outcome = undefined;
    var select = std.Io.Select(Outcome).init(io, &buffer);
    defer select.cancelDiscard();
    try select.concurrent(.result, call, args);
    try select.concurrent(.timeout, struct {
        fn sleep(clock_io: std.Io, milliseconds: i64) std.Io.Cancelable!void {
            try clock_io.sleep(.fromMilliseconds(milliseconds), .awake);
        }
    }.sleep, .{ io, @as(i64, @intCast(@min(remaining, std.math.maxInt(i64)))) });
    return switch (try select.await()) {
        .result => |result| result,
        .timeout => |result| {
            try result;
            return error.GoogleRequestTimeout;
        },
    };
}
fn consumeResponse(machine: *Machine, model: t.Model, options: t.StreamOptions, response: http.Response) !void {
    const a = machine.allocator;
    const google = std.mem.eql(u8, model.api, "google-generative-ai") or std.mem.eql(u8, model.api, "google-vertex");
    const expose_error_response = observesAllResponses(model.api);
    if (!expose_error_response and !google) if (options.callbacks.onResponse) |callback| try callback(options.callbacks.context, .{ .status = response.status, .headers = try headers.fromValue(a, try headers.headersToRecord(a, response.headers)) }, model);
    const pi = std.mem.eql(u8, model.api, "pi-messages");
    const bedrock = std.mem.eql(u8, model.api, "bedrock-converse-stream");
    if (pi and !response.bodyPresent) {
        machine.message.errorMessage = try std.fmt.allocPrint(a, "{s} response has no body", .{model.provider});
        return error.MissingPiMessagesResponseBody;
    }
    if (std.mem.eql(u8, model.api, "mistral-conversations") and !response.bodyPresent) {
        machine.message.errorMessage = "Mistral response has no body";
        return error.MissingMistralResponseBody;
    }
    if (!pi and !bedrock) try machine.start();
    if (bedrock) {
        try @import("bedrock_converse_stream.zig").responseMetadata(machine, response.status, response.headers, false);
        var decoder = @import("../transport/aws_event_stream.zig").Decoder.init(a, response.body);
        defer decoder.deinit();
        while (decoder.next() catch |err| return response.readError(err)) |frame| {
            const event = (try @import("bedrock_converse_stream.zig").frameEvent(machine, frame)) orelse continue;
            if (options.callbacks.onProviderStreamEvent) |callback| try callback(options.callbacks.context, event, model);
            try machine.consume(event);
        }
    } else {
        const codex = std.mem.eql(u8, model.api, "openai-codex-responses");
        const anthropic = std.mem.eql(u8, model.api, "anthropic-messages");
        if (anthropic and !response.bodyPresent) {
            machine.message.errorMessage = "Attempted to iterate over an Anthropic response with no body";
            return error.MissingAnthropicResponseBody;
        }
        if (google and !response.bodyPresent) {
            machine.message.errorMessage = "Response body is empty";
            return error.MissingGoogleResponseBody;
        }
        var anthropic_started = false;
        var anthropic_ended = false;
        const completions = std.mem.eql(u8, model.api, "openai-completions");
        const sdk_stream = completions or std.mem.eql(u8, model.api, "openai-responses") or std.mem.eql(u8, model.api, "azure-openai-responses");
        const mistral = std.mem.eql(u8, model.api, "mistral-conversations");
        const residual = anthropic or mistral or sdk_stream or codex or pi;
        var decoder = @import("../transport/sse.zig").Decoder.init(a, response.body, residual);
        defer decoder.deinit();
        var google_body = @import("google_http.zig").StreamingBody{ .allocator = a, .errorMessage = &machine.message.errorMessage };
        if (anthropic or google) decoder.bodyChunks = response;
        if (google) decoder.bodyChunkObserver = .{ .context = &google_body, .call = @import("google_http.zig").StreamingBody.observe };
        decoder.framing = if (codex) .codex else if (pi) .pi else if (google) .google else .lines;
        decoder.requireDataColon = mistral;
        decoder.trimDataLineStart = mistral;
        decoder.dispatchEventWithoutData = sdk_stream or anthropic;
        decoder.utf8PerLine = sdk_stream or anthropic or mistral;
        decoder.stripBomPerLine = sdk_stream;
        decoder.collectRawLines = anthropic;
        decoder.requireCompleteEvent = google;
        while (decoder.next() catch |err| return response.readError(err)) |frame| {
            if (anthropic) {
                if (std.mem.eql(u8, frame.event, "error")) {
                    machine.message.errorMessage = try a.dupe(u8, frame.data);
                    return error.ProviderError;
                }
                const MessageEvent = enum { message_start, message_delta, message_stop, content_block_start, content_block_delta, content_block_stop };
                if (std.meta.stringToEnum(MessageEvent, frame.event) == null) continue;
            }
            const trim_data = mistral or pi or codex;
            const data = if (trim_data) try @import("../utils/unicode.zig").trimWhitespace(a, frame.data) else frame.data;
            defer if (trim_data) a.free(data);
            if (data.len == 0 and !sdk_stream and !anthropic and !google) continue;
            const done = std.mem.eql(u8, data, "[DONE]");
            if (!google and !anthropic and done) {
                if (pi or codex) continue else break;
            }
            // The pinned Anthropic adapter consumes asResponse() through
            // its own repairing SSE iterator, not the SDK stream decoder.
            const parsed = (if (anthropic) @import("../json/partial.zig").parseJsonWithRepair(a, data) else @import("../json/value.zig").parseFromSliceLeaky(Value, a, data, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always })) catch |err| {
                if (err == error.OutOfMemory) return err;
                if (codex) {
                    machine.message.errorMessage = try std.fmt.allocPrint(a, "Invalid Codex SSE JSON: {s}", .{@errorName(err)});
                    return error.CodexProtocolError;
                }
                if (anthropic) {
                    machine.message.errorMessage = try std.fmt.allocPrint(a, "Could not parse Anthropic SSE event {s}: {s}; data={s}; raw={s}", .{ frame.event, @errorName(err), data, frame.raw });
                    return err;
                }
                if (sdk_stream) {
                    machine.message.errorMessage = "Error reading response: malformed server-sent event JSON.";
                    return error.InvalidOpenAIStreamingJson;
                }
                return err;
            };
            if (pi and !j.truthy(parsed)) continue;
            if (anthropic) {
                if (parsed == .null) {
                    machine.message.errorMessage = try std.fmt.allocPrint(a, "Could not parse Anthropic SSE event {s}: Cannot read properties of null (reading 'type'); data={s}; raw={s}", .{ frame.event, data, frame.raw });
                    return error.InvalidAnthropicEvent;
                }
                const kind = j.field(parsed, "type") orelse "";
                if (std.mem.eql(u8, kind, "message_start")) anthropic_started = true else if (std.mem.eql(u8, kind, "message_stop")) anthropic_ended = true;
            }
            if (std.mem.eql(u8, model.api, "mistral-conversations") and (parsed != .object or (j.get(parsed, "choices") orelse .null) != .array)) return error.InvalidMistralStreamingEvent;
            const threaded = sdk_stream and std.mem.startsWith(u8, frame.event, "thread.");
            if (sdk_stream and !threaded) {
                const failure = j.get(parsed, "error") orelse .null;
                if (std.mem.eql(u8, frame.event, "error") or j.truthy(failure)) {
                    machine.message.errorMessage = try @import("sdk_http_errors.zig").openAIStreamFailure(a, model, if (failure == .null) parsed else failure);
                    return error.ProviderError;
                }
            }
            const observed = if (google) try @import("google_wire.zig").event(a, std.mem.eql(u8, model.api, "google-vertex"), parsed, response.headers) else if (threaded) try j.from(a, .{ .event = frame.event, .data = parsed }) else parsed;
            if (options.callbacks.onProviderStreamEvent) |callback| try callback(options.callbacks.context, observed, model);
            const event = if (codex) try @import("openai_codex_responses.zig").mapEvent(a, machine, observed) else observed;
            try machine.consume(event);
            if (pi and machine.terminal) break;
            if (codex and machine.terminal) {
                @import("openai_codex_responses.zig").applyServiceTierPricing(machine, event, options);
                break;
            }
            const event_type = j.field(event, "type") orelse "";
            if (std.mem.eql(u8, model.api, "openai-responses") and (std.mem.eql(u8, event_type, "response.completed") or std.mem.eql(u8, event_type, "response.incomplete"))) {
                const tier = j.getNonNull(j.get(event, "response") orelse .null, "service_tier") orelse c.extra(options, "serviceTier") orelse .null;
                @import("openai_responses.zig").applyServiceTierPricing(&machine.message.usage, j.text(tier), model);
            }
        }
        if (anthropic) {
            if (anthropic_started and !anthropic_ended) {
                machine.message.errorMessage = "Anthropic stream ended before message_stop";
                return error.StreamEndedWithoutTerminalEvent;
            }
            machine.terminal = true;
        }
    }
    try machine.finish();
}
fn requestTimeoutMs(model: t.Model, options: t.StreamOptions) ?f64 {
    if (std.mem.eql(u8, model.api, "pi-messages") or std.mem.eql(u8, model.api, "google-generative-ai") or std.mem.eql(u8, model.api, "google-vertex") or std.mem.eql(u8, model.api, "bedrock-converse-stream")) return null;
    if (std.mem.eql(u8, model.api, "openai-codex-responses")) return if (options.timeoutMs == 0) null else options.timeoutMs;
    if (std.mem.eql(u8, model.api, "mistral-conversations")) return options.timeoutMs orelse 60_000;
    if (options.authFromClient and std.mem.eql(u8, model.api, "anthropic-messages") and options.timeoutMs == null) return null;
    return options.timeoutMs orelse 600_000;
}
fn hasHeaderAuthentication(values: t.ProviderHeaders) bool {
    for (values) |header| if (std.ascii.eqlIgnoreCase(header.name, "authorization") or std.ascii.eqlIgnoreCase(header.name, "cf-aig-authorization")) {
        if (header.value) |value| if (std.mem.trim(u8, value, &std.ascii.whitespace).len > 0) return true;
    };
    return false;
}
pub fn hydrateEndpointEnv(a: std.mem.Allocator, io: std.Io, context: ?@import("../auth/types.zig").AuthContext, original: t.ProviderEnv, model: t.Model) !t.ProviderEnv {
    const ctx = context orelse return original;
    const names: []const []const u8 = if (std.mem.eql(u8, model.api, "google-vertex")) &.{ "GOOGLE_CLOUD_PROJECT", "GCLOUD_PROJECT", "GOOGLE_CLOUD_LOCATION" } else if (std.mem.eql(u8, model.api, "azure-openai-responses") or c.is(model, "azure")) &.{ "AZURE_OPENAI_BASE_URL", "AZURE_OPENAI_RESOURCE_NAME", "AZURE_OPENAI_API_VERSION", "AZURE_OPENAI_DEPLOYMENT_NAME_MAP" } else if (std.mem.eql(u8, model.api, "bedrock-converse-stream")) &.{ "AWS_REGION", "AWS_DEFAULT_REGION", "AWS_BEDROCK_FORCE_CACHE" } else return original;
    var env: t.ProviderEnv = .{};
    for (original.keys(), original.values()) |key, value| try env.put(a, key, value);
    for (names) |name| {
        if (env.get(name)) |value| if (value.len > 0) continue;
        if (try ctx.env(a, io, name)) |value| if (value.len > 0) try env.put(a, name, value);
    }
    return env;
}
fn isStrictToolLimitError(a: std.mem.Allocator, bytes: []const u8) bool {
    const parsed = @import("../json/value.zig").parseFromSlice(Value, a, bytes, .{ .duplicate_field_behavior = .use_last }) catch return false;
    defer parsed.deinit();
    const failure = j.get(parsed.value, "error") orelse parsed.value;
    if (!std.mem.eql(u8, j.field(failure, "type") orelse "", "invalid_request_error")) return false;
    const message = j.field(failure, "message") orelse return false;
    for ([_][]const u8{ "The compiled grammar is too large", "Too many strict tools (", "Schemas contains too many parameters with union types (" }) |needle| if (std.mem.find(u8, message, needle) != null) return true;
    return false;
}
pub fn endpointFor(a: std.mem.Allocator, model: t.Model, options: t.StreamOptions, payload: Value) !std.Uri {
    if (std.mem.eql(u8, model.api, "openai-completions")) return @import("openai_completions.zig").endpoint(a, model.baseUrl);
    if (std.mem.eql(u8, model.api, "openai-responses")) return @import("openai_responses.zig").endpoint(a, model.baseUrl);
    if (std.mem.eql(u8, model.api, "azure-openai-responses")) return @import("azure_openai_responses.zig").endpoint(a, model, options);
    if (std.mem.eql(u8, model.api, "openai-codex-responses")) return @import("openai_codex_responses.zig").endpoint(a, model.baseUrl);
    if (std.mem.eql(u8, model.api, "anthropic-messages")) return @import("anthropic_messages.zig").endpoint(a, model.baseUrl);
    if (std.mem.eql(u8, model.api, "mistral-conversations")) return @import("mistral_conversations.zig").endpoint(a, model.baseUrl);
    if (std.mem.eql(u8, model.api, "pi-messages")) return @import("pi_messages.zig").endpoint(a, model.baseUrl, j.truthy(c.extra(options, "debug") orelse .null));
    if (std.mem.eql(u8, model.api, "google-generative-ai")) return @import("google_generative_ai.zig").endpoint(a, model, options, payload);
    if (std.mem.eql(u8, model.api, "google-vertex")) return @import("google_vertex.zig").endpoint(a, model, options, payload);
    if (std.mem.eql(u8, model.api, "bedrock-converse-stream")) return @import("bedrock_converse_stream.zig").endpoint(a, model.baseUrl, payload, options.errorMessage);
    return error.UnsupportedApi;
}
pub fn requestHeaders(a: std.mem.Allocator, io: std.Io, model: t.Model, options: t.StreamOptions) !t.ProviderHeaders {
    return requestHeadersForContext(a, io, model, .{ .messages = &.{} }, options);
}
pub fn requestHeadersForContext(a: std.mem.Allocator, io: std.Io, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !t.ProviderHeaders {
    return requestHeadersWithEnvironment(a, io, model, context, options, null);
}
fn requestHeadersWithEnvironment(a: std.mem.Allocator, io: std.Io, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions, auth_context: ?@import("../auth/types.zig").AuthContext) !t.ProviderHeaders {
    // Codex forces token/account headers after model and caller overrides.
    if (std.mem.eql(u8, model.api, "openai-codex-responses")) return @import("openai_codex_responses.zig").requestHeaders(a, io, model, options, false);
    if (std.mem.eql(u8, model.api, "pi-messages")) return @import("pi_messages.zig").requestHeaders(a, options);
    if (std.mem.eql(u8, model.api, "mistral-conversations")) return @import("mistral_conversations.zig").requestHeaders(a, model, options);
    var defaults: std.ArrayList(t.ProviderHeader) = .empty;
    const pi_agent = try @import("../utils/pi_user_agent.zig").getPiUserAgent(a);
    try defaults.append(a, .{ .name = "user-agent", .value = pi_agent });
    try defaults.appendSlice(a, &.{ .{ .name = "content-type", .value = "application/json" }, .{ .name = "accept", .value = "text/event-stream" } });
    const anthropic = std.mem.eql(u8, model.api, "anthropic-messages");
    const anthropic_oauth = anthropic and !options.authFromClient and !c.is(model, "github-copilot") and (if (options.apiKey) |key| std.mem.find(u8, key, "sk-ant-oat") != null else false);
    const anthropic_affinity = anthropic and !options.authFromClient and !c.is(model, "github-copilot") and !anthropic_oauth;
    const openai = std.mem.eql(u8, model.api, "openai-completions") or std.mem.eql(u8, model.api, "openai-responses");
    const openai_sdk = openai or std.mem.eql(u8, model.api, "azure-openai-responses");
    if (openai_sdk) {
        try defaults.appendSlice(a, try @import("sdk_request_headers.zig").openAI(a, io, auth_context, options.timeoutMs));
        try defaults.append(a, .{ .name = "user-agent", .value = pi_agent });
    }
    if (anthropic) {
        if (!options.authFromClient) try defaults.appendSlice(a, try @import("sdk_request_headers.zig").anthropic(a, options.timeoutMs orelse 600_000));
        try defaults.appendSlice(a, &.{ .{ .name = "anthropic-version", .value = "2023-06-01" }, .{ .name = "anthropic-dangerous-direct-browser-access", .value = "true" }, .{ .name = "accept", .value = "application/json" } });
        if (if (!options.authFromClient) options.apiKey else null) |key| {
            const oauth = anthropic_oauth;
            if (oauth or c.is(model, "github-copilot")) try defaults.append(a, .{ .name = "authorization", .value = try std.fmt.allocPrint(a, "Bearer {s}", .{key}) }) else try defaults.append(a, .{ .name = "x-api-key", .value = key });
            if (oauth) try defaults.appendSlice(a, &.{ .{ .name = "user-agent", .value = "claude-cli/2.1.280" }, .{ .name = "x-app", .value = "cli" } });
        }
    } else if (!std.mem.eql(u8, model.api, "bedrock-converse-stream")) if (if (options.apiKey != null and options.apiKey.?.len > 0) options.apiKey else if (std.mem.eql(u8, model.api, "openai-completions") or std.mem.eql(u8, model.api, "openai-responses")) @as(?[]const u8, "unused") else null) |key| {
        const azure = std.mem.eql(u8, model.api, "azure-openai-responses");
        const google = std.mem.eql(u8, model.api, "google-generative-ai") or std.mem.eql(u8, model.api, "google-vertex");
        try defaults.append(a, .{ .name = if (azure) "api-key" else if (google) "x-goog-api-key" else "authorization", .value = if (azure or google) key else try std.fmt.allocPrint(a, "Bearer {s}", .{key}) });
    };
    var dynamic: t.ProviderHeaders = &.{};
    if (c.is(model, "github-copilot") and (anthropic or openai)) {
        const copilot = @import("github_copilot_headers.zig");
        dynamic = try copilot.buildCopilotDynamicHeaders(a, context.messages, copilot.hasCopilotVisionInput(context.messages));
    }
    var affinity: std.ArrayList(t.ProviderHeader) = .empty;
    if (if (c.cacheRetention(options) != .none) options.sessionId else null) |session| {
        const router = c.is(model, "openrouter") or c.has(model, "openrouter.ai");
        const compat = if (std.mem.eql(u8, model.api, "openai-completions")) try @import("openai_completions.zig").getCompat(a, model) else model.compat orelse .null;
        const send = std.mem.eql(u8, model.api, "openai-responses") or (if (std.mem.eql(u8, model.api, "openai-completions")) j.truthy(j.get(compat, "sendSessionAffinityHeaders") orelse .null) else if (anthropic) @import("anthropic_messages.zig").compatFlag(model, "sendSessionAffinityHeaders", router) else j.boolean(j.get(compat, "sendSessionAffinityHeaders") orelse .null) orelse router);
        if ((anthropic_affinity or openai) and send and (!(std.mem.eql(u8, model.api, "openai-completions") or std.mem.eql(u8, model.api, "openai-responses") or anthropic) or session.len > 0)) {
            const standard_openai = std.mem.eql(u8, model.api, "openai-completions") or std.mem.eql(u8, model.api, "openai-responses");
            const format = if (standard_openai) j.text(j.getNonNull(compat, "sessionAffinityFormat") orelse j.str(if (router) "openrouter" else "openai")) orelse "" else j.field(compat, "sessionAffinityFormat") orelse if (router) "openrouter" else if (anthropic) "anthropic" else "openai";
            if (std.mem.eql(u8, format, "openrouter")) try affinity.append(a, .{ .name = "x-session-id", .value = session }) else if (anthropic) try affinity.append(a, .{ .name = "x-session-affinity", .value = session }) else {
                if (std.mem.eql(u8, format, "openai")) try affinity.append(a, .{ .name = "session_id", .value = session });
                try affinity.append(a, .{ .name = "x-client-request-id", .value = session });
                if (std.mem.eql(u8, model.api, "openai-completions")) try affinity.append(a, .{ .name = "x-session-affinity", .value = session });
            }
        }
    }
    const sdk_custom = if (openai_sdk) try @import("sdk_request_headers.zig").openAICustomHeaders(a, io, auth_context, options.errorMessage) else &.{};
    if (openai_sdk) {
        const provider_defaults = try headers.spread(a, &.{ &.{.{ .name = "User-Agent", .value = pi_agent }}, try headers.fromValue(a, model.headers), dynamic, if (openai) affinity.items else &.{}, options.headers });
        return headers.merge(a, &.{ defaults.items, sdk_custom, provider_defaults, &.{.{ .name = "content-type", .value = "application/json" }} });
    }
    if (anthropic and !options.authFromClient) {
        const custom = try @import("sdk_request_headers.zig").anthropicCustomHeaders(a, io, auth_context);
        const provider_defaults = try headers.spread(a, &.{ custom, &.{ .{ .name = "User-Agent", .value = pi_agent }, .{ .name = "accept", .value = "application/json" }, .{ .name = "anthropic-dangerous-direct-browser-access", .value = "true" } }, if (anthropic_oauth) &.{ .{ .name = "user-agent", .value = "claude-cli/2.1.280" }, .{ .name = "x-app", .value = "cli" } } else &.{}, affinity.items, try headers.fromValue(a, model.headers), dynamic, options.headers });
        return @import("sdk_request_headers.zig").mergeAnthropic(a, &.{ defaults.items, provider_defaults, &.{.{ .name = "content-type", .value = "application/json" }} });
    }
    return headers.merge(a, &.{ defaults.items, if (anthropic) affinity.items else &.{}, try headers.fromValue(a, model.headers), dynamic, if (openai) affinity.items else &.{}, options.headers, if (anthropic and !options.authFromClient) &.{.{ .name = "content-type", .value = "application/json" }} else &.{} });
}
