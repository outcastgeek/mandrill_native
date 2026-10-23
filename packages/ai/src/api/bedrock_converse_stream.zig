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
pub const BedrockOptions = @import("options.zig").BedrockOptions;
pub const BedrockThinkingDisplay = @import("options.zig").BedrockThinkingDisplay;
/// Native callback JSON byte-array helpers. Returned data is owned by a.
pub const blobValue = @import("bedrock_wire.zig").blobValue;
pub const blobBytes = @import("bedrock_wire.zig").blobBytes;
fn nonempty(value: ?[]const u8) ?[]const u8 {
    const text = value orelse return null;
    return if (text.len > 0) text else null;
}
pub fn endpoint(a: std.mem.Allocator, base_url: []const u8, payload: Value, error_message: ?*?[]const u8) !std.Uri {
    const identifier = j.getNonNull(payload, "modelId") orelse {
        if (error_message) |slot| slot.* = "No value provided for input HTTP label: modelId.";
        return error.MissingBedrockModelId;
    };
    const label = if (identifier == .object or identifier == .array) try j.stringify(a, identifier, .{ .whitespace = .indent_2 }) else try @import("../utils/diagnostics.zig").formatThrownValue(a, .{ .value = identifier });
    var target = try @import("../transport/uri.zig").parseHttpUrl(a, base_url);
    var path = std.Io.Writer.Allocating.init(a);
    defer path.deinit();
    try target.path.formatPath(&path.writer);
    const prefix = if (std.mem.eql(u8, path.written(), "/")) "" else path.written();
    target.path = .{ .percent_encoded = try std.mem.concat(a, u8, &.{ prefix, "/model/", try @import("../transport/uri.zig").encodeComponent(a, label), "/converse-stream" }) };
    return target;
}
pub fn isClaude(model: t.Model) bool {
    return std.ascii.findIgnoreCase(model.id, "anthropic.claude") != null or std.ascii.findIgnoreCase(model.id, "anthropic/claude") != null or std.ascii.findIgnoreCase(model.name, "claude") != null;
}
fn match(a: std.mem.Allocator, model: t.Model, needles: []const []const u8) !bool {
    var separators = try @import("../regex/regexp.zig").Regex.init(a, null, "[\\s_.:]+", .{});
    defer separators.deinit();
    for ([_][]const u8{ model.id, model.name }) |source| {
        const lower = try std.ascii.allocLowerString(a, source);
        defer a.free(lower);
        const units = try std.unicode.wtf8ToWtf16LeAlloc(a, lower);
        defer a.free(units);
        var normalized: std.ArrayList(u8) = .empty;
        defer normalized.deinit(a);
        var offset: usize = 0;
        while (try separators.exec(a, lower, offset)) |found| {
            var selected = found;
            defer selected.deinit();
            const span = selected.captures[0].?;
            const prefix = try std.unicode.wtf16LeToWtf8Alloc(a, units[offset..span.start]);
            defer a.free(prefix);
            try normalized.appendSlice(a, prefix);
            try normalized.append(a, '-');
            offset = span.end;
        }
        const tail = try std.unicode.wtf16LeToWtf8Alloc(a, units[offset..]);
        defer a.free(tail);
        try normalized.appendSlice(a, tail);
        for (needles) |needle| if (std.mem.find(u8, lower, needle) != null or std.mem.find(u8, normalized.items, needle) != null) return true;
    }
    return false;
}
pub fn supportsAdaptiveThinking(a: std.mem.Allocator, model: t.Model) !bool {
    return match(a, model, &.{ "opus-4-6", "opus-4-7", "opus-4-8", "opus-5", "sonnet-4-6", "sonnet-5", "haiku-5", "fable-5" });
}
fn supportsBinding(a: std.mem.Allocator, model: t.Model) !bool {
    return match(a, model, &.{ "opus-4-7", "opus-4-8", "opus-5", "sonnet-5", "haiku-5", "fable-5" });
}
pub fn supportsPromptCaching(a: std.mem.Allocator, model: t.Model, options: t.StreamOptions) !bool {
    if (!try match(a, model, &.{"claude"})) return if (options.env.get("AWS_BEDROCK_FORCE_CACHE")) |v| std.mem.eql(u8, v, "1") else false;
    return match(a, model, &.{ "fable-5", "opus-5", "sonnet-5", "haiku-5", "-4-", "claude-3-7-sonnet", "claude-3-5-haiku" });
}
fn normalizeId(a: std.mem.Allocator, id: []const u8, _: t.Model, _: t.AssistantMessage) ![]const u8 {
    const units = try std.unicode.wtf8ToWtf16LeAlloc(a, id);
    defer a.free(units);
    const out = try a.alloc(u8, @min(units.len, 64));
    for (units[0..out.len], out) |unit, *byte| byte.* = if (unit <= 127 and (std.ascii.isAlphanumeric(@intCast(unit)) or unit == '_' or unit == '-')) @intCast(unit) else '_';
    return out;
}
fn decodedBlob(a: std.mem.Allocator, input: []const u8) !Value {
    const decoded = try @import("../utils/base64.zig").decodeAtob(a, input);
    defer a.free(decoded);
    return @import("bedrock_wire.zig").blobValue(a, decoded);
}
fn imageBlock(a: std.mem.Allocator, image: t.ImageContent, error_message: ?*?[]const u8) !Value {
    const format = if (std.mem.eql(u8, image.mimeType, "image/jpeg") or std.mem.eql(u8, image.mimeType, "image/jpg")) "jpeg" else if (std.mem.eql(u8, image.mimeType, "image/png")) "png" else if (std.mem.eql(u8, image.mimeType, "image/gif")) "gif" else if (std.mem.eql(u8, image.mimeType, "image/webp")) "webp" else {
        if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Unknown image type: {s}", .{image.mimeType});
        return error.UnsupportedBedrockImageFormat;
    };
    // The SDK decodes the public image before its Smithy byte codec encodes it.
    return j.from(a, .{ .source = .{ .bytes = try decodedBlob(a, image.data) }, .format = format });
}
fn sanitizeDocument(a: std.mem.Allocator, input: Value) anyerror!Value {
    if (input == .object) {
        var out = j.map(a);
        var it = input.object.iterator();
        while (it.next()) |entry| if (entry.key_ptr.len > 0) {
            try j.put(a, &out, entry.key_ptr.*, try sanitizeDocument(a, entry.value_ptr.*));
        };
        return out;
    }
    if (input == .array) {
        var out = j.array(a);
        for (input.array.items) |item| try out.array.append(try sanitizeDocument(a, item));
        return out;
    }
    return input;
}
fn nonBlankText(a: std.mem.Allocator, input: []const u8) !?Value {
    const sanitized = try sanitizeText(a, input);
    if (try @import("../utils/unicode.zig").isBlank(sanitized)) return null;
    return try j.from(a, .{ .text = sanitized });
}
fn resultContent(a: std.mem.Allocator, blocks: []const t.Content, error_message: ?*?[]const u8) !Value {
    var out = j.array(a);
    for (blocks) |block| switch (block) {
        .text => |v| if (try nonBlankText(a, v.text)) |text| {
            try out.array.append(text);
        },
        .image => |v| {
            var item = j.map(a);
            try j.put(a, &item, "image", try imageBlock(a, v, error_message));
            try out.array.append(item);
        },
        else => {},
    };
    if (out.array.items.len == 0) try out.array.append(try j.from(a, .{ .text = "<empty>" }));
    return out;
}
fn cachePoint(a: std.mem.Allocator, retention: t.CacheRetention) !Value {
    var point = try j.from(a, .{ .type = "default" });
    if (retention == .long) try j.put(a, &point, "ttl", j.str("1h"));
    var out = j.map(a);
    try j.put(a, &out, "cachePoint", point);
    return out;
}
pub fn buildPayload(a: std.mem.Allocator, io: std.Io, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !Value {
    const collapsed = try tr.collapseSystemMessages(a, context);
    const initial = tr.getInitialSystemMessage(collapsed.messages);
    const messages = try transform.transformMessages(a, io, tr.withoutInitialSystemMessage(collapsed.messages), model, normalizeId);
    const retention = c.cacheRetention(options);
    const cache = retention != .none and try supportsPromptCaching(a, model, options);
    var conversation = j.array(a);
    var i: usize = 0;
    while (i < messages.len) : (i += 1) {
        var blocks = j.array(a);
        var role: []const u8 = "user";
        switch (messages[i]) {
            .user => |user| {
                if (user.content == .missing or user.content == .nullContent) return error.InvalidMessageContent;
                if (user.content == .string) try blocks.array.append((try nonBlankText(a, user.content.string)) orelse try j.from(a, .{ .text = "<empty>" })) else {
                    for (user.content.blocks) |block| switch (block) {
                        .text => |v| if (try nonBlankText(a, v.text)) |text| {
                            try blocks.array.append(text);
                        },
                        .image => |v| {
                            var part = j.map(a);
                            try j.put(a, &part, "image", try imageBlock(a, v, options.errorMessage));
                            try blocks.array.append(part);
                        },
                        else => {},
                    };
                    if (blocks.array.items.len == 0) try blocks.array.append(try j.from(a, .{ .text = "<empty>" }));
                }
            },
            .assistant => |assistant| {
                role = "assistant";
                for (assistant.content) |block| switch (block) {
                    .text => |v| if (try nonBlankText(a, v.text)) |text| {
                        try blocks.array.append(text);
                    },
                    .toolCall => |call| {
                        var use = try j.from(a, .{ .toolUseId = call.id, .name = call.name });
                        try j.put(a, &use, "input", try sanitizeDocument(a, call.arguments));
                        var part = j.map(a);
                        try j.put(a, &part, "toolUse", use);
                        try blocks.array.append(part);
                    },
                    .thinking => |v| {
                        if (v.redacted orelse false) {
                            if (v.thinkingSignature) |sig| if (sig.len > 0) {
                                const bytes = decodedBlob(a, sig) catch |err| {
                                    if (err == error.OutOfMemory) return err;
                                    continue;
                                };
                                if (bytes.array.items.len > 0) try blocks.array.append(try j.from(a, .{ .reasoningContent = .{ .redactedContent = bytes } }));
                            };
                            continue;
                        }
                        const thinking = try sanitizeText(a, v.thinking);
                        if (try @import("../utils/unicode.zig").isBlank(thinking)) continue;
                        if (isClaude(model) and (v.thinkingSignature == null or try @import("../utils/unicode.zig").isBlank(v.thinkingSignature.?))) {
                            try blocks.array.append(try j.from(a, .{ .text = thinking }));
                            continue;
                        }
                        var reasoning = try j.from(a, .{ .text = thinking });
                        if (isClaude(model)) if (v.thinkingSignature) |signature| try j.put(a, &reasoning, "signature", j.str(signature));
                        var content = j.map(a);
                        try j.put(a, &content, "reasoningText", reasoning);
                        var part = j.map(a);
                        try j.put(a, &part, "reasoningContent", content);
                        try blocks.array.append(part);
                    },
                    else => {},
                };
                if (blocks.array.items.len == 0) continue;
            },
            .toolResult => {
                var error_images = j.array(a);
                while (i < messages.len and messages[i] == .toolResult) : (i += 1) {
                    const result = messages[i].toolResult;
                    const split = if (isClaude(model)) try transform.splitErrorToolResultImages(a, result) else null;
                    var tool = try j.from(a, .{ .toolUseId = result.toolCallId });
                    try j.put(a, &tool, "content", try resultContent(a, if (split) |v| v.content else result.content, options.errorMessage));
                    try j.put(a, &tool, "status", j.str(if (result.isError) "error" else "success"));
                    var part = j.map(a);
                    try j.put(a, &part, "toolResult", tool);
                    try blocks.array.append(part);
                    if (split) |v| {
                        try error_images.array.append(try j.from(a, .{ .text = v.caption }));
                        for (v.images) |image| {
                            var image_part = j.map(a);
                            try j.put(a, &image_part, "image", try imageBlock(a, image, options.errorMessage));
                            try error_images.array.append(image_part);
                        }
                    }
                }
                i -= 1;
                try blocks.array.appendSlice(error_images.array.items);
            },
            .system => continue,
        }
        var message = try j.from(a, .{ .role = role });
        try j.put(a, &message, "content", blocks);
        try conversation.array.append(message);
    }
    if (cache and conversation.array.items.len > 0) {
        const last = &conversation.array.items[conversation.array.items.len - 1];
        if (std.mem.eql(u8, j.field(last.*, "role") orelse "", "user")) try last.object.getPtr("content").?.array.append(try cachePoint(a, retention));
    }
    var payload = try j.from(a, .{ .modelId = model.id });
    try j.put(a, &payload, "messages", conversation);
    if (initial) |s| {
        const prompt = try txt.getSystemMessageText(a, s);
        if (prompt.len > 0) {
            var system = j.array(a);
            try system.array.append(try j.from(a, .{ .text = try sanitizeText(a, prompt) }));
            if (cache) try system.array.append(try cachePoint(a, retention));
            try j.put(a, &payload, "system", system);
        }
    }
    var inference = j.map(a);
    const configured_max = try j.fieldValue(a, options, "maxTokens");
    if (configured_max != null and configured_max.? != .null) try j.put(a, &inference, "maxTokens", configured_max.?) else if (isClaude(model)) if (try j.fieldValue(a, model, "maxTokens")) |value| {
        try j.put(a, &inference, "maxTokens", value);
    };
    if (try j.fieldValue(a, options, "temperature")) |temperature| try j.put(a, &inference, "temperature", temperature);
    try j.put(a, &payload, "inferenceConfig", inference);
    const tools = try tr.getCurrentTools(a, collapsed.messages);
    const custom_choice = c.extra(options, "toolChoice");
    const omit_tools = options.toolChoice == .none or (custom_choice != null and custom_choice.? == .string and std.mem.eql(u8, custom_choice.?.string, "none"));
    if (tools.len > 0 and !omit_tools) {
        var definitions = j.array(a);
        for (tools) |tool| {
            const strict = try constrained.resolveJsonSchemaStrictSamplingWithError(a, tool, j.truthy(j.getNonNull(model.compat orelse .null, "supportsStrictMode") orelse .{ .bool = false }), null, options.errorMessage);
            var spec = try j.from(a, .{ .name = tool.name, .description = tool.description });
            var schema = j.map(a);
            try j.put(a, &schema, "json", try constrained.getJsonSchemaToolParametersWithError(a, tool, strict, options.errorMessage));
            try j.put(a, &spec, "inputSchema", schema);
            if (strict orelse false) try j.put(a, &spec, "strict", .{ .bool = true });
            var definition = j.map(a);
            try j.put(a, &definition, "toolSpec", spec);
            try definitions.array.append(definition);
        }
        var config = j.map(a);
        try j.put(a, &config, "tools", definitions);
        if (options.toolChoice) |choice| if (choice == .auto or choice == .any) {
            var selected = j.map(a);
            try j.put(a, &selected, @tagName(choice), j.map(a));
            try j.put(a, &config, "toolChoice", selected);
        };
        if (custom_choice) |choice| {
            if (choice == .string and (std.mem.eql(u8, choice.string, "auto") or std.mem.eql(u8, choice.string, "any"))) {
                var selected = j.map(a);
                try j.put(a, &selected, choice.string, j.map(a));
                try j.put(a, &config, "toolChoice", selected);
            } else if (choice == .object and std.mem.eql(u8, j.field(choice, "type") orelse "", "tool")) {
                var selected = j.map(a);
                if (j.get(choice, "name")) |name| try j.put(a, &selected, "name", name);
                var named = j.map(a);
                try j.put(a, &named, "tool", selected);
                try j.put(a, &config, "toolChoice", named);
            }
        }
        try j.put(a, &payload, "toolConfig", config);
    }
    if (options.reasoning) |level| if (level != .off and model.reasoning) {
        var fields = j.map(a);
        if (isClaude(model)) {
            const configured = if (c.extra(options, "region")) |v| j.text(v) else null;
            const region = nonempty(configured) orelse nonempty(options.env.get("AWS_REGION")) orelse nonempty(options.env.get("AWS_DEFAULT_REGION"));
            const gov = (if (region) |v| std.ascii.startsWithIgnoreCase(v, "us-gov-") else false) or std.ascii.startsWithIgnoreCase(model.id, "us-gov.") or std.ascii.startsWithIgnoreCase(model.id, "arn:aws-us-gov:");
            const adaptive = try supportsAdaptiveThinking(a, model);
            var thinking = try j.from(a, .{ .type = if (adaptive) "adaptive" else "enabled" });
            if (!gov) try j.put(a, &thinking, "display", j.getNonNull(options.extra orelse .null, "thinkingDisplay") orelse j.str("summarized"));
            if (adaptive) {
                const binding = !gov and try supportsBinding(a, model);
                if (binding) try j.put(a, &thinking, "block_binding", try j.from(a, .{ .prefix_mismatch_behavior = "drop_block" }));
                const mapped = if (model.thinkingLevelMap) |map| j.field(map, @tagName(level)) else null;
                const effort = if (level == .xhigh and try supportsBinding(a, model)) "xhigh" else mapped orelse switch (level) {
                    .minimal, .low => "low",
                    .medium => "medium",
                    else => "high",
                };
                try j.put(a, &fields, "thinking", thinking);
                try j.put(a, &fields, "output_config", try j.from(a, .{ .effort = effort }));
                if (binding) try j.put(a, &fields, "anthropic_beta", try j.from(a, [_][]const u8{"thinking-binding-controls-2026-08-01"}));
            } else {
                const simple = @import("simple_options.zig");
                const custom = try simple.thinkingBudgetForLevel(a, level, options.thinkingBudgets);
                const budget = if (custom) |value| (if (value == .null) try simple.thinkingBudgetForLevel(a, level, null) else value) else try simple.thinkingBudgetForLevel(a, level, null);
                if (budget) |value| try j.put(a, &thinking, "budget_tokens", value);
                try j.put(a, &fields, "thinking", thinking);
                const interleaved = j.truthy(j.getNonNull(options.extra orelse .null, "interleavedThinking") orelse .{ .bool = true });
                if (interleaved) try j.put(a, &fields, "anthropic_beta", try j.from(a, [_][]const u8{"interleaved-thinking-2025-05-14"}));
            }
        } else if (try match(a, model, &.{"gpt-oss"})) {
            try j.put(a, &fields, "reasoning_effort", j.str(switch (level) {
                .minimal, .low => "low",
                .medium => "medium",
                else => "high",
            }));
        } else if (try match(a, model, &.{"gpt-"})) {
            const mapped = if (model.thinkingLevelMap) |map| j.field(map, @tagName(level)) else null;
            const effort = mapped orelse switch (level) {
                .minimal => "low",
                else => @tagName(level),
            };
            try j.put(a, &fields, "reasoning", try j.from(a, .{ .effort = effort }));
        }
        if (fields.object.count() > 0) try j.put(a, &payload, "additionalModelRequestFields", fields);
    };
    if (c.extra(options, "requestMetadata")) |metadata| try j.put(a, &payload, "requestMetadata", metadata);
    return payload;
}
fn eventMember(value: Value, key: []const u8) ?Value {
    const member = j.get(value, key) orelse return null;
    return if (j.truthy(member)) member else null;
}
pub fn consume(self: *@import("reducer.zig").Machine, event: Value) !void {
    if (eventMember(event, "messageStart")) |start| {
        if (!std.mem.eql(u8, j.field(start, "role") orelse "", "assistant")) {
            self.message.errorMessage = "Unexpected assistant message start but got user message start instead";
            return error.UnexpectedBedrockMessageRole;
        }
        try self.start();
        return;
    }
    if (eventMember(event, "contentBlockStart")) |start| {
        if (eventMember(j.get(start, "start") orelse .null, "toolUse")) |tool| {
            _ = try self.beginProviderBlock(.{ .toolCall = .{ .id = j.field(tool, "toolUseId") orelse "", .name = j.field(tool, "name") orelse "", .arguments = j.map(self.allocator) } }, @import("reducer.zig").Machine.ProviderIndex.from(start, "contentBlockIndex"));
        }
        return;
    }
    if (eventMember(event, "contentBlockDelta")) |block_delta| {
        const wire_index = @import("reducer.zig").Machine.ProviderIndex.from(block_delta, "contentBlockIndex");
        const delta = j.get(block_delta, "delta") orelse return;
        if (j.get(delta, "text")) |text| {
            const index = self.findWireIndex(wire_index) orelse blk: {
                const created = try self.beginProviderBlock(.{ .text = .{ .text = "" } }, wire_index);
                break :blk created;
            };
            if (self.blocks.items[index] == .text) try self.providerValueDelta(index, text);
        } else if (eventMember(delta, "toolUse")) |tool| {
            if (self.findWireIndex(wire_index)) |index| if (self.blocks.items[index] == .toolCall) try self.providerValueDelta(index, eventMember(tool, "input") orelse j.str(""));
        } else if (eventMember(delta, "reasoningContent")) |reasoning| {
            const index = self.findWireIndex(wire_index) orelse blk: {
                const created = try self.beginProviderBlock(.{ .thinking = .{ .thinking = "", .thinkingSignature = "" } }, wire_index);
                break :blk created;
            };
            if (self.blocks.items[index] != .thinking) return;
            if (eventMember(reasoning, "text")) |text| try self.providerValueDelta(index, text);
            const thinking = &self.blocks.items[index].thinking;
            if (!(thinking.redacted orelse false)) if (eventMember(reasoning, "signature")) |signature| {
                const text = try @import("../utils/diagnostics.zig").formatThrownValue(self.allocator, .{ .value = signature });
                thinking.thinkingSignature = try std.mem.concat(self.allocator, u8, &.{ thinking.thinkingSignature orelse "", text });
            };
            if (j.get(reasoning, "redactedContent")) |content| {
                const bytes = switch (content) {
                    .string => |encoded| try @import("../utils/base64.zig").decodeSmithy(self.allocator, encoded),
                    .array => try @import("bedrock_wire.zig").blobBytes(self.allocator, content),
                    else => return,
                };
                defer self.allocator.free(bytes);
                if (bytes.len == 0) return;
                if (!(thinking.redacted orelse false)) {
                    thinking.redacted = true;
                    thinking.thinkingSignature = "";
                    try self.providerDelta(index, "[Reasoning redacted]");
                }
                try self.states.items[index].redactedBytes.appendSlice(self.allocator, bytes);
                self.states.items[index].redactedPending = true;
            }
        }
        return;
    }
    if (eventMember(event, "contentBlockStop")) |stop| {
        const wire_index = @import("reducer.zig").Machine.ProviderIndex.from(stop, "contentBlockIndex");
        if (self.findWireIndex(wire_index)) |index| {
            self.states.items[index].wireIndex = .{};
            try flushRedacted(self, index);
            try self.endProviderBlock(index);
        }
        return;
    }
    if (eventMember(event, "messageStop")) |stop| {
        self.terminal = true;
        const reason = j.field(stop, "stopReason") orelse "";
        self.message.rawStopReason = reason;
        self.message.stopReason = if (std.mem.eql(u8, reason, "end_turn") or std.mem.eql(u8, reason, "stop_sequence")) .stop else if (std.mem.eql(u8, reason, "max_tokens") or std.mem.eql(u8, reason, "model_context_window_exceeded")) .length else if (std.mem.eql(u8, reason, "tool_use")) .toolUse else .@"error";
        if (self.message.stopReason == .@"error" and reason.len > 0) {
            self.message.errorMessage = try std.fmt.allocPrint(self.allocator, "Provider stopped with: {s}", .{reason});
            self.finishReasonMessage = self.message.errorMessage;
        }
        return;
    }
    if (eventMember(event, "metadata")) |metadata| {
        if (j.get(metadata, "usage")) |usage| if (j.truthy(usage)) {
            const accounting = @import("../utils/usage.zig");
            const a = self.allocator;
            try accounting.set(a, &self.message.usage, "input", accounting.orZero(usage, "inputTokens"));
            try accounting.set(a, &self.message.usage, "output", accounting.orZero(usage, "outputTokens"));
            try accounting.set(a, &self.message.usage, "cacheRead", accounting.orZero(usage, "cacheReadInputTokens"));
            try accounting.set(a, &self.message.usage, "cacheWrite", accounting.orZero(usage, "cacheWriteInputTokens"));
            if (j.getNonNull(usage, "cacheDetails")) |details| {
                if (details != .array) return error.InvalidBedrockCacheDetails;
                var long: Value = j.num(0);
                for (details.array.items) |detail| {
                    if (detail == .null) return error.InvalidBedrockCacheDetail;
                    const contribution = if (std.mem.eql(u8, j.field(detail, "ttl") orelse "", "1h")) j.getNonNull(detail, "inputTokens") orelse j.num(0) else j.num(0);
                    long = try @import("../utils/cost.zig").addValues(a, long, contribution);
                }
                try accounting.set(a, &self.message.usage, "cacheWrite1h", long);
            } else j.markAbsent(&self.message.usage, "cacheWrite1h");
            const total = j.get(usage, "totalTokens") orelse j.num(0);
            try accounting.set(a, &self.message.usage, "totalTokens", if (j.truthy(total)) total else try accounting.sum(a, self.message.usage, &.{ "input", "output" }));
            _ = try @import("../utils/cost.zig").calculateCost(self.allocator, self.model, &self.message.usage);
        };
        return;
    }
    if (event == .object) {
        var it = event.object.iterator();
        while (it.next()) |entry| if (j.truthy(entry.value_ptr.*) and std.mem.endsWith(u8, entry.key_ptr.*, "Exception")) {
            self.message.errorMessage = if (j.field(entry.value_ptr.*, "name")) |name| blk: {
                self.bedrockErrorCode = name;
                break :blk try dataRetentionHint(self.allocator, try std.fmt.allocPrint(self.allocator, "{s}: {s}", .{ errorPrefix(name), j.field(entry.value_ptr.*, "message") orelse "Unknown" }));
            } else try dataRetentionHint(self.allocator, try @import("../json/value.zig").stringify(self.allocator, entry.value_ptr.*, .{}));
            return error.BedrockStreamException;
        };
    }
}
fn flushRedacted(self: *@import("reducer.zig").Machine, index: usize) !void {
    const state = &self.states.items[index];
    if (!state.redactedPending or self.blocks.items[index] != .thinking) return;
    const bytes = state.redactedBytes.items;
    const signature = try self.allocator.alloc(u8, std.base64.standard.Encoder.calcSize(bytes.len));
    _ = std.base64.standard.Encoder.encode(signature, bytes);
    self.blocks.items[index].thinking.thinkingSignature = signature;
    state.redactedPending = false;
    state.redactedBytes.clearRetainingCapacity();
}
pub fn finalizeBlocks(self: *@import("reducer.zig").Machine) !void {
    for (self.states.items, 0..) |_, index| try flushRedacted(self, index);
}
/// A retained value is allocated with `a`; blank and overlong values are dropped.
pub fn normalizeDiagnosticValue(a: std.mem.Allocator, value: ?[]const u8) !?[]const u8 {
    const text = try @import("../utils/unicode.zig").trimWhitespace(a, value orelse return null);
    errdefer a.free(text);
    if (text.len == 0 or try @import("../utils/unicode.zig").utf16Length(text) > 200) {
        a.free(text);
        return null;
    }
    return text;
}
pub fn appendFailureDiagnostic(self: *@import("reducer.zig").Machine) !void {
    var details = j.map(self.allocator);
    if (self.bedrockFailureStatus) |status| try j.put(self.allocator, &details, "status", j.int(status));
    if (try normalizeDiagnosticValue(self.allocator, self.bedrockErrorCode)) |code| if (std.mem.endsWith(u8, code, "Exception")) try j.put(self.allocator, &details, "errorCode", j.str(code));
    if (try normalizeDiagnosticValue(self.allocator, self.bedrockRequestId)) |request_id| try j.put(self.allocator, &details, "requestId", j.str(request_id));
    if (details.object.count() == 0) return;
    try @import("../utils/diagnostics.zig").appendAssistantMessageDiagnostic(self.allocator, &self.message, .{ .type = "bedrock_response_failure", .timestamp = @as(f64, @floatFromInt(std.Io.Clock.real.now(self.io).toMilliseconds())), .details = details });
}
fn dataRetentionHint(a: std.mem.Allocator, message: []const u8) ![]const u8 {
    if (std.ascii.findIgnoreCase(message, "data retention mode") == null) return message;
    return std.fmt.allocPrint(a, "{s} See https://docs.aws.amazon.com/bedrock/latest/userguide/data-retention.html for supported data retention modes.", .{message});
}
pub fn responseMetadata(self: *@import("reducer.zig").Machine, status: u16, response_headers: t.ProviderHeaders, failure: bool) !void {
    if (failure) self.bedrockFailureStatus = status;
    for (response_headers) |header| if (std.ascii.eqlIgnoreCase(header.name, "x-amzn-requestid") or std.ascii.eqlIgnoreCase(header.name, "x-amzn-request-id") or std.ascii.eqlIgnoreCase(header.name, "x-amz-request-id")) {
        if (try normalizeDiagnosticValue(self.allocator, header.value)) |value| {
            self.bedrockRequestId = value;
            break;
        }
    };
}
fn exceptionName(member: []const u8) ?[]const u8 {
    for ([_][]const u8{ "internalServerException", "modelStreamErrorException", "validationException", "throttlingException", "serviceUnavailableException" }, [_][]const u8{ "InternalServerException", "ModelStreamErrorException", "ValidationException", "ThrottlingException", "ServiceUnavailableException" }) |key, name| if (std.mem.eql(u8, key, member)) return name;
    return null;
}
fn errorPrefix(name: []const u8) []const u8 {
    for ([_][]const u8{ "InternalServerException", "ModelStreamErrorException", "ValidationException", "ThrottlingException", "ServiceUnavailableException" }, [_][]const u8{ "Internal server error", "Model stream error", "Validation error", "Throttling error", "Service unavailable" }) |code, prefix| if (std.mem.eql(u8, code, name)) return prefix;
    return name;
}
pub fn toWirePayload(a: std.mem.Allocator, payload: Value) !Value {
    if (payload != .object) return error.ExpectedJsonObject;
    return @import("bedrock_wire.zig").request(a, payload);
}
fn parseEventPayload(a: std.mem.Allocator, payload: []const u8) !Value {
    if (payload.len == 0) return j.map(a);
    const text = try std.fmt.allocPrint(a, "{f}", .{std.unicode.fmtUtf8(payload)});
    defer a.free(text);
    return @import("../json/value.zig").parseFromSliceLeaky(Value, a, text, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always });
}
pub fn frameEvent(self: *@import("reducer.zig").Machine, frame: @import("../transport/aws_event_stream.zig").Message) !?Value {
    switch (try frame.kind()) {
        .@"error" => {
            const name = (try frame.stringHeader(":error-code")) orelse return error.MissingBedrockErrorCode;
            self.bedrockErrorCode = try self.allocator.dupe(u8, name);
            const message = (try frame.stringHeader(":error-message")) orelse "UnknownError";
            self.message.errorMessage = try dataRetentionHint(self.allocator, try self.allocator.dupe(u8, if (message.len > 0) message else "UnknownError"));
            return error.BedrockStreamException;
        },
        .exception => {
            const member = (try frame.stringHeader(":exception-type")) orelse return error.MissingBedrockExceptionType;
            if (exceptionName(member)) |name| {
                const parsed = try @import("bedrock_wire.zig").event(self.allocator, member, try parseEventPayload(self.allocator, frame.payload));
                self.bedrockErrorCode = name;
                self.message.errorMessage = try dataRetentionHint(self.allocator, try std.fmt.allocPrint(self.allocator, "{s}: {s}", .{ errorPrefix(name), j.field(parsed, "message") orelse j.field(parsed, "Message") orelse "Unknown" }));
            } else {
                self.bedrockErrorCode = try self.allocator.dupe(u8, member);
                self.message.errorMessage = try dataRetentionHint(self.allocator, try std.fmt.allocPrint(self.allocator, "{f}", .{std.unicode.fmtUtf8(frame.payload)}));
            }
            return error.BedrockStreamException;
        },
        .event => {
            const member = (try frame.stringHeader(":event-type")) orelse return error.MissingBedrockEventType;
            const known = for ([_][]const u8{ "messageStart", "contentBlockStart", "contentBlockDelta", "contentBlockStop", "messageStop", "metadata", "internalServerException", "modelStreamErrorException", "validationException", "throttlingException", "serviceUnavailableException" }) |name| {
                if (std.mem.eql(u8, name, member)) break true;
            } else false;
            if (!known) return null;
            var parsed = try @import("bedrock_wire.zig").event(self.allocator, member, try parseEventPayload(self.allocator, frame.payload));
            if (exceptionName(member)) |name| {
                if (parsed != .object) return error.InvalidBedrockException;
                try j.put(self.allocator, &parsed, "name", j.str(name));
                try j.put(self.allocator, &parsed, "$fault", j.str(if (std.mem.eql(u8, name, "InternalServerException") or std.mem.eql(u8, name, "ServiceUnavailableException") or std.mem.eql(u8, name, "ModelStreamErrorException")) "server" else "client"));
                if (j.field(parsed, "message") == null) try j.put(self.allocator, &parsed, "message", j.str(j.field(parsed, "Message") orelse "Unknown"));
            }
            var event = j.map(self.allocator);
            try j.put(self.allocator, &event, try self.allocator.dupe(u8, member), parsed);
            return event;
        },
    }
}
fn normalizedErrorCode(raw: []const u8) []const u8 {
    var comma = std.mem.splitScalar(u8, raw, ',');
    var colon = std.mem.splitScalar(u8, comma.first(), ':');
    var namespace = std.mem.splitScalar(u8, colon.first(), '#');
    const first = namespace.first();
    return namespace.next() orelse first;
}
pub fn httpFailure(self: *@import("reducer.zig").Machine, status: u16, response_headers: t.ProviderHeaders, body: []const u8) ![]const u8 {
    try responseMetadata(self, status, response_headers, true);
    const text = try std.fmt.allocPrint(self.allocator, "{f}", .{std.unicode.fmtUtf8(body)});
    defer self.allocator.free(text);
    const parsed = @import("../json/value.zig").parseFromSliceLeaky(Value, self.allocator, text, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |err| blk: {
        if (err == error.OutOfMemory) return err;
        break :blk .null;
    };
    self.bedrockServerTime = j.field(parsed, "ServerTime");
    var code: ?[]const u8 = null;
    for (response_headers) |header| if (std.ascii.eqlIgnoreCase(header.name, "x-amzn-errortype")) {
        if (header.value) |value| code = normalizedErrorCode(value);
        break;
    };
    if (code == null and parsed == .object) {
        var entries = parsed.object.iterator();
        while (entries.next()) |entry| if (std.ascii.eqlIgnoreCase(entry.key_ptr.*, "code")) {
            const value = entry.value_ptr.*;
            if (value == .string) code = normalizedErrorCode(value.string) else if (j.number(value)) |number| code = normalizedErrorCode(try @import("../utils/numbers.zig").toString(self.allocator, number));
            break;
        };
        if (code == null) if (j.field(parsed, "__type")) |value| {
            code = normalizedErrorCode(value);
        };
    }
    if (code) |value| self.bedrockErrorCode = try self.allocator.dupe(u8, value);
    const core = if (j.field(parsed, "message") orelse j.field(parsed, "Message")) |message| message else try @import("../utils/error_body.zig").formatProviderError(self.allocator, .{ .status = @floatFromInt(status), .body = try @import("../utils/error_body.zig").truncateErrorText(self.allocator, try @import("../utils/unicode.zig").trimWhitespace(self.allocator, text), 4000), .message = "UnknownError", .messageCarriesBody = false }, null);
    return dataRetentionHint(self.allocator, try std.fmt.allocPrint(self.allocator, "{s}: {s}", .{ errorPrefix(code orelse "Unknown"), core }));
}

pub fn bedrockConverseStreamApi() @import("../models.zig").ApiImplementation {
    return @import("driver.zig").implementation("bedrock-converse-stream");
}
pub fn stream(operation: @import("../models.zig").OperationContext, model: t.Model, context: t.TranscriptContext, options: BedrockOptions) !*@import("../utils/event_stream.zig").AssistantMessageEventStream {
    if (!std.mem.eql(u8, model.api, "bedrock-converse-stream")) return error.MismatchedApi;
    var arena = std.heap.ArenaAllocator.init(operation.allocator);
    defer arena.deinit();
    return @import("driver.zig").stream(null, operation, model, context, try options.streamOptions(arena.allocator()));
}
pub fn streamSimple(operation: @import("../models.zig").OperationContext, model: t.Model, context: t.TranscriptContext, options: t.SimpleStreamOptions) !*@import("../utils/event_stream.zig").AssistantMessageEventStream {
    if (!std.mem.eql(u8, model.api, "bedrock-converse-stream")) return error.MismatchedApi;
    return @import("driver.zig").streamSimple(null, operation, model, context, options);
}

/// Native owner for the source module override. Retain it, its selected
/// implementation and returned LazyApi values through all joined streams.
pub const BedrockProviderModule = struct {
    mutex: std.Io.Mutex = .init,
    module: ?@import("../models.zig").ApiImplementation = null,
    fn load(ctx: ?*anyopaque, operation: @import("../models.zig").OperationContext) !@import("../models.zig").ApiImplementation {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        try self.mutex.lock(operation.io);
        defer self.mutex.unlock(operation.io);
        return self.module orelse bedrockConverseStreamApi();
    }
};
pub fn setBedrockProviderModule(owner: *BedrockProviderModule, io: std.Io, module: @import("../models.zig").ApiImplementation) !void {
    try owner.mutex.lock(io);
    defer owner.mutex.unlock(io);
    owner.module = module;
}
pub fn bedrockConverseStreamApiWithModule(owner: *BedrockProviderModule) @import("lazy.zig").LazyApi {
    return @import("lazy.zig").lazyApi("bedrock-converse-stream", .{ .context = owner, .call = BedrockProviderModule.load }, .{});
}
