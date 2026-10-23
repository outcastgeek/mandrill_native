const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const c = @import("common.zig");
const transcript = @import("../utils/transcript.zig");
const text = @import("../utils/text.zig");
const transform = @import("transform_messages.zig");
const Value = std.json.Value;
const sanitizeText = @import("../utils/unicode.zig").sanitizeSurrogates;
pub const OpenAICompletionsOptions = @import("options.zig").OpenAICompletionsOptions;
pub fn endpoint(a: std.mem.Allocator, base: []const u8) !std.Uri {
    return @import("openai_request_url.zig").endpoint(a, base, "/chat/completions");
}
pub fn getCompat(a: std.mem.Allocator, model: t.Model) !Value {
    const zai = c.is(model, "zai") or c.is(model, "zai-coding-cn") or c.has(model, "api.z.ai") or c.has(model, "open.bigmodel.cn");
    const together = c.is(model, "together") or c.has(model, "api.together.ai") or c.has(model, "api.together.xyz");
    const moonshot = c.is(model, "moonshotai") or c.is(model, "moonshotai-cn") or c.has(model, "api.moonshot.");
    const router = c.is(model, "openrouter") or c.has(model, "openrouter.ai");
    const workers = c.is(model, "cloudflare-workers-ai") or c.has(model, "api.cloudflare.com");
    const gateway = c.is(model, "cloudflare-ai-gateway") or c.has(model, "gateway.ai.cloudflare.com");
    const nvidia = c.is(model, "nvidia") or c.has(model, "integrate.api.nvidia.com");
    const ant = c.is(model, "ant-ling") or c.has(model, "api.ant-ling.com");
    const lower_base = try std.ascii.allocLowerString(a, model.baseUrl);
    defer a.free(lower_base);
    const deepseek = c.is(model, "deepseek") or std.mem.find(u8, lower_base, "deepseek.com") != null;
    const grok = c.is(model, "xai") or c.has(model, "api.x.ai");
    const nonstandard = nvidia or c.is(model, "cerebras") or c.has(model, "cerebras.ai") or grok or together or c.has(model, "chutes.ai") or deepseek or zai or moonshot or c.is(model, "opencode") or c.has(model, "opencode.ai") or workers or gateway or ant;
    var out = try j.from(a, .{
        .supportsStore = !nonstandard,
        .supportsDeveloperRole = (router and (std.mem.startsWith(u8, model.id, "anthropic/") or std.mem.startsWith(u8, model.id, "openai/"))) or (!nonstandard and !router),
        .supportsReasoningEffort = !grok and !zai and !moonshot and !together and !gateway and !nvidia and !ant,
        .supportsUsageInStreaming = true,
        .supportsFinishReason = true,
        .maxTokensField = if (c.has(model, "chutes.ai") or deepseek or moonshot or gateway or together or nvidia or ant or zai) "max_tokens" else "max_completion_tokens",
        .requiresToolResultName = false,
        .requiresAssistantAfterToolResult = false,
        .requiresThinkingAsText = false,
        .requiresReasoningContentOnAssistantMessages = deepseek,
        .thinkingFormat = if (deepseek) "deepseek" else if (zai) "zai" else if (together) "together" else if (ant) "ant-ling" else if (router) "openrouter" else "openai",
        .zaiToolStream = false,
        .supportsThinkingTokenBudget = false,
        .supportsStrictMode = false,
        .supportsOpenAIGrammarTools = false,
        .supportsMidConvoSystemMessages = false,
        .supportsMidConvoToolAdditions = false,
        .sendSessionAffinityHeaders = router,
        .sessionAffinityFormat = if (router) "openrouter" else "openai",
        .supportsLongCacheRetention = !(together or workers or gateway or nvidia or ant),
    });
    if (c.is(model, "openrouter") and std.mem.startsWith(u8, model.id, "anthropic/")) try j.put(a, &out, "cacheControlFormat", j.str("anthropic"));
    for ([_][]const u8{ "openRouterRouting", "vercelGatewayRouting", "chatTemplateKwargs", "chatTemplateArgs" }) |name| try j.put(a, &out, name, j.map(a));
    if (model.compat) |provided| if (provided == .object) {
        var fields = provided.object.iterator();
        while (fields.next()) |field| {
            const name = field.key_ptr.*;
            const value = field.value_ptr.*;
            if (std.mem.eql(u8, name, "vllmPriority")) {
                try j.put(a, &out, name, value);
                continue;
            }
            if (value == .null) continue;
            if (j.get(out, name) != null or std.mem.eql(u8, name, "cacheControlFormat") or std.mem.eql(u8, name, "thinkingTokenBudgetField")) try j.put(a, &out, name, value);
        }
    };
    return out;
}
fn flag(compat: Value, key: []const u8) bool {
    return j.truthy(j.get(compat, key) orelse .null);
}
fn strictFlag(compat: Value, key: []const u8) bool {
    return j.boolean(j.get(compat, key) orelse .null) orelse false;
}
fn setting(compat: Value, key: []const u8, default: []const u8) []const u8 {
    return j.field(compat, key) orelse default;
}
pub fn convertTools(a: std.mem.Allocator, tools: []const t.Tool, compat: Value) !Value {
    return convertToolsWithError(a, tools, compat, null);
}
pub fn convertToolsWithError(a: std.mem.Allocator, tools: []const t.Tool, compat: Value, error_message: ?*?[]const u8) !Value {
    var out = j.array(a);
    for (tools) |tool| {
        if (try @import("../schema/constrained.zig").resolveGrammarConstrainedSamplingWithError(a, tool, j.truthy(j.get(compat, "supportsOpenAIGrammarTools") orelse .null), error_message)) |grammar| {
            try out.array.append(try j.from(a, .{ .type = "custom", .custom = .{ .name = tool.name, .description = tool.description, .format = .{ .type = "grammar", .grammar = .{ .syntax = @tagName(grammar.format), .definition = grammar.definition } } } }));
            continue;
        }
        const strict_capability = j.get(compat, "supportsStrictMode") orelse .null;
        const supports_strict = !(strict_capability == .bool and !strict_capability.bool);
        const strict = try @import("../schema/constrained.zig").resolveJsonSchemaStrictSamplingWithError(a, tool, supports_strict, null, error_message);
        var function = try j.from(a, .{ .name = tool.name, .description = tool.description });
        try j.put(a, &function, "parameters", try @import("../schema/constrained.zig").getJsonSchemaToolParametersWithError(a, tool, strict, error_message));
        if (supports_strict) try j.put(a, &function, "strict", .{ .bool = strict orelse false });
        var entry = try j.from(a, .{ .type = "function" });
        try j.put(a, &entry, "function", function);
        try out.array.append(entry);
    }
    return out;
}
fn normalizeId(a: std.mem.Allocator, id: []const u8, model: t.Model, _: t.AssistantMessage) ![]const u8 {
    if (std.mem.findScalar(u8, id, '|')) |separator| {
        const call = try transform.normalizeAsciiToolId(a, id[0..separator], std.math.maxInt(usize));
        const item = try transform.normalizeAsciiToolId(a, id[separator + 1 ..], std.math.maxInt(usize));
        const combined = if (item.len > 0) try std.fmt.allocPrint(a, "{s}_{s}", .{ call, item }) else call;
        if (combined.len <= 40) return combined;
        const hash = try @import("../utils/hash.zig").shortHash(a, id);
        const hash_length: usize = @min(hash.len, 8);
        const prefix = call[0..@min(call.len, @max(1, 40 - hash_length - 1))];
        return std.fmt.allocPrint(a, "{s}_{s}", .{ prefix, hash[0..hash_length] });
    }
    if (c.is(model, "openai") and try @import("../utils/unicode.zig").utf16Length(id) > 40) {
        const units = try std.unicode.wtf8ToWtf16LeAlloc(a, id);
        defer a.free(units);
        return std.unicode.wtf16LeToWtf8Alloc(a, units[0..40]);
    }
    return id;
}
pub fn convertMessages(a: std.mem.Allocator, io: std.Io, model: t.Model, context: t.TranscriptContext, compat: Value) !Value {
    return convertMessagesWithError(a, io, model, context, compat, null);
}
pub const ConvertCompletionsMessagesOptions = struct { grammarToolInputProperties: std.StringHashMapUnmanaged([]const u8) = .{}, errorMessage: ?*?[]const u8 = null };
pub fn convertMessagesWithError(a: std.mem.Allocator, io: std.Io, model: t.Model, context: t.TranscriptContext, compat: Value, error_message: ?*?[]const u8) !Value {
    var grammar = try @import("../schema/constrained.zig").createGrammarToolInputPropertiesWithError(a, try transcript.getDeclaredTools(a, context.messages), flag(compat, "supportsOpenAIGrammarTools"), error_message);
    defer grammar.deinit(a);
    return convertMessagesWithOptions(a, io, model, context, compat, .{ .grammarToolInputProperties = grammar, .errorMessage = error_message });
}
pub fn convertMessagesWithOptions(a: std.mem.Allocator, io: std.Io, model: t.Model, context: t.TranscriptContext, compat: Value, options: ConvertCompletionsMessagesOptions) !Value {
    const error_message = options.errorMessage;
    const grammar_properties = options.grammarToolInputProperties;
    const normalized = try transcript.resolveTranscript(a, context, flag(compat, "supportsMidConvoSystemMessages"));
    const messages = try transform.transformMessages(a, io, normalized.messages, model, normalizeId);
    const tools = try transcript.resolveTranscriptTools(a, normalized.messages, strictFlag(compat, "supportsMidConvoSystemMessages") and strictFlag(compat, "supportsMidConvoToolAdditions"));
    var out = j.array(a);
    var last: ?std.meta.Tag(t.Message) = null;
    var i: usize = 0;
    while (i < messages.len) : (i += 1) {
        const message = messages[i];
        if (flag(compat, "requiresAssistantAfterToolResult") and last == .toolResult and message == .user) try out.array.append(try j.from(a, .{ .role = "assistant", .content = "I have processed the tool results." }));
        switch (message) {
            .system => |s| {
                if (i > 0 and tools.anchorsAdditions and (s.toolsAdded orelse @as([]const t.Tool, &.{})).len > 0) {
                    var addition = try j.from(a, .{ .role = "system" });
                    try j.put(a, &addition, "tools", try convertToolsWithError(a, s.toolsAdded.?, compat, error_message));
                    try out.array.append(addition);
                }
                const content = if (i == 0) try text.getSystemMessageText(a, s) else try text.renderSystemMessageUpdate(a, s);
                if (content.len > 0) try out.array.append(try j.from(a, .{ .role = if (model.reasoning and flag(compat, "supportsDeveloperRole")) "developer" else "system", .content = try sanitizeText(a, content) }));
            },
            .user => |u| {
                if (u.content == .missing or u.content == .nullContent) return error.InvalidMessageContent;
                var entry = try j.from(a, .{ .role = "user" });
                if (u.content == .string) try j.put(a, &entry, "content", j.str(try sanitizeText(a, u.content.string))) else {
                    var parts = j.array(a);
                    for (u.content.blocks) |block| switch (block) {
                        .text => |v| if (v.text.len > 0) {
                            try parts.array.append(try j.from(a, .{ .type = "text", .text = try sanitizeText(a, v.text) }));
                        },
                        .image => |v| try parts.array.append(try j.from(a, .{ .type = "image_url", .image_url = .{ .url = try c.imageDataUri(a, v) } })),
                        else => {},
                    };
                    if (parts.array.items.len == 0) continue;
                    try j.put(a, &entry, "content", parts);
                }
                try out.array.append(entry);
            },
            .assistant => |assistant| {
                var entry = try j.from(a, .{ .role = "assistant" });
                try j.put(a, &entry, "content", if (flag(compat, "requiresAssistantAfterToolResult")) j.str("") else .null);
                var texts: std.ArrayList([]const u8) = .empty;
                var thoughts: std.ArrayList([]const u8) = .empty;
                var calls = j.array(a);
                var signature: ?[]const u8 = null;
                var details: ?Value = null;
                var legacy_details = j.array(a);
                for (assistant.content) |block| switch (block) {
                    .text => |v| if (!try @import("../utils/unicode.zig").isBlank(v.text)) {
                        try texts.append(a, try sanitizeText(a, v.text));
                    },
                    .thinking => |v| {
                        if (!try @import("../utils/unicode.zig").isBlank(v.thinking)) {
                            if (thoughts.items.len == 0) signature = v.thinkingSignature;
                            try thoughts.append(a, v.thinking);
                        }
                        if (details == null) details = try @import("openai_reasoning_details.zig").parseOpenAIReasoningDetails(a, v.thinkingSignature);
                    },
                    .toolCall => |call| {
                        if (try @import("openai_reasoning_details.zig").parseLegacyEncryptedReasoningDetail(a, call.thoughtSignature)) |detail| try legacy_details.array.append(detail);
                        if (grammar_properties.get(call.name)) |property| try calls.array.append(try j.from(a, .{ .id = call.id, .type = "custom", .custom = .{ .name = call.name, .input = try @import("../utils/unicode.zig").sanitizeSurrogates(a, try @import("../schema/constrained.zig").getGrammarToolInputWithError(a, call.name, call.arguments, property, error_message)) } })) else try calls.array.append(try j.from(a, .{ .id = call.id, .type = "function", .function = .{ .name = call.name, .arguments = try @import("../json/value.zig").stringify(a, call.arguments, .{}) } }));
                    },
                    else => {},
                };
                if (details == null and legacy_details.array.items.len > 0) details = legacy_details;
                if (texts.items.len > 0) try j.put(a, &entry, "content", j.str(try std.mem.join(a, "", texts.items)));
                if (thoughts.items.len > 0) {
                    if (flag(compat, "requiresThinkingAsText")) {
                        var parts = j.array(a);
                        try parts.array.append(try j.from(a, .{ .type = "text", .text = try sanitizeText(a, try std.mem.join(a, "\n\n", thoughts.items)) }));
                        for (texts.items) |s| try parts.array.append(try j.from(a, .{ .type = "text", .text = s }));
                        try j.put(a, &entry, "content", parts);
                    } else if (details == null) if (signature) |sig| {
                        const field = if (c.is(model, "opencode-go") and std.mem.eql(u8, sig, "reasoning")) "reasoning_content" else sig;
                        if (std.mem.eql(u8, field, "reasoning") or std.mem.eql(u8, field, "reasoning_content") or std.mem.eql(u8, field, "reasoning_text")) try j.put(a, &entry, field, j.str(try std.mem.join(a, "\n", thoughts.items)));
                    };
                }
                if (calls.array.items.len > 0) try j.put(a, &entry, "tool_calls", calls);
                if (details) |v| try j.put(a, &entry, "reasoning_details", v);
                if (flag(compat, "requiresReasoningContentOnAssistantMessages") and model.reasoning and j.get(entry, "reasoning_content") == null) try j.put(a, &entry, "reasoning_content", j.str(""));
                if (texts.items.len == 0 and calls.array.items.len == 0 and (!flag(compat, "requiresThinkingAsText") or thoughts.items.len == 0)) continue;
                try out.array.append(entry);
            },
            .toolResult => {
                var image_parts = j.array(a);
                try image_parts.array.append(try j.from(a, .{ .type = "text", .text = "Attached image(s) from tool result:" }));
                while (i < messages.len and messages[i] == .toolResult) : (i += 1) {
                    const result = messages[i].toolResult;
                    const result_text = try text.contentText(a, .{ .blocks = result.content }, "\n");
                    const has_images = for (result.content) |block| {
                        if (block == .image) break true;
                    } else false;
                    const content = if (!try @import("../utils/unicode.zig").isBlank(result_text)) result_text else if (has_images) "(see attached image)" else "(no tool output)";
                    var entry = try j.from(a, .{ .role = "tool", .content = try sanitizeText(a, content), .tool_call_id = result.toolCallId });
                    if (flag(compat, "requiresToolResultName") and result.toolName.len > 0) try j.put(a, &entry, "name", j.str(result.toolName));
                    try out.array.append(entry);
                    const vision = for (model.input) |modality| {
                        if (modality == .image) break true;
                    } else false;
                    if (vision) for (result.content) |block| if (block == .image) {
                        try image_parts.array.append(try j.from(a, .{ .type = "image_url", .image_url = .{ .url = try c.imageDataUri(a, block.image) } }));
                    };
                }
                i -= 1;
                if (image_parts.array.items.len > 1) {
                    if (flag(compat, "requiresAssistantAfterToolResult")) try out.array.append(try j.from(a, .{ .role = "assistant", .content = "I have processed the tool results." }));
                    var user = try j.from(a, .{ .role = "user" });
                    try j.put(a, &user, "content", image_parts);
                    try out.array.append(user);
                    last = .user;
                } else last = .toolResult;
                continue;
            },
        }
        last = std.meta.activeTag(message);
    }
    return out;
}
pub fn buildPayload(a: std.mem.Allocator, io: std.Io, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !Value {
    const compat = try getCompat(a, model);
    var payload = try j.from(a, .{ .model = model.id, .messages = try convertMessagesWithError(a, io, model, context, compat, options.errorMessage), .stream = true });
    const retention = c.cacheRetention(options);
    if (try @import("openai_prompt_cache.zig").clampOpenAIPromptCacheKey(options.sessionId)) |id| if ((c.has(model, "api.openai.com") and retention != .none) or (retention == .long and flag(compat, "supportsLongCacheRetention"))) try j.put(a, &payload, "prompt_cache_key", j.str(id));
    if (retention == .long and flag(compat, "supportsLongCacheRetention")) try j.put(a, &payload, "prompt_cache_retention", j.str("24h"));
    const streaming_usage = j.get(compat, "supportsUsageInStreaming") orelse .null;
    if (!(streaming_usage == .bool and !streaming_usage.bool)) try j.put(a, &payload, "stream_options", try j.from(a, .{ .include_usage = true }));
    if (flag(compat, "supportsStore")) try j.put(a, &payload, "store", .{ .bool = false });
    if (options.maxTokens) |n| if (n != 0 and !std.math.isNan(n)) try j.put(a, &payload, if (std.mem.eql(u8, setting(compat, "maxTokensField", "max_completion_tokens"), "max_tokens")) "max_tokens" else "max_completion_tokens", j.num(n));
    if (options.temperature) |n| try j.put(a, &payload, "temperature", j.num(n));
    const tools = try transcript.resolveTranscriptTools(a, context.messages, strictFlag(compat, "supportsMidConvoSystemMessages") and strictFlag(compat, "supportsMidConvoToolAdditions"));
    if (tools.requestTools.len > 0) {
        try j.put(a, &payload, "tools", try convertToolsWithError(a, tools.requestTools, compat, options.errorMessage));
        if (flag(compat, "zaiToolStream")) try j.put(a, &payload, "tool_stream", .{ .bool = true });
    } else {
        const history = history: {
            for (context.messages) |message| {
                if (message == .toolResult) break :history true;
                if (message == .assistant) for (message.assistant.content) |block| {
                    if (block == .toolCall) break :history true;
                };
            }
            break :history false;
        };
        if (history) try j.put(a, &payload, "tools", j.array(a));
    }
    if (options.toolChoice) |choice| try j.put(a, &payload, "tool_choice", j.str(@tagName(choice)));
    if (c.extra(options, "toolChoice")) |choice| if (j.truthy(choice)) {
        try j.put(a, &payload, "tool_choice", choice);
    };
    if (j.get(compat, "vllmPriority")) |priority| try j.put(a, &payload, "priority", priority);
    const requested = if (c.extra(options, "reasoningEffort")) |value| j.text(value) else if (options.reasoning) |level| (if (level == .off) null else @tagName(level)) else null;
    const enabled = requested != null and requested.?.len > 0;
    const level = if (enabled) std.meta.stringToEnum(t.ThinkingLevel, requested.?) orelse .off else .off;
    const raw_effort = if (model.thinkingLevelMap) |map| j.get(map, if (enabled) requested.? else "off") else null;
    const effort = if (raw_effort) |value| (if (value == .null) j.str(if (enabled) requested.? else "none") else value) else j.str(if (enabled) requested.? else "none");
    const off_omitted = raw_effort != null and raw_effort.? == .null;
    const format = setting(compat, "thinkingFormat", "openai");
    const ceiling = j.number(j.get(payload, "max_tokens") orelse j.get(payload, "max_completion_tokens") orelse j.num(model.maxTokens)) orelse model.maxTokens;
    const budget: ?f64 = if (enabled and model.reasoning) blk: {
        const selected = @import("simple_options.zig").clampThinkingBudgetToAnswerRoom(try @import("simple_options.zig").budgetNumber(a, try @import("simple_options.zig").thinkingBudgetForLevel(a, level, options.thinkingBudgets)), try @import("simple_options.zig").modelTokenLimit(@max(0, ceiling)));
        break :blk if (selected > 0) selected else null;
    } else null;
    if (model.reasoning) {
        if (std.mem.eql(u8, format, "zai")) {
            try j.put(a, &payload, "thinking", if (enabled) try j.from(a, .{ .type = "enabled", .clear_thinking = false }) else try j.from(a, .{ .type = "disabled" }));
            if (enabled and flag(compat, "supportsReasoningEffort")) {
                const mapped = raw_effort orelse j.str(requested.?);
                if (mapped == .string) try j.put(a, &payload, "reasoning_effort", mapped);
            }
        } else if (std.mem.eql(u8, format, "qwen")) {
            try j.put(a, &payload, "enable_thinking", .{ .bool = enabled });
            if (enabled and flag(compat, "supportsReasoningEffort") and effort == .string) try j.put(a, &payload, "reasoning_effort", effort);
        } else if (std.mem.eql(u8, format, "qwen-chat-template")) try j.put(a, &payload, "chat_template_kwargs", try j.from(a, .{ .enable_thinking = enabled, .preserve_thinking = true })) else if (std.mem.eql(u8, format, "chat-template") or std.mem.eql(u8, format, "baseten")) {
            const key = if (std.mem.eql(u8, format, "baseten")) "chatTemplateArgs" else "chatTemplateKwargs";
            if (try buildChatTemplateValues(a, model, if (enabled) requested else null, j.get(compat, key) orelse .null, budget)) |values| try j.put(a, &payload, if (std.mem.eql(u8, format, "baseten")) "chat_template_args" else "chat_template_kwargs", values);
            if (std.mem.eql(u8, format, "baseten") and flag(compat, "supportsReasoningEffort")) {
                if (raw_effort) |mapped| {
                    if (mapped == .string) try j.put(a, &payload, "reasoning_effort", mapped);
                } else if (enabled) try j.put(a, &payload, "reasoning_effort", j.str(requested.?));
            }
        } else if (std.mem.eql(u8, format, "deepseek")) {
            if (enabled or !off_omitted) try j.put(a, &payload, "thinking", try j.from(a, .{ .type = if (enabled) "enabled" else "disabled" }));
            if (enabled and flag(compat, "supportsReasoningEffort")) try j.put(a, &payload, "reasoning_effort", effort);
        } else if (std.mem.eql(u8, format, "openrouter")) {
            if (enabled or !off_omitted) try j.put(a, &payload, "reasoning", try j.from(a, .{ .effort = effort }));
        } else if (std.mem.eql(u8, format, "ant-ling") and enabled) {
            if (raw_effort) |mapped| if (mapped == .string) {
                try j.put(a, &payload, "reasoning", try j.from(a, .{ .effort = mapped }));
            };
        } else if (std.mem.eql(u8, format, "together")) {
            try j.put(a, &payload, "reasoning", try j.from(a, .{ .enabled = enabled }));
            if (enabled and flag(compat, "supportsReasoningEffort")) try j.put(a, &payload, "reasoning_effort", effort);
        } else if (std.mem.eql(u8, format, "string-thinking")) {
            if (enabled or !off_omitted) try j.put(a, &payload, "thinking", effort);
        } else if (flag(compat, "supportsReasoningEffort")) {
            if (enabled) try j.put(a, &payload, "reasoning_effort", effort) else if (raw_effort) |mapped| if (mapped == .string) {
                try j.put(a, &payload, "reasoning_effort", mapped);
            };
        }
    }
    if (budget) |tokens| {
        const configured = j.field(compat, "thinkingTokenBudgetField");
        const field: ?[]const u8 = if (configured != null and configured.?.len > 0) configured else if (flag(compat, "supportsThinkingTokenBudget")) "thinking_token_budget" else null;
        if (field) |name| try j.put(a, &payload, name, j.num(tokens));
    }
    if (std.mem.eql(u8, setting(compat, "cacheControlFormat", ""), "anthropic") and retention != .none) {
        var control = try j.from(a, .{ .type = "ephemeral" });
        if (retention == .long and flag(compat, "supportsLongCacheRetention")) try j.put(a, &control, "ttl", j.str("1h"));
        try applyAnthropicCacheControl(a, &payload, control);
    }
    if (j.get(model.compat orelse .null, "openRouterRouting")) |routing| if (j.truthy(routing)) {
        try j.put(a, &payload, "provider", routing);
    };
    if (j.get(model.compat orelse .null, "vercelGatewayRouting")) |routing| if (j.truthy(routing)) {
        var gateway = j.map(a);
        for ([_][]const u8{ "only", "order" }) |key| if (j.get(routing, key)) |value| if (j.truthy(value)) {
            try j.put(a, &gateway, key, value);
        };
        if (gateway.object.count() > 0) {
            var provider_options = j.map(a);
            try j.put(a, &provider_options, "gateway", gateway);
            try j.put(a, &payload, "providerOptions", provider_options);
        }
    };
    var sampling_options = options;
    sampling_options.reasoning = level;
    try c.sampling(a, &payload, model, sampling_options);
    return payload;
}
fn buildChatTemplateValues(a: std.mem.Allocator, model: t.Model, requested: ?[]const u8, values: Value, budget: ?f64) !?Value {
    if (values != .object) return null;
    var output = j.map(a);
    const entries = try j.objectEntries(a, values.object);
    defer a.free(entries);
    for (entries) |entry| {
        if (std.mem.eql(u8, entry.key, "__proto__")) continue;
        var value = entry.node.*;
        if (value == .object) {
            if (requested == null and j.truthy(j.get(value, "omitWhenOff") orelse .null)) continue;
            const variable = j.field(value, "$var") orelse "";
            if (std.mem.eql(u8, variable, "thinking.enabled")) value = .{ .bool = requested != null } else if (std.mem.eql(u8, variable, "thinking.budget")) {
                value = j.num(budget orelse continue);
            } else {
                const mapped = if (model.thinkingLevelMap) |map| j.get(map, requested orelse "off") else null;
                if (mapped) |item| {
                    if (item != .string) continue;
                    value = item;
                } else value = j.str(requested orelse continue);
            }
        }
        try j.put(a, &output, entry.key, value);
    }
    return if (output.object.count() > 0) output else null;
}
fn addCacheControlToTextContent(a: std.mem.Allocator, message: *Value, control: Value) !bool {
    const content = message.object.getPtr("content") orelse return false;
    if (content.* == .string) {
        if (content.string.len == 0) return false;
        var part = try j.from(a, .{ .type = "text", .text = content.string });
        try j.put(a, &part, "cache_control", control);
        var parts = j.array(a);
        try parts.array.append(part);
        content.* = parts;
        return true;
    }
    if (content.* != .array) return false;
    var index = content.array.items.len;
    while (index > 0) {
        index -= 1;
        const part = &content.array.items[index];
        if (std.mem.eql(u8, j.field(part.*, "type") orelse "", "text")) {
            try j.put(a, part, "cache_control", control);
            return true;
        }
    }
    return false;
}
fn applyAnthropicCacheControl(a: std.mem.Allocator, payload: *Value, control: Value) !void {
    const messages = payload.object.getPtr("messages").?;
    for (messages.array.items) |*message| {
        const role = j.field(message.*, "role") orelse "";
        if (std.mem.eql(u8, role, "system") or std.mem.eql(u8, role, "developer")) {
            _ = try addCacheControlToTextContent(a, message, control);
            break;
        }
    }
    if (payload.object.getPtr("tools")) |tools| if (tools.* == .array and tools.array.items.len > 0) try j.put(a, &tools.array.items[tools.array.items.len - 1], "cache_control", control);
    var index = messages.array.items.len;
    while (index > 0) {
        index -= 1;
        const message = &messages.array.items[index];
        const role = j.field(message.*, "role") orelse "";
        if (std.mem.eql(u8, role, "user") or std.mem.eql(u8, role, "assistant") or std.mem.eql(u8, role, "tool")) if (try addCacheControlToTextContent(a, message, control)) break;
    }
}
pub fn parseChunkUsage(a: std.mem.Allocator, raw: Value, model: t.Model) !t.Usage {
    const accounting = @import("../utils/usage.zig");
    const details = j.get(raw, "prompt_tokens_details") orelse .null;
    const raw_reads = j.getNonNull(details, "cached_tokens") orelse j.getNonNull(raw, "prompt_cache_hit_tokens") orelse j.getNonNull(raw, "cached_tokens") orelse j.int(0);
    const writes = accounting.orZero(details, "cache_write_tokens");
    var usage = t.Usage{ .input = @import("../utils/numbers.zig").maximum(0, try accounting.number(a, accounting.orZero(raw, "prompt_tokens")) - try accounting.number(a, raw_reads) - try accounting.number(a, writes)) };
    try accounting.set(a, &usage, "output", accounting.orZero(raw, "completion_tokens"));
    try accounting.set(a, &usage, "cacheRead", raw_reads);
    try accounting.set(a, &usage, "cacheWrite", writes);
    try accounting.set(a, &usage, "reasoning", accounting.orZero(j.get(raw, "completion_tokens_details") orelse .null, "reasoning_tokens"));
    try accounting.total(a, &usage);
    _ = try @import("../utils/cost.zig").calculateCost(a, model, &usage);
    return usage;
}
pub fn mapStopReason(reason: []const u8) t.StopReason {
    if (std.mem.eql(u8, reason, "stop") or std.mem.eql(u8, reason, "end")) return .stop;
    if (std.mem.eql(u8, reason, "length")) return .length;
    if (std.mem.eql(u8, reason, "function_call") or std.mem.eql(u8, reason, "tool_calls")) return .toolUse;
    return .@"error";
}

pub fn openAICompletionsApi() @import("../models.zig").ApiImplementation {
    return @import("driver.zig").implementation("openai-completions");
}
pub fn stream(operation: @import("../models.zig").OperationContext, model: t.Model, context: t.TranscriptContext, options: OpenAICompletionsOptions) !*@import("../utils/event_stream.zig").AssistantMessageEventStream {
    if (!std.mem.eql(u8, model.api, "openai-completions")) return error.MismatchedApi;
    var arena = std.heap.ArenaAllocator.init(operation.allocator);
    defer arena.deinit();
    return @import("driver.zig").stream(null, operation, model, context, try options.streamOptions(arena.allocator()));
}
pub fn streamSimple(operation: @import("../models.zig").OperationContext, model: t.Model, context: t.TranscriptContext, options: t.SimpleStreamOptions) !*@import("../utils/event_stream.zig").AssistantMessageEventStream {
    if (!std.mem.eql(u8, model.api, "openai-completions")) return error.MismatchedApi;
    return @import("driver.zig").streamSimple(null, operation, model, context, options);
}
