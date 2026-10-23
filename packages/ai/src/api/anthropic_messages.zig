const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const c = @import("common.zig");
const tr = @import("../utils/transcript.zig");
const txt = @import("../utils/text.zig");
const transform = @import("transform_messages.zig");
const constrained = @import("../schema/constrained.zig");
const Value = std.json.Value;
const sanitizeText = @import("../utils/unicode.zig").sanitizeSurrogates;
pub const AnthropicOptions = @import("options.zig").AnthropicOptions;
pub const AnthropicEffort = @import("options.zig").AnthropicEffort;
pub const AnthropicThinkingDisplay = @import("options.zig").AnthropicThinkingDisplay;
pub fn endpoint(a: std.mem.Allocator, base: []const u8) !std.Uri {
    return @import("openai_request_url.zig").endpoint(a, if (base.len > 0) base else "https://api.anthropic.com", "/v1/messages?beta=true");
}
pub fn compatFlag(model: t.Model, key: []const u8, default: bool) bool {
    return j.truthy(j.getNonNull(model.compat orelse .null, key) orelse .{ .bool = default });
}
pub fn providerThinkingLevel(model: t.Model, options: t.StreamOptions) ?[]const u8 {
    const enabled = j.boolean(c.extra(options, "thinkingEnabled") orelse .null);
    if (!c.compatBool(model, "supportsMidConvoEffort", false) or (model.reasoning and enabled == false and @import("../utils/thinking.zig").supports(model, .off))) return null;
    return j.text(c.extra(options, "effort") orelse j.str("high")) orelse "high";
}
pub fn getBetaFeatures(a: std.mem.Allocator, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) ![]const []const u8 {
    var configured: ?t.ProviderHeader = null;
    for (try @import("../utils/headers.zig").fromValue(a, model.headers)) |header| if (std.ascii.eqlIgnoreCase(header.name, "anthropic-beta")) {
        configured = header;
    };
    for (options.headers) |header| if (std.ascii.eqlIgnoreCase(header.name, "anthropic-beta")) {
        configured = header;
    };
    var features: std.StringArrayHashMapUnmanaged(void) = .{};
    if (configured) |header| {
        const value = header.value orelse return &.{};
        var values = std.mem.splitScalar(u8, value, ',');
        while (values.next()) |item| {
            const feature = try @import("../utils/unicode.zig").trimWhitespace(a, item);
            if (feature.len > 0) try features.put(a, feature, {});
        }
    } else {
        if (options.apiKey) |key| if (!options.authFromClient and std.mem.find(u8, key, "sk-ant-oat") != null and !c.is(model, "github-copilot")) {
            try features.put(a, "claude-code-20250219", {});
            try features.put(a, "oauth-2025-04-20", {});
        };
        if ((try tr.getCurrentTools(a, context.messages)).len > 0 and !compatFlag(model, "supportsEagerToolInputStreaming", true)) try features.put(a, "fine-grained-tool-streaming-2025-05-14", {});
        const enabled = j.boolean(c.extra(options, "thinkingEnabled") orelse .null) orelse false;
        if (model.reasoning and enabled and j.truthy(j.getNonNull(options.extra orelse .null, "interleavedThinking") orelse .{ .bool = true }) and !c.compatBool(model, "forceAdaptiveThinking", false)) try features.put(a, "interleaved-thinking-2025-05-14", {});
        if ((c.array(model.compat orelse .null, "allowedFallbackModels")).len > 0) try features.put(a, "server-side-fallback-2026-07-01", {});
        if (c.compatBool(model, "supportsMidConvoEffort", false)) {
            try features.put(a, "mid-conversation-output-config-2026-07-01", {});
            try features.put(a, "thinking-binding-controls-2026-08-01", {});
        }
        const normalized = try tr.resolveTranscript(a, context, compatFlag(model, "supportsMidConvoSystemMessages", false));
        const initial = tr.getInitialSystemMessage(normalized.messages);
        if (compatFlag(model, "supportsMidConvoSystemMessages", false) and compatFlag(model, "supportsMidConvoToolChanges", false) and (if (initial) |value| (value.toolsAdded orelse @as([]const t.Tool, &.{})).len > 0 else false)) try features.put(a, "inline-tools-2026-09-15", {});
    }
    const result = try a.dupe([]const u8, features.keys());
    features.deinit(a);
    return result;
}
pub fn toClaudeCodeName(name: []const u8) []const u8 {
    for ([_][]const u8{ "Read", "Write", "Edit", "Bash", "Grep", "Glob", "AskUserQuestion", "EnterPlanMode", "ExitPlanMode", "KillShell", "NotebookEdit", "Skill", "Task", "TaskOutput", "TodoWrite", "WebFetch", "WebSearch" }) |canonical| if (std.ascii.eqlIgnoreCase(name, canonical)) return canonical;
    return name;
}
pub fn fromClaudeCodeName(name: []const u8, tools: []const t.Tool) []const u8 {
    for (tools) |tool| if (std.ascii.eqlIgnoreCase(tool.name, name)) return tool.name;
    return name;
}
fn normalizeId(a: std.mem.Allocator, id: []const u8, _: t.Model, _: t.AssistantMessage) ![]const u8 {
    return transform.normalizeAsciiToolId(a, id, 64);
}
pub fn imageBlock(a: std.mem.Allocator, image: t.ImageContent) !Value {
    return j.from(a, .{ .type = "image", .source = .{ .type = "base64", .media_type = image.mimeType, .data = image.data } });
}
fn contentBlocks(a: std.mem.Allocator, blocks: []const t.Content) !Value {
    const has_images = for (blocks) |block| {
        if (block == .image) break true;
    } else false;
    if (!has_images) {
        var texts: std.ArrayList([]const u8) = .empty;
        for (blocks) |block| if (block == .text) {
            try texts.append(a, try sanitizeText(a, block.text.text));
        };
        return j.str(try std.mem.join(a, "\n", texts.items));
    }
    var out = j.array(a);
    var has_text = false;
    for (blocks) |block| switch (block) {
        .text => |v| {
            has_text = true;
            try out.array.append(try j.from(a, .{ .type = "text", .text = try sanitizeText(a, v.text) }));
        },
        .image => |v| try out.array.append(try imageBlock(a, v)),
        else => {},
    };
    if (!has_text) try out.array.insert(0, try j.from(a, .{ .type = "text", .text = "(see attached image)" }));
    return out;
}
fn cacheControl(a: std.mem.Allocator, model: t.Model, options: t.StreamOptions) !?Value {
    const retention = c.cacheRetention(options);
    if (retention == .none) return null;
    var out = try j.from(a, .{ .type = "ephemeral" });
    if (retention == .long and compatFlag(model, "supportsLongCacheRetention", true)) try j.put(a, &out, "ttl", j.str("1h"));
    return out;
}
pub fn convertTools(a: std.mem.Allocator, tools: []const t.Tool, model: t.Model, oauth: bool, cache: ?Value) !Value {
    return convertToolsWithFallback(a, tools, model, oauth, cache, false, null, null);
}
fn strictUnsupportedKeyword(_: ?*anyopaque, key: []const u8, value: Value) anyerror!bool {
    const blocked = [_][]const u8{ "minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum", "multipleOf", "maxItems", "uniqueItems", "minContains", "maxContains", "minProperties", "maxProperties" };
    for (blocked) |name| if (std.mem.eql(u8, key, name)) return true;
    if (std.mem.eql(u8, key, "minItems")) {
        const count = j.number(value) orelse return true;
        return count != 0 and count != 1;
    }
    if (std.mem.eql(u8, key, "format")) {
        const format = j.text(value) orelse return true;
        const allowed = for ([_][]const u8{ "date-time", "time", "date", "duration", "email", "hostname", "uri", "ipv4", "ipv6", "uuid" }) |candidate| {
            if (std.mem.eql(u8, format, candidate)) break true;
        } else false;
        return !allowed;
    }
    return false;
}
fn convertToolsWithFallback(a: std.mem.Allocator, tools: []const t.Tool, model: t.Model, oauth: bool, cache: ?Value, disable_preferred: bool, error_message: ?*?[]const u8, has_preferred: ?*bool) !Value {
    var out = j.array(a);
    for (tools, 0..) |tool, i| {
        const config = tool.constrainedSampling orelse .null;
        const preferred = std.mem.eql(u8, j.field(config, "type") orelse "", "json_schema") and std.mem.eql(u8, j.field(config, "strict") orelse "", "prefer");
        const strict = if (disable_preferred and preferred) null else try constrained.resolveJsonSchemaStrictSamplingWithError(a, tool, compatFlag(model, "supportsStrictTools", false), .{ .call = strictUnsupportedKeyword }, error_message);
        if ((strict orelse false) and preferred) if (has_preferred) |flag| {
            flag.* = true;
        };
        const parameters = try constrained.getJsonSchemaToolParametersWithError(a, tool, strict, error_message);
        if (parameters == .null) {
            if (error_message) |slot| slot.* = "Cannot read properties of null (reading 'properties')";
            return error.InvalidAnthropicToolSchema;
        }
        var schema = if (strict orelse false) try j.clone(a, parameters) else j.map(a);
        try j.put(a, &schema, "type", j.str("object"));
        try j.put(a, &schema, "properties", j.getNonNull(parameters, "properties") orelse j.map(a));
        try j.put(a, &schema, "required", j.getNonNull(parameters, "required") orelse j.array(a));
        var entry = try j.from(a, .{ .name = if (oauth) toClaudeCodeName(tool.name) else tool.name, .description = tool.description });
        try j.put(a, &entry, "input_schema", schema);
        if (compatFlag(model, "supportsEagerToolInputStreaming", true)) try j.put(a, &entry, "eager_input_streaming", .{ .bool = true });
        if (strict orelse false) try j.put(a, &entry, "strict", .{ .bool = true });
        if (cache) |v| if (i == tools.len - 1) try j.put(a, &entry, "cache_control", v);
        try out.array.append(entry);
    }
    return out;
}
pub fn buildPayload(a: std.mem.Allocator, io: std.Io, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !Value {
    return buildPayloadWithFallback(a, io, model, context, options, false);
}
pub fn buildPayloadWithFallback(a: std.mem.Allocator, io: std.Io, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions, disable_preferred: bool) !Value {
    return (try buildPayloadDetails(a, io, model, context, options, disable_preferred)).payload;
}
pub const BuiltPayload = struct { payload: Value, hasPreferredStrictTools: bool, providerThinkingLevel: ?Value };
pub fn buildPayloadDetails(a: std.mem.Allocator, io: std.Io, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions, disable_preferred: bool) !BuiltPayload {
    var has_preferred = false;
    const oauth = !options.authFromClient and !c.is(model, "github-copilot") and (if (options.apiKey) |key| std.mem.find(u8, key, "sk-ant-oat") != null else false);
    const normalized = try tr.resolveTranscript(a, context, compatFlag(model, "supportsMidConvoSystemMessages", false));
    const initial = tr.getInitialSystemMessage(normalized.messages);
    const messages = try transform.transformMessages(a, io, tr.withoutInitialSystemMessage(normalized.messages), model, normalizeId);
    const cache = try cacheControl(a, model, options);
    const thinking_enabled = j.boolean(c.extra(options, "thinkingEnabled") orelse .null);
    const enabled = j.truthy(c.extra(options, "thinkingEnabled") orelse .null);
    const off_supported = @import("../utils/thinking.zig").supports(model, .off);
    const disabled = model.reasoning and thinking_enabled == false and off_supported;
    const managed = c.compatBool(model, "supportsMidConvoEffort", false) and !disabled;
    const initial_tools = if (initial) |s| s.toolsAdded orelse &.{} else &.{};
    const native_changes = compatFlag(model, "supportsMidConvoSystemMessages", false) and compatFlag(model, "supportsMidConvoToolChanges", false) and initial_tools.len > 0;
    var conversation = j.array(a);
    var held = j.array(a);
    var i: usize = 0;
    while (i < messages.len) : (i += 1) {
        switch (messages[i]) {
            .system => |s| {
                var blocks = j.array(a);
                const text = try txt.renderSystemMessageUpdate(a, s);
                if (text.len > 0) try blocks.array.append(try j.from(a, .{ .type = "text", .text = try sanitizeText(a, text) }));
                if (native_changes) {
                    const added = s.toolsAdded orelse &.{};
                    for (s.toolsRemoved orelse &.{}) |removed| {
                        const replaced = for (added) |tool| {
                            if (std.mem.eql(u8, tool.name, removed.name)) break true;
                        } else false;
                        if (!replaced) try blocks.array.append(try j.from(a, .{ .type = "tool_removal", .tool = .{ .type = "tool_reference", .name = if (oauth) toClaudeCodeName(removed.name) else removed.name } }));
                    }
                    const definitions = try convertToolsWithFallback(a, added, model, oauth, null, disable_preferred, options.errorMessage, &has_preferred);
                    for (definitions.array.items) |definition| {
                        var block = try j.from(a, .{ .type = "tool_addition", .tool = .{ .type = "tool_definition" } });
                        var tool = j.get(block, "tool").?;
                        try j.put(a, &tool, "definition", definition);
                        try j.put(a, &block, "tool", tool);
                        try blocks.array.append(block);
                    }
                }
                if (blocks.array.items.len > 0) {
                    var item = try j.from(a, .{ .role = "system" });
                    try j.put(a, &item, "content", blocks);
                    try held.array.append(item);
                }
            },
            .user => |user| {
                if (user.content == .missing or user.content == .nullContent) return error.InvalidMessageContent;
                if (user.content == .string) {
                    if (!try @import("../utils/unicode.zig").isBlank(user.content.string)) try conversation.array.append(try j.from(a, .{ .role = "user", .content = try sanitizeText(a, user.content.string) }));
                } else {
                    var blocks = j.array(a);
                    for (user.content.blocks) |block| switch (block) {
                        .text => |value| try blocks.array.append(try j.from(a, .{ .type = "text", .text = try sanitizeText(a, value.text) })),
                        .image => |value| try blocks.array.append(try imageBlock(a, value)),
                        else => {},
                    };
                    var index: usize = 0;
                    while (index < blocks.array.items.len) {
                        const block = blocks.array.items[index];
                        if (std.mem.eql(u8, j.field(block, "type") orelse "", "text") and try @import("../utils/unicode.zig").isBlank(j.field(block, "text") orelse "")) {
                            _ = blocks.array.orderedRemove(index);
                        } else index += 1;
                    }
                    if (blocks.array.items.len > 0) {
                        var item = try j.from(a, .{ .role = "user" });
                        try j.put(a, &item, "content", blocks);
                        try conversation.array.append(item);
                    }
                }
            },
            .assistant => |assistant| {
                try conversation.array.appendSlice(held.array.items);
                held.array.clearRetainingCapacity();
                var blocks = j.array(a);
                for (assistant.content) |block| switch (block) {
                    .text => |v| if (!try @import("../utils/unicode.zig").isBlank(v.text)) {
                        try blocks.array.append(try j.from(a, .{ .type = "text", .text = try sanitizeText(a, v.text) }));
                    },
                    .thinking => |v| {
                        if (v.redacted orelse false) {
                            var redacted = try j.from(a, .{ .type = "redacted_thinking" });
                            if (try j.fieldValue(a, v, "thinkingSignature")) |data| try j.put(a, &redacted, "data", data);
                            try blocks.array.append(redacted);
                            continue;
                        }
                        const signed = if (v.thinkingSignature) |s| !try @import("../utils/unicode.zig").isBlank(s) else false;
                        if (try @import("../utils/unicode.zig").isBlank(v.thinking) and !signed) continue;
                        if (!signed and !compatFlag(model, "allowEmptySignature", false)) try blocks.array.append(try j.from(a, .{ .type = "text", .text = try sanitizeText(a, v.thinking) })) else try blocks.array.append(try j.from(a, .{ .type = "thinking", .thinking = try sanitizeText(a, v.thinking), .signature = if (signed) v.thinkingSignature.? else "" }));
                    },
                    .toolCall => |call| {
                        var item = try j.from(a, .{ .type = "tool_use", .id = call.id, .name = if (oauth) toClaudeCodeName(call.name) else call.name });
                        try j.put(a, &item, "input", if (call.arguments == .null) j.map(a) else call.arguments);
                        try blocks.array.append(item);
                    },
                    else => {},
                };
                if (blocks.array.items.len > 0) {
                    if (managed and std.mem.eql(u8, assistant.api, "anthropic-messages") and std.mem.eql(u8, assistant.provider, model.provider)) if (assistant.providerThinkingLevel) |effort| {
                        if (std.meta.stringToEnum(AnthropicEffort, effort) != null) try conversation.array.append(try j.from(a, .{ .role = "system", .content = j.array(a), .output_config = .{ .effort = effort } }));
                    };
                    var item = try j.from(a, .{ .role = "assistant" });
                    try j.put(a, &item, "content", blocks);
                    try conversation.array.append(item);
                }
            },
            .toolResult => {
                var blocks = j.array(a);
                var error_images = j.array(a);
                while (i < messages.len and messages[i] == .toolResult) : (i += 1) {
                    const result = messages[i].toolResult;
                    const split = try transform.splitErrorToolResultImages(a, result);
                    var item = try j.from(a, .{ .type = "tool_result", .tool_use_id = result.toolCallId, .is_error = result.isError });
                    try j.put(a, &item, "content", try contentBlocks(a, if (split) |v| v.content else result.content));
                    try blocks.array.append(item);
                    if (split) |v| {
                        try error_images.array.append(try j.from(a, .{ .type = "text", .text = v.caption }));
                        for (v.images) |image| try error_images.array.append(try imageBlock(a, image));
                    }
                }
                i -= 1;
                try blocks.array.appendSlice(error_images.array.items);
                var item = try j.from(a, .{ .role = "user" });
                try j.put(a, &item, "content", blocks);
                try conversation.array.append(item);
            },
        }
    }
    try conversation.array.appendSlice(held.array.items);
    if (cache) |control| if (conversation.array.items.len > 0) {
        const last = &conversation.array.items[conversation.array.items.len - 1];
        const role = j.field(last.*, "role") orelse "";
        if (std.mem.eql(u8, role, "user") or std.mem.eql(u8, role, "system")) {
            const content = last.object.getPtr("content").?;
            if (content.* == .string) {
                var parts = j.array(a);
                var block = try j.from(a, .{ .type = "text", .text = content.string });
                try j.put(a, &block, "cache_control", control);
                try parts.array.append(block);
                content.* = parts;
            } else if (content.* == .array and content.array.items.len > 0) try j.put(a, &content.array.items[content.array.items.len - 1], "cache_control", control);
        }
    };
    const active_effort = j.getNonNull(options.extra orelse .null, "effort") orelse j.str("high");
    if (managed) try conversation.array.append(try j.from(a, .{ .role = "system", .content = j.array(a), .output_config = .{ .effort = active_effort } }));
    var payload = try j.from(a, .{ .model = model.id, .messages = conversation, .max_tokens = options.maxTokens orelse try @import("simple_options.zig").modelTokenLimit(model.maxTokens), .stream = true });
    const betas = try getBetaFeatures(a, model, normalized, options);
    if (betas.len > 0) try j.put(a, &payload, "betas", try j.from(a, betas));
    var system = j.array(a);
    if (oauth) {
        var identity = try j.from(a, .{ .type = "text", .text = "You are Claude Code, Anthropic's official CLI for Claude." });
        if (cache) |v| try j.put(a, &identity, "cache_control", v);
        try system.array.append(identity);
    }
    if (initial) |s| {
        const text = try txt.getSystemMessageText(a, s);
        if (text.len > 0) {
            var block = try j.from(a, .{ .type = "text", .text = try sanitizeText(a, text) });
            if (cache) |v| try j.put(a, &block, "cache_control", v);
            try system.array.append(block);
        }
    }
    if (system.array.items.len > 0) try j.put(a, &payload, "system", system);
    if (options.temperature) |temperature| if (!enabled and !c.compatBool(model, "supportsMidConvoEffort", false) and compatFlag(model, "supportsTemperature", true)) try j.put(a, &payload, "temperature", j.num(temperature));
    const selected_tools = if (native_changes) initial_tools else try tr.getCurrentTools(a, normalized.messages);
    if (selected_tools.len > 0) {
        var tools = try convertToolsWithFallback(a, selected_tools, model, oauth, if (compatFlag(model, "supportsCacheControlOnTools", true)) cache else null, disable_preferred, options.errorMessage, &has_preferred);
        if (native_changes) try tools.array.append(try j.from(a, .{ .name = "__pi_deferred_placeholder__", .description = "Reserved placeholder. Never available. Never call this.", .input_schema = .{ .type = "object", .properties = .{}, .required = j.array(a) }, .defer_loading = true }));
        try j.put(a, &payload, "tools", tools);
    }
    if (disabled) try j.put(a, &payload, "thinking", try j.from(a, .{ .type = "disabled" })) else if (managed) {
        try j.put(a, &payload, "thinking", try j.from(a, .{ .type = "adaptive", .display = j.getNonNull(options.extra orelse .null, "thinkingDisplay") orelse j.str("summarized"), .block_binding = .{ .prefix_mismatch_behavior = "drop_block" } }));
        try j.put(a, &payload, "output_config", try j.from(a, .{ .effort = "high" }));
    } else if (model.reasoning and enabled) {
        const display = j.getNonNull(options.extra orelse .null, "thinkingDisplay") orelse j.str("summarized");
        if (c.compatBool(model, "forceAdaptiveThinking", false)) {
            try j.put(a, &payload, "thinking", try j.from(a, .{ .type = "adaptive", .display = display }));
            if (c.extra(options, "effort")) |effort| if (j.truthy(effort)) {
                try j.put(a, &payload, "output_config", try j.from(a, .{ .effort = effort }));
            };
        } else {
            const budget = c.extra(options, "thinkingBudgetTokens") orelse .null;
            try j.put(a, &payload, "thinking", try j.from(a, .{ .type = "enabled", .budget_tokens = if (j.truthy(budget)) budget else j.int(1024), .display = display }));
        }
    }
    if (options.metadata) |metadata| if (j.field(metadata, "user_id")) |user_id| try j.put(a, &payload, "metadata", try j.from(a, .{ .user_id = user_id }));
    if (c.extra(options, "toolChoice")) |choice| {
        if (j.truthy(choice)) try j.put(a, &payload, "tool_choice", if (choice == .string) try j.from(a, .{ .type = choice.string }) else try j.clone(a, choice));
    } else if (options.toolChoice) |choice| try j.put(a, &payload, "tool_choice", try j.from(a, .{ .type = @tagName(choice) }));
    if (model.compat) |compat| if (j.get(compat, "allowedFallbackModels")) |fallbacks| {
        var out = j.array(a);
        if (fallbacks == .array) for (fallbacks.array.items) |fallback| {
            if (fallback == .null) {
                if (options.errorMessage) |slot| slot.* = "Cannot read properties of null (reading 'model')";
                return error.InvalidAnthropicFallback;
            }
            var item = j.map(a);
            if (j.get(fallback, "model")) |value| try j.put(a, &item, "model", value);
            try out.array.append(item);
        };
        if (out.array.items.len > 0) try j.put(a, &payload, "fallbacks", out);
    };
    return .{ .payload = payload, .hasPreferredStrictTools = has_preferred, .providerThinkingLevel = if (managed) active_effort else null };
}
pub fn mapStopReason(reason: []const u8) !t.StopReason {
    if (std.mem.eql(u8, reason, "end_turn") or std.mem.eql(u8, reason, "pause_turn") or std.mem.eql(u8, reason, "stop_sequence")) return .stop;
    if (std.mem.eql(u8, reason, "max_tokens")) return .length;
    if (std.mem.eql(u8, reason, "tool_use")) return .toolUse;
    if (std.mem.eql(u8, reason, "refusal") or std.mem.eql(u8, reason, "sensitive")) return .@"error";
    return error.UnhandledAnthropicStopReason;
}

pub fn anthropicMessagesApi() @import("../models.zig").ApiImplementation {
    return @import("driver.zig").implementation("anthropic-messages");
}
pub fn stream(operation: @import("../models.zig").OperationContext, model: t.Model, context: t.TranscriptContext, options: AnthropicOptions) !*@import("../utils/event_stream.zig").AssistantMessageEventStream {
    if (!std.mem.eql(u8, model.api, "anthropic-messages")) return error.MismatchedApi;
    var arena = std.heap.ArenaAllocator.init(operation.allocator);
    defer arena.deinit();
    return @import("driver.zig").stream(null, operation, model, context, try options.streamOptions(arena.allocator()));
}
pub fn streamSimple(operation: @import("../models.zig").OperationContext, model: t.Model, context: t.TranscriptContext, options: t.SimpleStreamOptions) !*@import("../utils/event_stream.zig").AssistantMessageEventStream {
    if (!std.mem.eql(u8, model.api, "anthropic-messages")) return error.MismatchedApi;
    return @import("driver.zig").streamSimple(null, operation, model, context, options);
}
