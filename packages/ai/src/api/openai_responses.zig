const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const c = @import("common.zig");
const tr = @import("../utils/transcript.zig");
const txt = @import("../utils/text.zig");
const hash = @import("../utils/hash.zig").shortHash;
const constrained = @import("../schema/constrained.zig");
const Value = std.json.Value;
const sanitizeText = @import("../utils/unicode.zig").sanitizeSurrogates;
pub const OpenAIResponsesOptions = @import("options.zig").OpenAIResponsesOptions;
pub fn endpoint(a: std.mem.Allocator, base: []const u8) !std.Uri {
    return @import("openai_request_url.zig").endpoint(a, base, "/responses");
}
pub fn applyServiceTierPricing(usage: *t.Usage, service_tier: ?[]const u8, model: t.Model) void {
    const tier = service_tier orelse return;
    const multiplier: f64 = if (std.mem.eql(u8, tier, "flex")) 0.5 else if (std.mem.eql(u8, tier, "priority") or std.mem.eql(u8, tier, "fast")) (if (std.mem.eql(u8, model.id, "gpt-5.5")) @as(f64, 2.5) else 2) else 1;
    if (multiplier == 1) return;
    usage.cost.input *= multiplier;
    usage.cost.output *= multiplier;
    usage.cost.cacheRead *= multiplier;
    usage.cost.cacheWrite *= multiplier;
    usage.cost.total = usage.cost.input + usage.cost.output + usage.cost.cacheRead + usage.cost.cacheWrite;
}
/// Responses resolves nullable compatibility fields before applying source
/// truthiness. Other adapters keep their independently defined policies.
pub fn compatFlag(model: t.Model, key: []const u8, default: bool) bool {
    if (!std.mem.eql(u8, model.api, "openai-responses")) return c.compatBool(model, key, default);
    return j.truthy(j.getNonNull(model.compat orelse .null, key) orelse .{ .bool = default });
}
pub const TextSignature = struct { id: []const u8, phase: ?[]const u8 = null };
pub fn isChatGPTSignIn(model: t.Model, key: ?[]const u8) bool {
    return c.is(model, "openai") and std.mem.eql(u8, model.baseUrl, "https://api.openai.com/v1") and key != null and !std.mem.startsWith(u8, key.?, "sk-");
}
pub const ReasoningOptions = struct { requested: ?[]const u8, mapped: ?[]const u8, summary: Value, level: t.ThinkingLevel, mappedValue: ?Value = null };
pub fn resolveReasoningOptions(model: t.Model, options: t.StreamOptions) ReasoningOptions {
    const explicit = if (c.extra(options, "reasoningEffort")) |value| j.text(value) else if (options.reasoning) |level| (if (level == .off) null else @tagName(level)) else null;
    const summary = c.extra(options, "reasoningSummary") orelse .null;
    const requested = explicit orelse if (j.truthy(summary)) @as(?[]const u8, "medium") else null;
    const mapped = if (explicit) |level| (if (model.thinkingLevelMap) |map| j.field(map, level) orelse level else level) else requested;
    const mapped_value = if (explicit) |level| j.getNonNull(model.thinkingLevelMap orelse .null, level) orelse j.str(level) else if (requested) |level| @as(?Value, j.str(level)) else null;
    return .{ .requested = requested, .mapped = mapped, .mappedValue = mapped_value, .summary = if (j.truthy(summary)) summary else j.str("auto"), .level = if (requested) |level| std.meta.stringToEnum(t.ThinkingLevel, level) orelse .off else .off };
}
pub fn getConfigurationUpdateEffortValue(model: t.Model, options: t.StreamOptions) ?Value {
    const explicit = if (c.extra(options, "reasoningEffort")) |value| j.text(value) else if (options.reasoning) |level| (if (level == .off) null else @tagName(level)) else null;
    if (!compatFlag(model, "supportsMidConvoEffort", false) or explicit == null or explicit.?.len == 0 or isChatGPTSignIn(model, options.apiKey)) return null;
    return j.getNonNull(model.thinkingLevelMap orelse .null, explicit.?) orelse j.str(explicit.?);
}
pub fn getConfigurationUpdateEffort(model: t.Model, options: t.StreamOptions) ?[]const u8 {
    return j.text(getConfigurationUpdateEffortValue(model, options) orelse return null);
}
pub fn parseTextSignature(a: std.mem.Allocator, signature: ?[]const u8) !?TextSignature {
    const source = signature orelse return null;
    if (source.len == 0) return null;
    // The source treats a signature with leading whitespace as a legacy ID.
    // Ask the standard scanner where its first object token ends; do not
    // inspect or split JSON delimiters ourselves.
    var scanner = std.json.Scanner.initCompleteInput(a, source);
    defer scanner.deinit();
    var diagnostics = std.json.Diagnostics{};
    scanner.enableDiagnostics(&diagnostics);
    const first = scanner.next() catch |err| {
        if (err == error.OutOfMemory) return err;
        return .{ .id = source };
    };
    if (first != .object_begin or diagnostics.getByteOffset() != 1) return .{ .id = source };
    const parsed = @import("../json/value.zig").parseFromSlice(Value, a, source, .{ .duplicate_field_behavior = .use_last }) catch |err| {
        if (err == error.OutOfMemory) return err;
        return .{ .id = source };
    };
    defer parsed.deinit();
    if (c.num(parsed.value, "v") == 1) if (j.field(parsed.value, "id")) |id| {
        const phase = j.field(parsed.value, "phase");
        const accepted = if (phase) |v| std.mem.eql(u8, v, "commentary") or std.mem.eql(u8, v, "final_answer") else false;
        return .{ .id = try a.dupe(u8, id), .phase = if (accepted) try a.dupe(u8, phase.?) else null };
    };
    return .{ .id = source };
}
fn normalizePart(a: std.mem.Allocator, id: []const u8) ![]const u8 {
    const out = try @import("transform_messages.zig").normalizeAsciiToolId(a, id, 64);
    defer a.free(out);
    return a.dupe(u8, std.mem.trimEnd(u8, out, "_"));
}
fn normalizeIdAllowed(a: std.mem.Allocator, id: []const u8, model: t.Model, source: t.AssistantMessage, allowed: bool) ![]const u8 {
    const separator = std.mem.findScalar(u8, id, '|') orelse return normalizePart(a, id);
    if (!allowed) return normalizePart(a, id);
    const call = try normalizePart(a, id[0..separator]);
    var items = std.mem.splitScalar(u8, id[separator + 1 ..], '|');
    const item = items.first();
    const foreign = !std.mem.eql(u8, source.provider, model.provider) or !std.mem.eql(u8, source.api, model.api);
    var item_id = if (foreign) try std.fmt.allocPrint(a, "fc_{s}", .{try hash(a, item)}) else try normalizePart(a, item);
    if (!std.mem.startsWith(u8, item_id, "fc_")) item_id = try normalizePart(a, try std.fmt.allocPrint(a, "fc_{s}", .{item_id}));
    return std.fmt.allocPrint(a, "{s}|{s}", .{ call, item_id });
}
pub const ConvertResponsesToolsOptions = struct { strict: ?bool = false, supportsStrictMode: bool = true, supportsOpenAIGrammarTools: bool = false, toolSearchResult: bool = false, errorMessage: ?*?[]const u8 = null };
pub fn convertTools(a: std.mem.Allocator, tools: []const t.Tool, model: t.Model, search: bool) !Value {
    return convertToolsWithError(a, tools, model, search, null);
}
pub fn convertToolsWithError(a: std.mem.Allocator, tools: []const t.Tool, model: t.Model, search: bool, error_message: ?*?[]const u8) !Value {
    return convertResponsesTools(a, tools, .{ .strict = if (std.mem.eql(u8, model.api, "openai-codex-responses")) null else false, .supportsStrictMode = compatFlag(model, "supportsStrictMode", false), .supportsOpenAIGrammarTools = compatFlag(model, "supportsOpenAIGrammarTools", false), .toolSearchResult = search, .errorMessage = error_message });
}
pub fn convertResponsesTools(a: std.mem.Allocator, tools: []const t.Tool, options: ConvertResponsesToolsOptions) !Value {
    var out = j.array(a);
    for (tools) |tool| {
        var entry: Value = undefined;
        if (try constrained.resolveGrammarConstrainedSamplingWithError(a, tool, options.supportsOpenAIGrammarTools, options.errorMessage)) |grammar| {
            entry = try j.from(a, .{ .type = "custom", .name = tool.name, .description = tool.description, .format = .{ .type = "grammar", .syntax = @tagName(grammar.format), .definition = grammar.definition } });
        } else {
            const requested = try constrained.resolveJsonSchemaStrictSamplingWithError(a, tool, options.supportsStrictMode, null, options.errorMessage);
            const strict: ?bool = if (requested) |value| value else options.strict;
            entry = try j.from(a, .{ .type = "function", .name = tool.name, .description = tool.description });
            try j.put(a, &entry, "parameters", try constrained.getJsonSchemaToolParametersWithError(a, tool, strict, options.errorMessage));
            if (options.supportsStrictMode) try j.put(a, &entry, "strict", if (strict) |value| .{ .bool = value } else .null);
        }
        if (options.toolSearchResult) try j.put(a, &entry, "defer_loading", .{ .bool = true });
        try out.array.append(entry);
    }
    return out;
}
fn toolOutput(a: std.mem.Allocator, model: t.Model, content: []const t.Content) !Value {
    const text = try txt.contentText(a, .{ .blocks = content }, "\n");
    var images = j.array(a);
    for (content) |block| if (block == .image) {
        try images.array.append(try j.from(a, .{ .type = "input_image", .detail = "auto", .image_url = try c.imageDataUri(a, block.image) }));
    };
    const vision = for (model.input) |modality| {
        if (modality == .image) break true;
    } else false;
    const has_text = !try @import("../utils/unicode.zig").isBlank(text);
    if (images.array.items.len == 0 or !vision) return j.str(try sanitizeText(a, if (has_text) text else if (images.array.items.len > 0) "(see attached image)" else "(no tool output)"));
    var out = j.array(a);
    if (has_text) try out.array.append(try j.from(a, .{ .type = "input_text", .text = try sanitizeText(a, text) }));
    try out.array.appendSlice(images.array.items);
    return out;
}
fn additions(a: std.mem.Allocator, out: *Value, tools: []const t.Tool, options: ConvertResponsesMessagesOptions, index: usize, error_message: ?*?[]const u8) !void {
    if (tools.len == 0) return;
    var tool_options = options.toolOptions;
    tool_options.errorMessage = error_message;
    if (options.supportsAdditionalTools) {
        var item = try j.from(a, .{ .type = "additional_tools", .role = "developer" });
        try j.put(a, &item, "tools", try convertResponsesTools(a, tools, tool_options));
        try out.array.append(item);
    } else if (options.supportsToolSearch) {
        var names: std.ArrayList([]const u8) = .empty;
        for (tools) |tool| try names.append(a, tool.name);
        const seed = try std.fmt.allocPrint(a, "system:{d}:{s}", .{ index, try std.mem.join(a, ",", names.items) });
        const id = try std.fmt.allocPrint(a, "pi_tool_load_{s}", .{try hash(a, seed)});
        try out.array.append(try j.from(a, .{ .type = "tool_search_call", .call_id = id, .execution = "client", .status = "completed", .arguments = .{ .query = try std.mem.join(a, " ", names.items), .limit = names.items.len } }));
        tool_options.toolSearchResult = true;
        var item = try j.from(a, .{ .type = "tool_search_output", .call_id = id, .execution = "client", .status = "completed" });
        try j.put(a, &item, "tools", try convertResponsesTools(a, tools, tool_options));
        try out.array.append(item);
    }
}
pub fn convertMessages(a: std.mem.Allocator, io: std.Io, model: t.Model, context: t.TranscriptContext, configuration_effort: ?[]const u8, include_system: bool) !Value {
    return convertMessagesWithError(a, io, model, context, configuration_effort, include_system, null);
}
pub const ConvertResponsesMessagesOptions = struct {
    includeSystemPrompt: bool = true,
    grammarToolInputProperties: std.StringHashMapUnmanaged([]const u8) = .{},
    supportsMidConvoSystemMessages: bool = false,
    supportsAdditionalTools: bool = false,
    supportsToolSearch: bool = false,
    toolOptions: ConvertResponsesToolsOptions = .{},
    configurationUpdateEffort: ?[]const u8 = null,
    configurationUpdateEffortValue: ?Value = null,
    errorMessage: ?*?[]const u8 = null,
};
pub fn convertMessagesWithError(a: std.mem.Allocator, io: std.Io, model: t.Model, context: t.TranscriptContext, configuration_effort: ?[]const u8, include_system: bool, error_message: ?*?[]const u8) !Value {
    return convertMessagesWithEffortValue(a, io, model, context, if (configuration_effort) |value| j.str(value) else null, include_system, error_message);
}
fn convertMessagesWithEffortValue(a: std.mem.Allocator, io: std.Io, model: t.Model, context: t.TranscriptContext, configuration_effort: ?Value, include_system: bool, error_message: ?*?[]const u8) !Value {
    const mid = compatFlag(model, "supportsMidConvoSystemMessages", false);
    const normalized = try tr.resolveTranscript(a, context, mid);
    var grammar = try constrained.createGrammarToolInputPropertiesWithError(a, try tr.getDeclaredTools(a, normalized.messages), compatFlag(model, "supportsOpenAIGrammarTools", false), error_message);
    defer grammar.deinit(a);
    const allowed: []const []const u8 = if (std.mem.eql(u8, model.api, "azure-openai-responses")) &.{ "openai", "openai-codex", "opencode", "azure" } else &.{ "openai", "openai-codex", "opencode" };
    return convertResponsesMessages(a, io, model, context, allowed, .{
        .includeSystemPrompt = include_system,
        .grammarToolInputProperties = grammar,
        .supportsMidConvoSystemMessages = mid,
        .supportsAdditionalTools = compatFlag(model, "supportsAdditionalTools", false),
        .supportsToolSearch = compatFlag(model, "supportsToolSearch", false),
        .toolOptions = .{ .strict = if (std.mem.eql(u8, model.api, "openai-codex-responses")) null else false, .supportsStrictMode = compatFlag(model, "supportsStrictMode", false), .supportsOpenAIGrammarTools = compatFlag(model, "supportsOpenAIGrammarTools", false), .errorMessage = error_message },
        .configurationUpdateEffortValue = configuration_effort,
        .errorMessage = error_message,
    });
}
pub fn convertResponsesMessages(a: std.mem.Allocator, io: std.Io, model: t.Model, context: t.TranscriptContext, allowed_providers: []const []const u8, options: ConvertResponsesMessagesOptions) !Value {
    const normalized = try tr.resolveTranscript(a, context, options.supportsMidConvoSystemMessages);
    var allowed = false;
    for (allowed_providers) |provider| if (std.mem.eql(u8, provider, model.provider)) {
        allowed = true;
        break;
    };
    const transformed = try @import("transform_messages.zig").transformMessagesWithNormalizer(a, io, normalized.messages, model, .{ .context = &allowed, .call = struct {
        fn call(ctx: ?*anyopaque, allocator: std.mem.Allocator, id: []const u8, target: t.Model, source: t.AssistantMessage) ![]const u8 {
            const enabled: *const bool = @ptrCast(@alignCast(ctx.?));
            return normalizeIdAllowed(allocator, id, target, source, enabled.*);
        }
    }.call });
    const tools = try tr.resolveTranscriptTools(a, normalized.messages, options.supportsAdditionalTools or options.supportsToolSearch);
    const grammar_properties = options.grammarToolInputProperties;
    const configuration_effort = options.configurationUpdateEffortValue orelse if (options.configurationUpdateEffort) |value| @as(?Value, j.str(value)) else null;
    const include_system = options.includeSystemPrompt;
    const error_message = options.errorMessage;
    var out = j.array(a);
    var msg_index: usize = 0;
    for (transformed, 0..) |message, source_index| {
        const leading = source_index == 0 and message == .system;
        switch (message) {
            .system => |s| {
                if (!leading and tools.anchorsAdditions) try additions(a, &out, s.toolsAdded orelse &.{}, options, msg_index, error_message);
                if (!leading or include_system) {
                    const text = if (leading) try txt.getSystemMessageText(a, s) else try txt.renderSystemMessageUpdate(a, s);
                    if (text.len > 0) try out.array.append(try j.from(a, .{ .role = if (model.reasoning and c.compatBool(model, "supportsDeveloperRole", true)) "developer" else "system", .content = try sanitizeText(a, text) }));
                }
            },
            .user => |user| {
                if (user.content == .missing or user.content == .nullContent) return error.InvalidMessageContent;
                var content = j.array(a);
                if (user.content == .string) try content.array.append(try j.from(a, .{ .type = "input_text", .text = try sanitizeText(a, user.content.string) })) else for (user.content.blocks) |block| switch (block) {
                    .text => |v| try content.array.append(try j.from(a, .{ .type = "input_text", .text = try sanitizeText(a, v.text) })),
                    .image => |v| try content.array.append(try j.from(a, .{ .type = "input_image", .detail = "auto", .image_url = try c.imageDataUri(a, v) })),
                    else => {},
                };
                if (content.array.items.len == 0) continue;
                var item = try j.from(a, .{ .role = "user" });
                try j.put(a, &item, "content", content);
                try out.array.append(item);
            },
            .assistant => |assistant| {
                const same_api = std.mem.eql(u8, assistant.provider, model.provider) and std.mem.eql(u8, assistant.api, model.api);
                const same_model = same_api and std.mem.eql(u8, assistant.model, model.id);
                const output_start = out.array.items.len;
                var text_index: usize = 0;
                for (assistant.content) |block| switch (block) {
                    .thinking => |thinking| {
                        if (thinking.thinkingSignature) |sig| if (sig.len > 0) {
                            try out.array.append(try j.parseFromSliceLeaky(Value, a, sig, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }));
                        };
                    },
                    .text => |text| {
                        const signature = try parseTextSignature(a, text.textSignature);
                        const signature_id = if (signature) |sig| (if (sig.id.len > 0) @as(?[]const u8, sig.id) else null) else null;
                        var id = signature_id orelse if (text_index == 0) try std.fmt.allocPrint(a, "msg_pi_{d}", .{msg_index}) else try std.fmt.allocPrint(a, "msg_pi_{d}_{d}", .{ msg_index, text_index });
                        text_index += 1;
                        if (try @import("../utils/unicode.zig").utf16Length(id) > 64) id = try std.fmt.allocPrint(a, "msg_{s}", .{try hash(a, id)});
                        var item = try j.from(a, .{ .type = "message", .role = "assistant", .content = .{.{ .type = "output_text", .text = try sanitizeText(a, text.text), .annotations = j.array(a) }}, .status = "completed", .id = id });
                        if (signature) |sig| if (sig.phase) |phase| try j.put(a, &item, "phase", j.str(phase));
                        try out.array.append(item);
                    },
                    .toolCall => |call| {
                        var parts = std.mem.splitScalar(u8, call.id, '|');
                        const call_id = parts.first();
                        const item_id = parts.next();
                        const custom_property = grammar_properties.get(call.name);
                        var item = if (custom_property) |property| try j.from(a, .{ .type = "custom_tool_call", .call_id = call_id, .name = call.name, .input = try @import("../utils/unicode.zig").sanitizeSurrogates(a, try constrained.getGrammarToolInputWithError(a, call.name, call.arguments, property, error_message)) }) else try j.from(a, .{ .type = "function_call", .call_id = call_id, .name = call.name, .arguments = try @import("../json/value.zig").stringify(a, call.arguments, .{}) });
                        if (item_id) |id| if (!(same_api and !same_model) and std.mem.startsWith(u8, id, if (custom_property != null) "ctc_" else "fc_")) try j.put(a, &item, "id", j.str(id));
                        if (same_model) if (try j.fieldValue(a, call, "namespace")) |namespace| try j.put(a, &item, "namespace", namespace);
                        try out.array.append(item);
                    },
                    else => {},
                };
                if (out.array.items.len == output_start) continue;
                if (configuration_effort != null and same_model) if (try j.fieldValue(a, assistant, "providerThinkingLevel")) |effort| try out.array.insert(output_start, try j.from(a, .{ .type = "configuration_update", .reasoning = .{ .effort = effort } }));
            },
            .toolResult => |result| {
                var parts = std.mem.splitScalar(u8, result.toolCallId, '|');
                var item = try j.from(a, .{ .type = if (grammar_properties.contains(result.toolName)) "custom_tool_call_output" else "function_call_output", .call_id = parts.first() });
                try j.put(a, &item, "output", try toolOutput(a, model, result.content));
                try out.array.append(item);
            },
        }
        if (!leading) msg_index += 1;
    }
    if (configuration_effort) |effort| try out.array.append(try j.from(a, .{ .type = "configuration_update", .reasoning = .{ .effort = effort } }));
    return out;
}
pub fn buildPayload(a: std.mem.Allocator, io: std.Io, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !Value {
    const resolved = resolveReasoningOptions(model, options);
    const subscription = isChatGPTSignIn(model, options.apiKey);
    const configuration = getConfigurationUpdateEffortValue(model, options);
    const normalized = try tr.resolveTranscript(a, context, compatFlag(model, "supportsMidConvoSystemMessages", false));
    var payload = try j.from(a, .{ .model = model.id, .input = try convertMessagesWithEffortValue(a, io, model, normalized, configuration, true, options.errorMessage), .stream = true });
    const retention = c.cacheRetention(options);
    if (retention != .none) if (try @import("openai_prompt_cache.zig").clampOpenAIPromptCacheKey(options.sessionId)) |session| try j.put(a, &payload, "prompt_cache_key", j.str(session));
    if (!subscription) {
        if (compatFlag(model, "supportsExplicitPromptCacheMode", false)) {
            if (retention == .none) try j.put(a, &payload, "prompt_cache_options", try j.from(a, .{ .mode = "explicit" })) else if (retention == .long and compatFlag(model, "supportsLongCacheRetention", true)) try j.put(a, &payload, "prompt_cache_options", try j.from(a, .{ .ttl = "30m" }));
        } else if (retention == .long and compatFlag(model, "supportsLongCacheRetention", true)) try j.put(a, &payload, "prompt_cache_retention", j.str("24h"));
    }
    try j.put(a, &payload, "store", .{ .bool = false });
    if (!subscription) {
        if (options.maxTokens) |n| if (n != 0 and !std.math.isNan(n) and compatFlag(model, "supportsMaxOutputTokens", true)) try j.put(a, &payload, "max_output_tokens", j.num(@import("../utils/numbers.zig").maximum(n, 16)));
        if (try j.fieldValue(a, options, "temperature")) |value| try j.put(a, &payload, "temperature", value);
    }
    if (c.extra(options, "serviceTier")) |tier| try j.put(a, &payload, "service_tier", tier);
    const tools = try tr.resolveTranscriptTools(a, normalized.messages, compatFlag(model, "supportsAdditionalTools", false) or compatFlag(model, "supportsToolSearch", false));
    if (tools.requestTools.len > 0) try j.put(a, &payload, "tools", try convertToolsWithError(a, tools.requestTools, model, false, options.errorMessage));
    if (options.toolChoice) |choice| try j.put(a, &payload, "tool_choice", j.str(@tagName(choice)));
    if (c.extra(options, "toolChoice")) |choice| try j.put(a, &payload, "tool_choice", choice);
    if (model.reasoning) {
        if (resolved.requested != null and resolved.requested.?.len > 0) {
            const requested = resolved.requested.?;
            var reasoning = try j.from(a, .{ .effort = if (configuration != null) j.str("medium") else resolved.mappedValue orelse j.str(requested) });
            try j.put(a, &reasoning, "summary", resolved.summary);
            try j.put(a, &payload, "reasoning", reasoning);
            try j.put(a, &payload, "include", try j.from(a, [_][]const u8{"reasoning.encrypted_content"}));
        } else if (!c.is(model, "github-copilot")) {
            const disabled = if (model.thinkingLevelMap) |map| j.get(map, "off") else null;
            if (disabled == null or disabled.? != .null) try j.put(a, &payload, "reasoning", try j.from(a, .{ .effort = disabled orelse j.str("none") }));
        }
        if (c.is(model, "xai")) try j.put(a, &payload, "include", try j.from(a, [_][]const u8{"reasoning.encrypted_content"}));
    }
    var sampling_options = options;
    sampling_options.reasoning = resolved.level;
    try c.sampling(a, &payload, model, sampling_options);
    return payload;
}

pub fn openAIResponsesApi() @import("../models.zig").ApiImplementation {
    return @import("driver.zig").implementation("openai-responses");
}
pub fn stream(operation: @import("../models.zig").OperationContext, model: t.Model, context: t.TranscriptContext, options: OpenAIResponsesOptions) !*@import("../utils/event_stream.zig").AssistantMessageEventStream {
    if (!std.mem.eql(u8, model.api, "openai-responses")) return error.MismatchedApi;
    var arena = std.heap.ArenaAllocator.init(operation.allocator);
    defer arena.deinit();
    return @import("driver.zig").stream(null, operation, model, context, try options.streamOptions(arena.allocator()));
}
pub fn streamSimple(operation: @import("../models.zig").OperationContext, model: t.Model, context: t.TranscriptContext, options: t.SimpleStreamOptions) !*@import("../utils/event_stream.zig").AssistantMessageEventStream {
    if (!std.mem.eql(u8, model.api, "openai-responses")) return error.MismatchedApi;
    return @import("driver.zig").streamSimple(null, operation, model, context, options);
}
