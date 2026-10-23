const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const c = @import("common.zig");
const uri = @import("../transport/uri.zig");
pub const AzureOpenAIResponsesOptions = @import("options.zig").AzureOpenAIResponsesOptions;
const DeploymentNameRecord = struct { modelId: []const u8, deploymentName: []const u8 };
fn deploymentNameRecord(reader: *std.Io.Reader) !?DeploymentNameRecord {
    const model_id = (try reader.takeDelimiter('=')) orelse return null;
    const deployment = (try reader.takeDelimiter('=')) orelse return null;
    if (model_id.len == 0 or deployment.len == 0) return null;
    return .{ .modelId = model_id, .deploymentName = deployment };
}
pub fn resolveDeploymentName(a: std.mem.Allocator, model: t.Model, options: t.StreamOptions) ![]const u8 {
    if (c.extra(options, "azureDeploymentName")) |v| if (j.text(v)) |name| if (name.len > 0) return name;
    if (options.env.get("AZURE_OPENAI_DEPLOYMENT_NAME_MAP")) |source| {
        var mapped: ?[]const u8 = null;
        errdefer if (mapped) |name| a.free(name);
        var entries = std.Io.Reader.fixed(source);
        while (try entries.takeDelimiter(',')) |entry| {
            const normalized = try @import("../utils/unicode.zig").trimWhitespace(a, entry);
            defer a.free(normalized);
            var reader = std.Io.Reader.fixed(normalized);
            const record = (try deploymentNameRecord(&reader)) orelse continue;
            const id = try @import("../utils/unicode.zig").trimWhitespace(a, record.modelId);
            defer a.free(id);
            if (!std.mem.eql(u8, id, model.id)) continue;
            const deployment = try @import("../utils/unicode.zig").trimWhitespace(a, record.deploymentName);
            if (mapped) |previous| a.free(previous);
            mapped = deployment;
        }
        if (mapped) |name| {
            if (name.len > 0) return name;
            a.free(name);
        }
    }
    return model.id;
}
fn nonempty(value: ?[]const u8) ?[]const u8 {
    const text = value orelse return null;
    return if (text.len > 0) text else null;
}
fn trimmed(a: std.mem.Allocator, value: ?[]const u8) !?[]const u8 {
    const text = try @import("../utils/unicode.zig").trimWhitespace(a, value orelse return null);
    if (text.len == 0) {
        a.free(text);
        return null;
    }
    return text;
}
fn parseBaseUrl(a: std.mem.Allocator, input: []const u8, display: []const u8, error_message: ?*?[]const u8) !std.Uri {
    const text = std.mem.trimEnd(u8, input, "/");
    const invalid = try @import("../schema/formats.zig").canParseAbsoluteUrl(a, text) == false;
    if (!invalid) return uri.parseHttpUrl(a, text) catch |err| {
        if (err == error.OutOfMemory) return err;
        if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Invalid Azure OpenAI base URL: {s}", .{display});
        return error.InvalidAzureBaseUrl;
    };
    if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Invalid Azure OpenAI base URL: {s}", .{display});
    return error.InvalidAzureBaseUrl;
}
pub fn resolveBaseUrl(a: std.mem.Allocator, model: t.Model, options: t.StreamOptions) ![]const u8 {
    const configured = (try trimmed(a, if (c.extra(options, "azureBaseUrl")) |v| j.text(v) else null)) orelse try trimmed(a, options.env.get("AZURE_OPENAI_BASE_URL"));
    defer if (configured) |base| a.free(base);
    const resource = nonempty(if (c.extra(options, "azureResourceName")) |v| j.text(v) else null) orelse nonempty(options.env.get("AZURE_OPENAI_RESOURCE_NAME"));
    var parsed: std.Uri = undefined;
    if (configured) |base| parsed = try parseBaseUrl(a, base, base, options.errorMessage) else if (resource) |name| {
        const base = try std.fmt.allocPrint(a, "https://{s}.openai.azure.com/openai/v1", .{name});
        parsed = try parseBaseUrl(a, base, base, options.errorMessage);
    } else if (model.baseUrl.len > 0) {
        const base = try @import("../utils/unicode.zig").trimWhitespace(a, model.baseUrl);
        defer a.free(base);
        parsed = try parseBaseUrl(a, base, model.baseUrl, options.errorMessage);
        // Keep parsed components alive through the final URI rendering.
        return normalizeBaseUrl(a, parsed);
    } else {
        if (options.errorMessage) |slot| slot.* = "Azure OpenAI base URL is required. Set AZURE_OPENAI_BASE_URL or AZURE_OPENAI_RESOURCE_NAME, or pass azureBaseUrl, azureResourceName, or model.baseUrl.";
        return error.AzureBaseUrlRequired;
    }
    return normalizeBaseUrl(a, parsed);
}
fn normalizeBaseUrl(a: std.mem.Allocator, original: std.Uri) ![]const u8 {
    var parsed = original;
    const host = if (parsed.host) |h| try h.toRawMaybeAlloc(a) else return error.InvalidAzureBaseUrl;
    const azure = std.ascii.endsWithIgnoreCase(host, ".openai.azure.com") or std.ascii.endsWithIgnoreCase(host, ".cognitiveservices.azure.com") or std.ascii.endsWithIgnoreCase(host, ".ai.azure.com");
    const original_path = switch (parsed.path) {
        .raw => |value| value,
        .percent_encoded => |value| value,
    };
    const path = std.mem.trimEnd(u8, original_path, "/");
    if (azure and (path.len == 0 or std.mem.eql(u8, path, "/openai") or std.mem.eql(u8, path, "/openai/v1/responses"))) {
        parsed.path = .{ .raw = "/openai/v1" };
        parsed.query = null;
    }
    return std.mem.trimEnd(u8, try uri.render(a, parsed), "/");
}
pub fn endpoint(a: std.mem.Allocator, model: t.Model, options: t.StreamOptions) !std.Uri {
    const base = try resolveBaseUrl(a, model, options);
    var out = try uri.appendPath(a, base, &.{"responses"});
    const version = nonempty(if (c.extra(options, "azureApiVersion")) |v| j.text(v) else null) orelse nonempty(options.env.get("AZURE_OPENAI_API_VERSION")) orelse "v1";
    out.query = .{ .percent_encoded = try uri.encode(a, &.{.{ .name = "api-version", .value = version }}) };
    return out;
}
pub fn buildPayload(a: std.mem.Allocator, io: std.Io, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !std.json.Value {
    const responses = @import("openai_responses.zig");
    var adapted = model;
    var compat = if (model.compat) |v| try j.clone(a, v) else j.map(a);
    if (j.get(compat, "supportsStrictMode") == null) try j.put(a, &compat, "supportsStrictMode", .{ .bool = true });
    // Azure does not send the OpenAI-specific cache-mode or retention fields.
    try j.put(a, &compat, "supportsExplicitPromptCacheMode", .{ .bool = false });
    try j.put(a, &compat, "supportsLongCacheRetention", .{ .bool = false });
    try j.put(a, &compat, "supportsMidConvoEffort", .{ .bool = false });
    adapted.compat = compat;
    var payload = try j.from(a, .{ .model = try resolveDeploymentName(a, model, options), .stream = true, .store = false });
    try j.put(a, &payload, "input", try responses.convertMessagesWithError(a, io, adapted, context, null, true, options.errorMessage));
    if (try @import("openai_prompt_cache.zig").clampOpenAIPromptCacheKey(options.sessionId)) |key| try j.put(a, &payload, "prompt_cache_key", j.str(key));
    if (options.maxTokens) |limit| if (limit != 0 and !std.math.isNan(limit)) try j.put(a, &payload, "max_output_tokens", j.num(@import("../utils/numbers.zig").maximum(limit, 16)));
    if (options.temperature) |temperature| try j.put(a, &payload, "temperature", j.num(temperature));
    const tools = try @import("../utils/transcript.zig").resolveTranscriptTools(a, context.messages, c.compatBool(model, "supportsAdditionalTools", false) or c.compatBool(model, "supportsToolSearch", false));
    if (tools.requestTools.len > 0) try j.put(a, &payload, "tools", try responses.convertToolsWithError(a, tools.requestTools, adapted, false, options.errorMessage));
    if (options.toolChoice) |choice| try j.put(a, &payload, "tool_choice", j.str(@tagName(choice)));
    if (c.extra(options, "toolChoice")) |choice| try j.put(a, &payload, "tool_choice", choice);
    const resolved = responses.resolveReasoningOptions(model, options);
    if (model.reasoning) {
        if (resolved.requested != null and resolved.requested.?.len > 0) {
            var reasoning = try j.from(a, .{ .effort = resolved.mapped orelse resolved.requested.? });
            try j.put(a, &reasoning, "summary", resolved.summary);
            try j.put(a, &payload, "reasoning", reasoning);
            try j.put(a, &payload, "include", try j.from(a, [_][]const u8{"reasoning.encrypted_content"}));
        } else {
            const disabled = if (model.thinkingLevelMap) |map| j.get(map, "off") else null;
            if (disabled == null or disabled.? != .null) try j.put(a, &payload, "reasoning", try j.from(a, .{ .effort = if (disabled) |value| j.text(value) orelse "none" else "none" }));
        }
    }
    var sampling_options = options;
    sampling_options.reasoning = resolved.level;
    try c.sampling(a, &payload, model, sampling_options);
    return payload;
}

pub fn azureOpenAIResponsesApi() @import("../models.zig").ApiImplementation {
    return @import("driver.zig").implementation("azure-openai-responses");
}
pub fn stream(operation: @import("../models.zig").OperationContext, model: t.Model, context: t.TranscriptContext, options: AzureOpenAIResponsesOptions) !*@import("../utils/event_stream.zig").AssistantMessageEventStream {
    if (!std.mem.eql(u8, model.api, "azure-openai-responses")) return error.MismatchedApi;
    var arena = std.heap.ArenaAllocator.init(operation.allocator);
    defer arena.deinit();
    return @import("driver.zig").stream(null, operation, model, context, try options.streamOptions(arena.allocator()));
}
pub fn streamSimple(operation: @import("../models.zig").OperationContext, model: t.Model, context: t.TranscriptContext, options: t.SimpleStreamOptions) !*@import("../utils/event_stream.zig").AssistantMessageEventStream {
    if (!std.mem.eql(u8, model.api, "azure-openai-responses")) return error.MismatchedApi;
    return @import("driver.zig").streamSimple(null, operation, model, context, options);
}
