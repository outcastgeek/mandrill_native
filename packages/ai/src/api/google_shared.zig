const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const c = @import("common.zig");
const tr = @import("../utils/transcript.zig");
const txt = @import("../utils/text.zig");
const constrained = @import("../schema/constrained.zig");
const Value = std.json.Value;
const sanitizeText = @import("../utils/unicode.zig").sanitizeSurrogates;
pub const GoogleToolCallIds = struct {
    mutex: std.Io.Mutex = .init,
    counters: [2]f64 = .{ 0, 0 },
    pub fn next(self: *@This(), io: std.Io, api: []const u8) !f64 {
        try self.mutex.lock(io);
        defer self.mutex.unlock(io);
        const index: usize = if (std.mem.eql(u8, api, "google-vertex")) 1 else 0;
        self.counters[index] += 1;
        return self.counters[index];
    }
};
pub const GoogleApiThinkingLevel = enum { THINKING_LEVEL_UNSPECIFIED, MINIMAL, LOW, MEDIUM, HIGH };
pub const ResolvedGoogleThinkingLevel = enum { minimal, low, medium, high };
pub const GoogleFunctionCallingMode = enum { AUTO, NONE, ANY, VALIDATED };
pub const GoogleOptions = struct {
    request: t.StreamOptions = .{},
    toolChoice: ?enum { auto, none, any } = null,
    thinking: ?struct { enabled: bool, budgetTokens: ?f64 = null, level: ?GoogleApiThinkingLevel = null } = null,
    pub fn streamOptions(self: @This(), a: std.mem.Allocator) !t.StreamOptions {
        var options = self.request;
        if (self.toolChoice) |choice| options.toolChoice = std.meta.stringToEnum(t.ToolChoice, @tagName(choice)).?;
        if (self.thinking) |thinking| {
            var extra = if (options.extra) |v| try j.clone(a, v) else j.map(a);
            try j.put(a, &extra, "thinking", try j.from(a, thinking));
            options.extra = extra;
        }
        return options;
    }
};
pub fn geminiMajor(id: []const u8) ?u32 {
    const prefix: []const u8 = if (std.ascii.startsWithIgnoreCase(id, "gemini-live-")) "gemini-live-" else if (std.ascii.startsWithIgnoreCase(id, "gemini-")) "gemini-" else return null;
    var end = prefix.len;
    while (end < id.len and std.ascii.isDigit(id[end])) end += 1;
    return std.fmt.parseInt(u32, id[prefix.len..end], 10) catch null;
}
pub fn requiresToolCallId(id: []const u8) bool {
    return std.mem.startsWith(u8, id, "claude-") or std.mem.startsWith(u8, id, "gpt-oss-") or (geminiMajor(id) orelse 0) >= 3;
}
pub fn supportsGoogleStrictToolSampling(id: []const u8) bool {
    return (geminiMajor(id) orelse 0) >= 3;
}
pub fn usesGoogleThinkingLevel(id: []const u8) bool {
    if (geminiMajor(id)) |major| {
        if (major >= 4) return true;
        if (major == 3 and (std.ascii.findIgnoreCase(id, "-pro") != null or std.ascii.findIgnoreCase(id, "-flash") != null)) return true;
    }
    return std.ascii.eqlIgnoreCase(id, "gemini-flash-latest") or std.ascii.eqlIgnoreCase(id, "gemini-flash-lite-latest") or std.ascii.findIgnoreCase(id, "gemma-4") != null or std.ascii.findIgnoreCase(id, "gemma4") != null;
}
pub fn resolveGoogleThinkingLevel(model: t.Model, level: t.ThinkingLevel) !ResolvedGoogleThinkingLevel {
    const mapped = if (model.thinkingLevelMap) |map| j.field(map, @tagName(level)) orelse @tagName(level) else @tagName(level);
    inline for (std.meta.tags(ResolvedGoogleThinkingLevel)) |resolved| if (std.ascii.eqlIgnoreCase(mapped, @tagName(resolved))) return resolved;
    return error.UnsupportedGoogleThinkingLevel;
}
pub fn resolveGoogleThinkingLevelWithError(a: std.mem.Allocator, model: t.Model, level: t.ThinkingLevel, error_message: ?*?[]const u8) !ResolvedGoogleThinkingLevel {
    return resolveGoogleThinkingLevel(model, level) catch |err| {
        if (error_message) |slot| {
            const mapped = if (model.thinkingLevelMap) |map| j.get(map, @tagName(level)) else null;
            const detail = if (mapped) |value| j.text(value) orelse try j.stringify(a, value, .{}) else "undefined";
            slot.* = try std.fmt.allocPrint(a, "Unsupported Google thinking level mapping for {s}/{s}: {s} -> {s}", .{ model.provider, model.id, @tagName(level), detail });
        }
        return err;
    };
}
pub fn toGoogleThinkingLevel(level: ResolvedGoogleThinkingLevel) GoogleApiThinkingLevel {
    return switch (level) {
        .minimal => .MINIMAL,
        .low => .LOW,
        .medium => .MEDIUM,
        .high => .HIGH,
    };
}
// The SDK enum uses the same wire values; there is no JavaScript SDK object in
// the native adapter, so this conversion preserves its typed wire enum.
pub fn toGoogleSdkThinkingLevel(level: GoogleApiThinkingLevel) GoogleApiThinkingLevel {
    return level;
}
pub fn getDisabledGoogleThinkingConfig(a: std.mem.Allocator, model: t.Model) !Value {
    return getDisabledGoogleThinkingConfigWithError(a, model, null);
}
pub fn getDisabledGoogleThinkingConfigWithError(a: std.mem.Allocator, model: t.Model, error_message: ?*?[]const u8) !Value {
    if (!usesGoogleThinkingLevel(model.id)) return j.from(a, .{ .thinkingBudget = 0 });
    const fallback = @import("../utils/thinking.zig").clampThinkingLevel(model, .off);
    return if (fallback == .off) j.from(a, .{ .thinkingBudget = 0 }) else j.from(a, .{ .thinkingLevel = toGoogleThinkingLevel(try resolveGoogleThinkingLevelWithError(a, model, fallback, error_message)) });
}
pub fn mapToolChoice(choice: []const u8) GoogleFunctionCallingMode {
    if (std.mem.eql(u8, choice, "none")) return .NONE;
    if (std.mem.eql(u8, choice, "any")) return .ANY;
    return .AUTO;
}
pub fn resolveGoogleFunctionCallingMode(a: std.mem.Allocator, tools: []const t.Tool, choice: ?t.ToolChoice, supports_strict: bool) !?GoogleFunctionCallingMode {
    return resolveGoogleFunctionCallingModeWithError(a, tools, choice, supports_strict, null);
}
pub fn resolveGoogleFunctionCallingModeWithError(a: std.mem.Allocator, tools: []const t.Tool, choice: ?t.ToolChoice, supports_strict: bool, error_message: ?*?[]const u8) !?GoogleFunctionCallingMode {
    var strict = false;
    for (tools) |tool| if ((try constrained.resolveJsonSchemaStrictSamplingWithError(a, tool, supports_strict, null, error_message)) orelse false) {
        strict = true;
        break;
    };
    if (choice == .none) return .NONE;
    if (choice == .any) return .ANY;
    if (strict) return .VALIDATED;
    return if (choice) |value| mapToolChoice(@tagName(value)) else null;
}
pub fn isThinkingPart(part: Value) bool {
    return j.boolean(j.get(part, "thought") orelse .null) orelse false;
}
pub fn retainThoughtSignature(existing: ?[]const u8, incoming: ?[]const u8) ?[]const u8 {
    if (incoming) |signature| if (signature.len > 0) return signature;
    return existing;
}
pub fn isValidThoughtSignature(signature: ?[]const u8) bool {
    const sig = signature orelse return false;
    if (sig.len == 0 or sig.len % 4 != 0) return false;
    // The pinned helper validates the alphabet/padding shape without decoding
    // opaque signature bytes or requiring canonical zero padding bits.
    const body = std.mem.trimEnd(u8, sig, "=");
    return body.len > 0 and sig.len - body.len <= 2 and std.mem.findNone(u8, body, &std.base64.standard_alphabet_chars) == null;
}
pub fn resolveThoughtSignature(_: std.mem.Allocator, same: bool, signature: ?[]const u8) !?[]const u8 {
    return if (same and isValidThoughtSignature(signature)) signature else null;
}
fn normalizeId(a: std.mem.Allocator, id: []const u8, model: t.Model, _: t.AssistantMessage) ![]const u8 {
    if (!requiresToolCallId(model.id)) return id;
    // The source replacement and length limit operate on UTF-16 units. Let
    // the standard Unicode codec decode them before normalizing this ID.
    const units = try std.unicode.wtf8ToWtf16LeAlloc(a, id);
    defer a.free(units);
    const out = try a.alloc(u8, @min(units.len, 64));
    for (out, units[0..out.len]) |*byte, unit| byte.* = if (unit <= 0x7f and (std.ascii.isAlphanumeric(@intCast(unit)) or unit == '_' or unit == '-')) @intCast(unit) else '_';
    return out;
}
fn imagePart(a: std.mem.Allocator, image: t.ImageContent) !Value {
    return j.from(a, .{ .inlineData = .{ .mimeType = image.mimeType, .data = image.data } });
}
pub fn convertMessages(a: std.mem.Allocator, io: std.Io, model: t.Model, context: t.TranscriptContext) !Value {
    const collapsed = try tr.collapseSystemMessages(a, context);
    const messages = try @import("transform_messages.zig").transformMessages(a, io, tr.withoutInitialSystemMessage(collapsed.messages), model, normalizeId);
    var out = j.array(a);
    for (messages) |message| switch (message) {
        .user => |user| {
            if (user.content == .missing or user.content == .nullContent) return error.InvalidMessageContent;
            var parts = j.array(a);
            if (user.content == .string) try parts.array.append(try j.from(a, .{ .text = try sanitizeText(a, user.content.string) })) else for (user.content.blocks) |block| switch (block) {
                .text => |v| try parts.array.append(try j.from(a, .{ .text = try sanitizeText(a, v.text) })),
                .image => |v| try parts.array.append(try imagePart(a, v)),
                else => {},
            };
            if (parts.array.items.len == 0) continue;
            var item = try j.from(a, .{ .role = "user" });
            try j.put(a, &item, "parts", parts);
            try out.array.append(item);
        },
        .assistant => |assistant| {
            const same = std.mem.eql(u8, assistant.provider, model.provider) and std.mem.eql(u8, assistant.model, model.id);
            var parts = j.array(a);
            for (assistant.content) |block| {
                var part: Value = undefined;
                var signature: ?[]const u8 = null;
                switch (block) {
                    .text => |v| {
                        signature = try resolveThoughtSignature(a, same, v.textSignature);
                        if (try @import("../utils/unicode.zig").isBlank(v.text) and signature == null) continue;
                        part = try j.from(a, .{ .text = try sanitizeText(a, v.text) });
                    },
                    .thinking => |v| {
                        signature = try resolveThoughtSignature(a, same, v.thinkingSignature);
                        if (try @import("../utils/unicode.zig").isBlank(v.thinking) and signature == null) continue;
                        part = try j.from(a, .{ .text = try sanitizeText(a, v.thinking) });
                        if (same) try j.put(a, &part, "thought", .{ .bool = true });
                    },
                    .toolCall => |call| {
                        signature = try resolveThoughtSignature(a, same, call.thoughtSignature);
                        var function = try j.from(a, .{ .name = call.name });
                        try j.put(a, &function, "args", if (call.arguments == .null) j.map(a) else call.arguments);
                        if (requiresToolCallId(model.id)) try j.put(a, &function, "id", j.str(call.id));
                        part = j.map(a);
                        try j.put(a, &part, "functionCall", function);
                    },
                    else => continue,
                }
                if (signature) |sig| try j.put(a, &part, "thoughtSignature", j.str(sig));
                try parts.array.append(part);
            }
            if (parts.array.items.len == 0) continue;
            var item = try j.from(a, .{ .role = "model" });
            try j.put(a, &item, "parts", parts);
            try out.array.append(item);
        },
        .toolResult => |result| {
            const text = try txt.contentText(a, .{ .blocks = result.content }, "\n");
            var image_parts = j.array(a);
            const vision = for (model.input) |modality| {
                if (modality == .image) break true;
            } else false;
            if (vision) for (result.content) |block| if (block == .image) {
                try image_parts.array.append(try imagePart(a, block.image));
            };
            const multimodal = if (geminiMajor(model.id)) |major| major >= 3 else true;
            var response = j.map(a);
            const has_text = !try @import("../utils/unicode.zig").isBlank(text);
            try j.put(a, &response, if (result.isError) "error" else "output", j.str(if (has_text) try sanitizeText(a, text) else if (image_parts.array.items.len > 0) "(see attached image)" else ""));
            var function = try j.from(a, .{ .name = result.toolName });
            try j.put(a, &function, "response", response);
            if (requiresToolCallId(model.id)) try j.put(a, &function, "id", j.str(result.toolCallId));
            if (multimodal and image_parts.array.items.len > 0) try j.put(a, &function, "parts", image_parts);
            var part = j.map(a);
            try j.put(a, &part, "functionResponse", function);
            var merged = false;
            if (out.array.items.len > 0) {
                const last = &out.array.items[out.array.items.len - 1];
                if (std.mem.eql(u8, j.field(last.*, "role") orelse "", "user")) {
                    if (last.object.getPtr("parts")) |parts| {
                        const has_response = for (parts.array.items) |p| {
                            if (j.get(p, "functionResponse") != null) break true;
                        } else false;
                        if (has_response) {
                            try parts.array.append(part);
                            merged = true;
                        }
                    }
                }
            }
            if (!merged) {
                var parts = j.array(a);
                try parts.array.append(part);
                var item = try j.from(a, .{ .role = "user" });
                try j.put(a, &item, "parts", parts);
                try out.array.append(item);
            }
            if (!multimodal and image_parts.array.items.len > 0) {
                var parts = j.array(a);
                try parts.array.append(try j.from(a, .{ .text = "Tool result image:" }));
                try parts.array.appendSlice(image_parts.array.items);
                var item = try j.from(a, .{ .role = "user" });
                try j.put(a, &item, "parts", parts);
                try out.array.append(item);
            }
        },
        .system => {},
    };
    return out;
}
fn sanitizeSchema(a: std.mem.Allocator, schema: Value) anyerror!Value {
    if (schema != .object) return schema;
    var out = j.map(a);
    var it = schema.object.iterator();
    while (it.next()) |entry| {
        const skip = for ([_][]const u8{ "$schema", "$id", "$anchor", "$dynamicAnchor", "$vocabulary", "$comment", "$defs", "definitions" }) |key| {
            if (std.mem.eql(u8, key, entry.key_ptr.*)) break true;
        } else false;
        if (!skip) try j.put(a, &out, entry.key_ptr.*, try sanitizeSchema(a, entry.value_ptr.*));
    }
    return out;
}
pub fn convertTools(a: std.mem.Allocator, tools: []const t.Tool, use_parameters: bool, supports_strict: bool) !Value {
    return convertToolsWithError(a, tools, use_parameters, supports_strict, null);
}
pub fn convertToolsWithError(a: std.mem.Allocator, tools: []const t.Tool, use_parameters: bool, supports_strict: bool, error_message: ?*?[]const u8) !Value {
    if (tools.len == 0) return .null;
    var declarations = j.array(a);
    for (tools) |tool| {
        const strict = try constrained.resolveJsonSchemaStrictSamplingWithError(a, tool, supports_strict, null, error_message);
        const parameters = try constrained.getJsonSchemaToolParametersWithError(a, tool, strict, error_message);
        var declaration = try j.from(a, .{ .name = tool.name, .description = tool.description });
        try j.put(a, &declaration, if (use_parameters) "parameters" else "parametersJsonSchema", if (use_parameters) try sanitizeSchema(a, parameters) else parameters);
        try declarations.array.append(declaration);
    }
    var group = j.map(a);
    try j.put(a, &group, "functionDeclarations", declarations);
    var out = j.array(a);
    try out.array.append(group);
    return out;
}
pub fn buildPayload(a: std.mem.Allocator, io: std.Io, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !Value {
    const normalized = try tr.collapseSystemMessages(a, context);
    var payload = j.map(a);
    try j.put(a, &payload, "contents", try convertMessages(a, io, model, normalized));
    const initial = tr.getInitialSystemMessage(normalized.messages);
    if (initial) |s| {
        const text = try txt.getSystemMessageText(a, s);
        if (text.len > 0) try j.put(a, &payload, "systemInstruction", try j.from(a, .{ .parts = .{.{ .text = try sanitizeText(a, text) }} }));
    }
    var config = j.map(a);
    if (try j.fieldValue(a, options, "temperature")) |value| try j.put(a, &config, "temperature", value);
    if (try j.fieldValue(a, options, "maxTokens")) |value| try j.put(a, &config, "maxOutputTokens", value);
    const tools = try tr.getCurrentTools(a, normalized.messages);
    const supports = supportsGoogleStrictToolSampling(model.id);
    if (tools.len > 0) {
        try j.put(a, &payload, "tools", try convertToolsWithError(a, tools, false, supports, options.errorMessage));
        if (try resolveGoogleFunctionCallingModeWithError(a, tools, options.toolChoice, supports, options.errorMessage)) |mode| {
            try j.put(a, &payload, "toolConfig", try j.from(a, .{ .functionCallingConfig = .{ .mode = mode } }));
        }
    }
    if (model.reasoning and c.extra(options, "thinking") != null and j.truthy(c.extra(options, "thinking").?)) {
        const thinking_options = c.extra(options, "thinking").?;
        const enabled = j.truthy(j.get(thinking_options, "enabled") orelse .null);
        var thinking = if (enabled) try j.from(a, .{ .includeThoughts = true }) else try getDisabledGoogleThinkingConfigWithError(a, model, options.errorMessage);
        if (enabled) {
            if (j.get(thinking_options, "level")) |raw_level| {
                if (j.text(raw_level)) |level| if (std.meta.stringToEnum(GoogleApiThinkingLevel, level)) |known| {
                    try j.put(a, &thinking, "thinkingLevel", try j.from(a, known));
                };
            } else if (j.get(thinking_options, "budgetTokens")) |budget| try j.put(a, &thinking, "thinkingBudget", budget);
        }
        try j.put(a, &config, "thinkingConfig", thinking);
    }
    if (config.object.count() > 0) try j.put(a, &payload, "generationConfig", config);
    return payload;
}
pub fn mapStopReasonString(reason: []const u8) t.StopReason {
    if (std.mem.eql(u8, reason, "STOP")) return .stop;
    if (std.mem.eql(u8, reason, "MAX_TOKENS")) return .length;
    return .@"error";
}
pub fn mapStopReason(reason: []const u8) !t.StopReason {
    const FinishReason = enum { STOP, MAX_TOKENS, BLOCKLIST, PROHIBITED_CONTENT, SPII, SAFETY, IMAGE_SAFETY, IMAGE_PROHIBITED_CONTENT, IMAGE_RECITATION, IMAGE_OTHER, RECITATION, FINISH_REASON_UNSPECIFIED, OTHER, LANGUAGE, MALFORMED_FUNCTION_CALL, UNEXPECTED_TOOL_CALL, TOO_MANY_TOOL_CALLS, NO_IMAGE };
    const known = std.meta.stringToEnum(FinishReason, reason) orelse return error.UnhandledGoogleStopReason;
    return switch (known) {
        .STOP => .stop,
        .MAX_TOKENS => .length,
        else => .@"error",
    };
}
pub fn buildSdkPayload(a: std.mem.Allocator, io: std.Io, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !Value {
    const wire = try buildPayload(a, io, model, context, options);
    var params = try j.from(a, .{ .model = model.id });
    try j.put(a, &params, "contents", j.get(wire, "contents").?);
    var config = j.map(a);
    const generation = j.get(wire, "generationConfig") orelse .null;
    inline for (.{ "temperature", "maxOutputTokens" }) |name| if (j.get(generation, name)) |value| try j.put(a, &config, name, value);
    if (j.get(wire, "systemInstruction")) |system| {
        const parts = c.array(system, "parts");
        if (parts.len > 0) try j.put(a, &config, "systemInstruction", j.str(j.field(parts[0], "text") orelse ""));
    }
    if (j.get(wire, "tools")) |tools| try j.put(a, &config, "tools", tools);
    if (j.get(wire, "toolConfig")) |tool_config| try j.put(a, &config, "toolConfig", tool_config);
    if (j.get(generation, "thinkingConfig")) |value| try j.put(a, &config, "thinkingConfig", value);
    try j.put(a, &params, "config", config);
    return params;
}
pub fn toWirePayload(a: std.mem.Allocator, params: Value) !Value {
    return toWirePayloadForApi(a, false, params);
}
pub fn httpOptions(params: Value) Value {
    return j.get(j.get(params, "config") orelse .null, "httpOptions") orelse .null;
}
pub fn patchHttpOption(a: std.mem.Allocator, base: Value, options: Value, name: []const u8) !Value {
    const override = j.get(options, name) orelse return j.clone(a, base);
    return switch (override) {
        .object, .array, .null => value: {
            var result = try j.objectSpread(a, base);
            const fields = try j.objectSpread(a, override);
            for (try j.objectEntries(a, fields.object)) |entry| try j.put(a, &result, entry.key, entry.node.*);
            break :value result;
        },
        else => try j.clone(a, override),
    };
}
pub fn httpTimeoutMs(a: std.mem.Allocator, params: Value) !?f64 {
    const raw = j.get(httpOptions(params), "timeout") orelse return null;
    if (!j.truthy(raw)) return null;
    const timeout = try @import("../utils/numbers.zig").coerceValue(a, raw);
    if (std.math.isNan(timeout) or timeout <= 0) return null;
    return timeout;
}
/// Preserve the SDK's exact-key object stage before Fetch normalizes names.
/// Runtime wire metadata uses the existing native unknown-host adaptation.
pub fn clientHeaders(a: std.mem.Allocator, model: t.Model, options: t.StreamOptions) !t.ProviderHeaders {
    const h = @import("../utils/headers.zig");
    const sdk = "google-genai-sdk/2.21.0 gl-node/unknown";
    const supplied = try h.providerHeadersToRecord(a, &.{try h.spread(a, &.{ &.{.{ .name = "User-Agent", .value = try @import("../utils/pi_user_agent.zig").getPiUserAgent(a) }}, try h.fromValue(a, model.headers), options.headers })});
    return h.spread(a, &.{ &.{ .{ .name = "User-Agent", .value = sdk }, .{ .name = "x-goog-api-client", .value = sdk }, .{ .name = "Content-Type", .value = "application/json" } }, try h.fromValue(a, supplied) });
}
pub fn requestHeaders(a: std.mem.Allocator, params: Value, original: t.ProviderHeaders, api_key: ?[]const u8) !t.ProviderHeaders {
    return requestHeadersWithError(a, params, original, api_key, null);
}
pub fn requestHeadersWithError(a: std.mem.Allocator, params: Value, original: t.ProviderHeaders, api_key: ?[]const u8, error_message: ?*?[]const u8) !t.ProviderHeaders {
    const h = @import("../utils/headers.zig");
    const config = httpOptions(params);
    var values = j.map(a);
    for (original) |header| try j.put(a, &values, header.name, if (header.value) |value| j.str(value) else .null);
    if (j.get(config, "headers")) |override| switch (override) {
        .object, .array, .null => {
            const fields = try j.objectSpread(a, override);
            for (try j.objectEntries(a, fields.object)) |entry| try j.put(a, &values, entry.key, entry.node.*);
        },
        else => values = try j.objectSpread(a, override),
    };
    var result: std.ArrayList(t.ProviderHeader) = .empty;
    for (try j.objectEntries(a, values.object)) |entry| try h.appendFetch(a, &result, entry.key, try @import("../utils/diagnostics.zig").formatThrownValue(a, .{ .value = entry.node.* }), error_message);
    if (try httpTimeoutMs(a, params)) |timeout| {
        const seconds = try @import("../utils/numbers.zig").toString(a, @ceil(timeout / 1000));
        try h.appendFetch(a, &result, "X-Server-Timeout", seconds, error_message);
    }
    if (api_key) |key| {
        if (std.mem.startsWith(u8, key, "auth_tokens/")) return error.GoogleEphemeralTokenUnsupported;
        if (h.get(result.items, "x-goog-api-key") == null) try h.appendFetch(a, &result, "x-goog-api-key", key, error_message);
    }
    std.mem.sort(t.ProviderHeader, result.items, {}, struct {
        fn less(_: void, left: t.ProviderHeader, right: t.ProviderHeader) bool {
            return std.mem.order(u8, left.name, right.name) == .lt;
        }
    }.less);
    return result.toOwnedSlice(a);
}
pub fn toWirePayloadForApi(a: std.mem.Allocator, vertex: bool, params: Value) !Value {
    return toWirePayloadWithResources(a, vertex, params, .{});
}
pub fn toWirePayloadWithError(a: std.mem.Allocator, vertex: bool, params: Value, resources: @import("google_wire.zig").Resources, error_message: ?*?[]const u8) !Value {
    return toWirePayloadWithResources(a, vertex, params, resources) catch |err| {
        if (error_message) |slot| {
            const field: ?[]const u8 = switch (err) {
                error.GoogleDeveloperRoutingConfigUnsupported => "routingConfig",
                error.GoogleDeveloperModelSelectionConfigUnsupported => "modelSelectionConfig",
                error.GoogleDeveloperLabelsUnsupported => "labels",
                error.GoogleDeveloperAudioTimestampUnsupported => "audioTimestamp",
                error.GoogleDeveloperModelArmorConfigUnsupported => "modelArmorConfig",
                error.GoogleDeveloperSafetyMethodUnsupported => "method",
                else => null,
            };
            if (field) |name| slot.* = try std.fmt.allocPrint(a, "{s} parameter is only supported in Gemini Enterprise Agent Platform mode, not in Gemini Developer API mode.", .{name}) else if (err == error.GoogleVertexEnhancedCivicAnswersUnsupported) slot.* = "enableEnhancedCivicAnswers parameter is only supported in Gemini Developer API mode, not in Gemini Enterprise Agent Platform mode." else if (@import("google_wire.zig").unsupportedField(err)) |name| {
                slot.* = try std.fmt.allocPrint(a, "{s} parameter is only supported in {s} mode, not in {s} mode.", .{ name, if (vertex) "Gemini Developer API" else "Gemini Enterprise Agent Platform", if (vertex) "Gemini Enterprise Agent Platform" else "Gemini Developer API" });
            } else {
                const message: ?[]const u8 = switch (err) {
                    error.InvalidGoogleModel => "invalid model parameter",
                    error.GoogleModelRequired => "model is required and must be a string",
                    error.GoogleSchemaTypeAndAnyOf => "type and anyOf cannot be both populated.",
                    error.GoogleSchemaOnlyNull => "type: null can not be the only possible type for the field.",
                    error.InvalidGoogleCachedContentName => "name must be a string",
                    error.MissingGooglePart => "PartUnion is required",
                    error.UnsupportedGoogleBooleanPart => "Unsupported part type: boolean",
                    error.UnsupportedGoogleNumberPart => "Unsupported part type: number",
                    error.MissingGoogleContent => "ContentUnion is required",
                    error.MissingGoogleParts => "PartListUnion is required",
                    error.MissingGoogleContents => "contents are required",
                    error.InvalidGoogleTools => "tools is required and must be an array of Tools",
                    error.GoogleFunctionPartRequiresContent => "To specify functionCall or functionResponse parts, please wrap them in a Content object, specifying the role for them",
                    error.GoogleFunctionPartListRequiresContent => "To specify functionCall or functionResponse parts, please wrap them, and any other parts, in Content objects as appropriate, specifying the role for them",
                    error.MixedGoogleContentAndParts => "Mixing Content and Parts is not supported, please group the parts into a the appropriate Content objects and specify the roles for them",
                    error.UnsupportedGoogleBooleanSpeechConfig => "Unsupported speechConfig type: boolean",
                    error.UnsupportedGoogleNumberSpeechConfig => "Unsupported speechConfig type: number",
                    else => null,
                };
                if (message) |text| slot.* = text;
            }
        }
        return err;
    };
}
pub fn validateRequestModel(params: Value) !?[]const u8 {
    const raw = j.getNonNull(params, "model") orelse return null;
    const model = j.text(raw) orelse return error.GoogleModelRequired;
    if (model.len == 0) return error.GoogleModelRequired;
    if (std.mem.find(u8, model, "..") != null or std.mem.findAny(u8, model, "?&") != null) return error.InvalidGoogleModel;
    return model;
}
pub fn toWirePayloadWithResources(a: std.mem.Allocator, vertex: bool, params: Value, resources: @import("google_wire.zig").Resources) !Value {
    const sdk = @import("google_wire.zig");
    _ = try validateRequestModel(params);
    var wire = j.map(a);
    if (j.getNonNull(params, "contents")) |contents| try j.put(a, &wire, "contents", try sdk.contents(a, vertex, contents));
    var generation = j.map(a);
    if (j.get(params, "config")) |config| if (config != .null) {
        // @google/genai 2.21.0's request converters copy declared fields only.
        // httpOptions and abortSignal are SDK controls, not generation fields.
        if (j.get(config, "serviceTier")) |value| if (value != .null) try j.put(a, &wire, "serviceTier", value);
        if (j.get(config, "systemInstruction")) |value| if (value != .null) try j.put(a, &wire, "systemInstruction", try sdk.content(a, vertex, value));
        for ([_][]const u8{ "temperature", "topP", "topK", "candidateCount", "maxOutputTokens", "stopSequences", "responseLogprobs", "logprobs", "presencePenalty", "frequencyPenalty", "seed", "responseMimeType" }) |key| if (j.get(config, key)) |value| if (value != .null) try j.put(a, &generation, key, value);
        const response_schema = j.get(config, "responseSchema");
        const move_response_schema = if (response_schema) |value| j.truthy(value) and j.get(value, "$schema") != null and !j.truthy(j.get(config, "responseJsonSchema") orelse .null) else false;
        if (response_schema) |value| if (value != .null and !move_response_schema) try j.put(a, &generation, "responseSchema", try sdk.schema(a, value));
        if (move_response_schema) try j.put(a, &generation, "responseJsonSchema", response_schema.?) else if (j.get(config, "responseJsonSchema")) |value| if (value != .null) try j.put(a, &generation, "responseJsonSchema", value);
        if (vertex) {
            if (j.get(config, "routingConfig")) |value| if (value != .null) try j.put(a, &generation, "routingConfig", value);
            if (j.get(config, "modelSelectionConfig")) |value| if (value != .null) try j.put(a, &generation, "modelConfig", value);
        } else {
            if (j.get(config, "routingConfig") != null) return error.GoogleDeveloperRoutingConfigUnsupported;
            if (j.get(config, "modelSelectionConfig") != null) return error.GoogleDeveloperModelSelectionConfigUnsupported;
        }
        if (j.get(config, "safetySettings")) |settings| if (settings != .null) {
            var values = settings;
            if (!vertex and settings == .array) {
                values = j.array(a);
                for (settings.array.items) |setting| {
                    if (j.get(setting, "method") != null) return error.GoogleDeveloperSafetyMethodUnsupported;
                    try values.array.append(try sdk.fields(a, setting, &.{ "category", "threshold" }));
                }
            }
            try j.put(a, &wire, "safetySettings", values);
        };
        if (j.get(config, "tools")) |value| if (value != .null) try j.put(a, &wire, "tools", try sdk.tools(a, vertex, value));
        if (j.get(config, "toolConfig")) |value| if (value != .null) try j.put(a, &wire, "toolConfig", try sdk.toolConfig(a, vertex, value));
        if (vertex) {
            if (j.get(config, "labels")) |value| if (value != .null) try j.put(a, &wire, "labels", value);
        } else if (j.get(config, "labels") != null) return error.GoogleDeveloperLabelsUnsupported;
        if (j.get(config, "cachedContent")) |value| if (value != .null) try j.put(a, &wire, "cachedContent", try sdk.cachedContentName(a, vertex, value, resources));
        for ([_][]const u8{ "responseModalities", "mediaResolution" }) |key| if (j.get(config, key)) |value| if (value != .null) try j.put(a, &generation, key, value);
        if (j.get(config, "speechConfig")) |value| if (value != .null) try j.put(a, &generation, "speechConfig", try sdk.speechConfigForApi(a, vertex, value));
        if (vertex) {
            if (j.get(config, "audioTimestamp")) |value| if (value != .null) try j.put(a, &generation, "audioTimestamp", value);
        } else if (j.get(config, "audioTimestamp") != null) return error.GoogleDeveloperAudioTimestampUnsupported;
        for ([_][]const u8{ "thinkingConfig", "audioTranscriptionConfig" }) |key| if (j.get(config, key)) |value| if (value != .null) try j.put(a, &generation, key, value);
        if (j.get(config, "imageConfig")) |value| if (value != .null) try j.put(a, &generation, "imageConfig", try sdk.imageConfig(a, vertex, value));
        if (vertex) {
            if (j.get(config, "enableEnhancedCivicAnswers") != null) return error.GoogleVertexEnhancedCivicAnswersUnsupported;
            if (j.get(config, "modelArmorConfig")) |value| if (value != .null) try j.put(a, &wire, "modelArmorConfig", value);
        } else {
            if (j.get(config, "enableEnhancedCivicAnswers")) |value| if (value != .null) try j.put(a, &generation, "enableEnhancedCivicAnswers", value);
            if (j.get(config, "modelArmorConfig") != null) return error.GoogleDeveloperModelArmorConfigUnsupported;
        }
        try j.put(a, &wire, "generationConfig", generation);
        if (j.get(j.get(config, "httpOptions") orelse .null, "extraBody")) |extra| if (j.truthy(extra)) {
            wire = try sdk.mergeExtraBody(a, wire, extra);
        };
    };
    return wire;
}

/// The native ProviderError interface explicitly supplies the source status/
/// headers shape, including absent headers, so Google uses the common retry.
pub const retryGoogleRequest = @import("../utils/provider_retry.zig").retryProviderRequest;
