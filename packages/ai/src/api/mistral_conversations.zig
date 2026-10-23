const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const c = @import("common.zig");
const tr = @import("../utils/transcript.zig");
const txt = @import("../utils/text.zig");
const constrained = @import("../schema/constrained.zig");
const Value = std.json.Value;
const sanitizeText = @import("../utils/unicode.zig").sanitizeSurrogates;
pub const MistralOptions = @import("options.zig").MistralOptions;
fn checkHeaderByteString(a: std.mem.Allocator, text: []const u8, error_message: ?*?[]const u8) !void {
    const units = try std.unicode.wtf8ToWtf16LeAlloc(a, text);
    defer a.free(units);
    for (units, 0..) |unit, index| if (std.mem.littleToNative(u16, unit) > 255) {
        if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Header string contains a character outside ByteString at index {d}", .{index});
        return error.InvalidMistralHeaderByteString;
    };
}
fn applyHeader(a: std.mem.Allocator, list: *std.ArrayList(t.ProviderHeader), header: t.ProviderHeader, method: []const u8, error_message: ?*?[]const u8) !void {
    try checkHeaderByteString(a, header.name, error_message);
    if (header.value) |value| try checkHeaderByteString(a, value, error_message);
    const value = if (header.value) |raw| std.mem.trim(u8, raw, "\t\r\n ") else null;
    const valid_name = try @import("../regex/regexp.zig").matches(a, null, "^[!#$%&'*+.^_`|~0-9A-Za-z-]+(?![\\s\\S])", header.name, .{});
    if (!valid_name or (if (value) |v| std.mem.findAny(u8, v, "\x00\r\n") != null else false)) {
        if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Headers.{s}: \"{s}\" is an invalid header {s}.", .{ method, if (!valid_name) header.name else value.?, if (!valid_name) "name" else "value" });
        return error.InvalidMistralHeader;
    }
    var index: usize = 0;
    while (index < list.items.len) {
        if (std.ascii.eqlIgnoreCase(list.items[index].name, header.name)) {
            _ = list.orderedRemove(index);
        } else index += 1;
    }
    if (value) |v| try list.append(a, .{ .name = try std.ascii.allocLowerString(a, header.name), .value = v });
}
/// Fetch Headers semantics at the adapter boundary; values remain native UTF-8
/// strings. Physical non-ASCII ByteString encoding remains a transport gap.
pub fn requestHeaders(a: std.mem.Allocator, model: t.Model, options: t.StreamOptions) !t.ProviderHeaders {
    const h = @import("../utils/headers.zig");
    var list: std.ArrayList(t.ProviderHeader) = .empty;
    errdefer list.deinit(a);
    const defaults = [_]t.ProviderHeader{
        .{ .name = "User-Agent", .value = try @import("../utils/pi_user_agent.zig").getPiUserAgent(a) },
        .{ .name = "accept", .value = "text/event-stream" },
        .{ .name = "authorization", .value = try std.fmt.allocPrint(a, "Bearer {s}", .{options.apiKey orelse ""}) },
        .{ .name = "content-type", .value = "application/json" },
    };
    for (defaults) |header| try applyHeader(a, &list, header, "append", options.errorMessage);
    const model_headers = try h.fromValue(a, model.headers);
    defer a.free(model_headers);
    var explicit = false;
    for ([_]t.ProviderHeaders{ model_headers, options.headers }) |source| for (source) |header| {
        if (std.ascii.eqlIgnoreCase(header.name, "x-affinity")) explicit = true;
        try applyHeader(a, &list, header, if (header.value == null) "delete" else "set", options.errorMessage);
    };
    if (!explicit and c.cacheRetention(options) != .none) if (options.sessionId) |session| if (session.len > 0) {
        try applyHeader(a, &list, .{ .name = "x-affinity", .value = session }, "set", options.errorMessage);
    };
    std.mem.sort(t.ProviderHeader, list.items, {}, struct {
        fn less(_: void, left: t.ProviderHeader, right: t.ProviderHeader) bool {
            return std.mem.order(u8, left.name, right.name) == .lt;
        }
    }.less);
    return list.toOwnedSlice(a);
}
pub fn httpFailure(a: std.mem.Allocator, status: u16, status_text: []const u8, body: []const u8) ![]const u8 {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    const decoded = try std.fmt.allocPrint(scratch, "{f}", .{std.unicode.fmtUtf8(body)});
    const trimmed = try @import("../utils/unicode.zig").trimWhitespace(scratch, decoded);
    const message = if (trimmed.len > 0) try @import("../utils/error_body.zig").truncateErrorText(scratch, trimmed, 4000) else if (status_text.len > 0) status_text else try std.fmt.allocPrint(scratch, "Request failed with status {d}", .{status});
    return std.fmt.allocPrint(a, "Mistral API error ({d}): {s}", .{ status, message });
}
/// Header-only AbortSignal.timeout validation, before wire serialization.
pub fn validateHeaderTimeout(a: std.mem.Allocator, options: t.StreamOptions, error_message: ?*?[]const u8) !void {
    const timeout = options.timeoutMs orelse 60_000;
    const integer = std.math.isFinite(timeout) and @floor(timeout) == timeout;
    if (integer and timeout >= 0 and timeout <= std.math.maxInt(u32)) return;
    if (error_message) |slot| {
        const text = try @import("../utils/numbers.zig").toString(a, timeout);
        var display = std.Io.Writer.Allocating.init(a);
        defer display.deinit();
        if (integer and @abs(timeout) > 4294967296) {
            // Node displays large integer diagnostics in three-character
            // groups, including the existing exponent spelling.
            const negative = std.mem.startsWith(u8, text, "-");
            if (negative) try display.writer.writeByte('-');
            const digits = text[@intFromBool(negative)..];
            const first = (digits.len - 1) % 3 + 1;
            try display.writer.writeAll(digits[0..first]);
            var groups = std.mem.window(u8, digits[first..], 3, 3);
            while (groups.next()) |group| {
                try display.writer.writeByte('_');
                try display.writer.writeAll(group);
            }
        } else try display.writer.writeAll(text);
        slot.* = try std.fmt.allocPrint(a, "The value of \"delay\" is out of range. It must be {s}. Received {s}", .{ if (integer) ">= 0 && <= 4294967295" else "an integer", display.written() });
    }
    return error.InvalidMistralTimeout;
}
pub fn endpoint(a: std.mem.Allocator, base: []const u8) !std.Uri {
    var parsed = try std.Uri.parse(base);
    parsed.scheme = try std.ascii.allocLowerString(a, parsed.scheme);
    if (parsed.host) |host| parsed.host = .{ .raw = try std.ascii.allocLowerString(a, try host.toRawMaybeAlloc(a)) };
    if (parsed.port) |port| if ((std.mem.eql(u8, parsed.scheme, "https") and port == 443) or (std.mem.eql(u8, parsed.scheme, "http") and port == 80)) {
        parsed.port = null;
    };
    var path = std.Io.Writer.Allocating.init(a);
    defer path.deinit();
    try parsed.path.formatPath(&path.writer);
    parsed.path = .{ .percent_encoded = try std.fmt.allocPrint(a, "{s}/", .{std.mem.trimEnd(u8, path.written(), "/")}) };
    const relative = "v1/chat/completions";
    const capacity = std.math.add(usize, base.len, relative.len + 1) catch return error.OutOfMemory;
    var storage = try a.alloc(u8, capacity);
    @memcpy(storage[0..relative.len], relative);
    return std.Uri.resolveInPlace(parsed, relative.len, &storage);
}
/// The outer catch formats the current failure, even after an earlier chunk
/// supplied an error finish reason. Its abort status comes from caller state.
pub fn prepareFailure(machine: *@import("reducer.zig").Machine, err: anyerror, aborted: bool) void {
    machine.mistralAborted = aborted;
    // Only the diagnostic assigned by an earlier finish_reason is stale.
    // Schema, request, callback and parser diagnostics keep their own storage.
    if (machine.mistralFinishReasonMessage) |previous| if (machine.message.errorMessage) |current| {
        if (current.ptr == previous.ptr and current.len == previous.len) machine.message.errorMessage = null;
    };
    if (err == error.Canceled and machine.message.errorMessage == null) machine.message.errorMessage = "This operation was aborted";
}
const ToolCallIdNormalizer = struct {
    ids: std.StringHashMapUnmanaged([]const u8) = .{},
    reverse: std.StringHashMapUnmanaged([]const u8) = .{},
    fn deinit(self: *@This(), a: std.mem.Allocator) void {
        self.ids.deinit(a);
        self.reverse.deinit(a);
    }
    fn normalize(ctx: ?*anyopaque, a: std.mem.Allocator, id: []const u8, _: t.Model, _: t.AssistantMessage) ![]const u8 {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        if (self.ids.get(id)) |existing| return existing;
        var attempt: usize = 0;
        while (true) : (attempt += 1) {
            const candidate = try deriveToolCallId(a, id, attempt);
            if (self.reverse.get(candidate)) |owner| if (!std.mem.eql(u8, owner, id)) {
                continue;
            };
            try self.ids.put(a, id, candidate);
            try self.reverse.put(a, candidate, id);
            return candidate;
        }
    }
};
pub fn mapToolChoice(a: std.mem.Allocator, choice: Value) !?Value {
    if (!j.truthy(choice)) return null;
    if (choice == .string) for ([_][]const u8{ "auto", "none", "any", "required" }) |name| if (std.mem.eql(u8, name, choice.string)) return choice;
    const function = j.get(choice, "function") orelse return error.InvalidMistralToolChoice;
    return try j.from(a, .{ .type = "function", .function = .{ .name = j.field(function, "name") orelse return error.InvalidMistralToolChoice } });
}
pub fn deriveToolCallId(a: std.mem.Allocator, id: []const u8, attempt: usize) ![]const u8 {
    var filtered: std.ArrayList(u8) = .empty;
    for (id) |byte| if (std.ascii.isAlphanumeric(byte)) {
        try filtered.append(a, byte);
    };
    if (attempt == 0 and filtered.items.len == 9) return filtered.items;
    const base = if (filtered.items.len > 0) filtered.items else id;
    const seed = if (attempt == 0) base else try std.fmt.allocPrint(a, "{s}:{d}", .{ base, attempt });
    const hash = try @import("../utils/hash.zig").shortHash(a, seed);
    return hash[0..@min(hash.len, 9)];
}
fn stripSymbolKeys(a: std.mem.Allocator, value: Value) anyerror!Value {
    return switch (value) {
        .array => |items| copied: {
            var result = j.array(a);
            for (items.items) |item| try result.array.append(try stripSymbolKeys(a, item));
            break :copied result;
        },
        .object => |object| copied: {
            var result = j.map(a);
            const entries = try j.objectEntries(a, object);
            defer a.free(entries);
            for (entries) |entry| {
                // The source copies into {}, using property assignment. Its
                // inherited __proto__ setter does not create an own JSON key.
                if (std.mem.eql(u8, entry.key, "__proto__")) continue;
                try j.put(a, &result, entry.key, try stripSymbolKeys(a, entry.node.*));
            }
            break :copied result;
        },
        else => value,
    };
}
pub fn buildPayload(a: std.mem.Allocator, io: std.Io, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !Value {
    const normalized = try tr.resolveTranscript(a, context, c.compatBool(model, "supportsMidConvoSystemMessages", false));
    var normalizer = ToolCallIdNormalizer{};
    defer normalizer.deinit(a);
    const transformed = try @import("transform_messages.zig").transformMessagesWithNormalizer(a, io, normalized.messages, model, .{ .context = &normalizer, .call = ToolCallIdNormalizer.normalize });
    var messages = j.array(a);
    const vision = for (model.input) |modality| {
        if (modality == .image) break true;
    } else false;
    for (transformed, 0..) |message, index| switch (message) {
        .system => |s| {
            const text = if (index == 0) try txt.getSystemMessageText(a, s) else try txt.renderSystemMessageUpdate(a, s);
            if (text.len > 0) try messages.array.append(try j.from(a, .{ .role = "system", .content = try sanitizeText(a, text) }));
        },
        .user => |user| {
            if (user.content == .missing or user.content == .nullContent) return error.InvalidMessageContent;
            if (user.content == .string) {
                try messages.array.append(try j.from(a, .{ .role = "user", .content = try sanitizeText(a, user.content.string) }));
                continue;
            }
            var content = j.array(a);
            for (user.content.blocks) |block| switch (block) {
                .text => |v| try content.array.append(try j.from(a, .{ .type = "text", .text = try sanitizeText(a, v.text) })),
                .image => |v| if (vision) {
                    try content.array.append(try j.from(a, .{ .type = "image_url", .imageUrl = try c.imageDataUri(a, v) }));
                },
                else => {},
            };
            if (content.array.items.len > 0) {
                var entry = try j.from(a, .{ .role = "user" });
                try j.put(a, &entry, "content", content);
                try messages.array.append(entry);
            } else if (!vision) {
                const had_images = for (user.content.blocks) |block| {
                    if (block == .image) break true;
                } else false;
                if (had_images) try messages.array.append(try j.from(a, .{ .role = "user", .content = "(image omitted: model does not support images)" }));
            }
        },
        .assistant => |assistant| {
            var content = j.array(a);
            var calls = j.array(a);
            for (assistant.content) |block| switch (block) {
                .text => |v| if (!try @import("../utils/unicode.zig").isBlank(v.text)) {
                    try content.array.append(try j.from(a, .{ .type = "text", .text = try sanitizeText(a, v.text) }));
                },
                .thinking => |v| if (!try @import("../utils/unicode.zig").isBlank(v.thinking)) {
                    try content.array.append(try j.from(a, .{ .type = "thinking", .thinking = .{.{ .type = "text", .text = try sanitizeText(a, v.thinking) }} }));
                },
                .toolCall => |call| try calls.array.append(try j.from(a, .{ .id = call.id, .type = "function", .function = .{ .name = call.name, .arguments = try @import("../json/value.zig").stringify(a, if (j.truthy(call.arguments)) call.arguments else j.map(a), .{}) }, .index = 0 })),
                else => {},
            };
            if (content.array.items.len == 0 and calls.array.items.len == 0) continue;
            var entry = try j.from(a, .{ .role = "assistant", .prefix = false });
            if (content.array.items.len > 0) try j.put(a, &entry, "content", content);
            if (calls.array.items.len > 0) try j.put(a, &entry, "toolCalls", calls);
            try messages.array.append(entry);
        },
        .toolResult => |result| {
            var content = j.array(a);
            const text = try @import("../utils/unicode.zig").trimWhitespace(a, try sanitizeText(a, try txt.contentText(a, .{ .blocks = result.content }, "\n")));
            const has_images = for (result.content) |block| {
                if (block == .image) break true;
            } else false;
            const prefix = if (result.isError) "[tool error] " else "";
            const output = if (text.len > 0) try std.fmt.allocPrint(a, "{s}{s}{s}", .{ prefix, text, if (has_images and !vision) "\n[tool image omitted: model does not support images]" else "" }) else try std.fmt.allocPrint(a, "{s}{s}", .{ prefix, if (has_images) (if (vision) "(see attached image)" else "(image omitted: model does not support images)") else "(no tool output)" });
            try content.array.append(try j.from(a, .{ .type = "text", .text = output }));
            if (vision) for (result.content) |block| if (block == .image) {
                try content.array.append(try j.from(a, .{ .type = "image_url", .imageUrl = try c.imageDataUri(a, block.image) }));
            };
            var entry = try j.from(a, .{ .role = "tool", .toolCallId = result.toolCallId, .name = result.toolName });
            try j.put(a, &entry, "content", content);
            try messages.array.append(entry);
        },
    };
    var payload = try j.from(a, .{ .model = model.id, .stream = true });
    try j.put(a, &payload, "messages", messages);
    const tools = try tr.getCurrentTools(a, context.messages);
    if (tools.len > 0) {
        var out = j.array(a);
        for (tools) |tool| {
            const strict = try constrained.resolveJsonSchemaStrictSamplingWithError(a, tool, true, null, options.errorMessage);
            const parameters = try stripSymbolKeys(a, try constrained.getJsonSchemaToolParametersWithError(a, tool, strict, options.errorMessage));
            try out.array.append(try j.from(a, .{ .type = "function", .function = .{ .name = tool.name, .description = tool.description, .parameters = parameters, .strict = strict orelse false } }));
        }
        try j.put(a, &payload, "tools", out);
    }
    if (try j.fieldValue(a, options, "temperature")) |value| try j.put(a, &payload, "temperature", value);
    if (try j.fieldValue(a, options, "maxTokens")) |value| try j.put(a, &payload, "maxTokens", value);
    if (c.extra(options, "toolChoice")) |choice| {
        if (try mapToolChoice(a, choice)) |mapped| try j.put(a, &payload, "toolChoice", mapped);
    } else if (options.toolChoice) |v| try j.put(a, &payload, "toolChoice", j.str(@tagName(v)));
    // Only streamSimple maps generic reasoning levels. The direct adapter
    // carries the provider-specific options supplied by its caller.
    if (c.extra(options, "promptMode")) |mode| if (j.truthy(mode)) {
        try j.put(a, &payload, "promptMode", mode);
    };
    if (c.extra(options, "reasoningEffort")) |effort| if (j.truthy(effort)) {
        try j.put(a, &payload, "reasoningEffort", effort);
    };
    if (options.cacheRetention != .none) if (options.sessionId) |id| if (id.len > 0) try j.put(a, &payload, "promptCacheKey", j.str(id));
    return payload;
}
fn remap(a: std.mem.Allocator, value: *Value, source: []const u8, target: []const u8) !void {
    if (value.* != .object) return;
    if (value.object.fetchOrderedRemove(source)) |entry| try j.put(a, value, target, entry.value);
}
pub fn toMistralWirePayload(a: std.mem.Allocator, payload: Value) !Value {
    return toMistralWirePayloadWithError(a, payload, null);
}
pub fn toMistralWirePayloadWithError(a: std.mem.Allocator, payload: Value, error_message: ?*?[]const u8) !Value {
    if (payload == .null) {
        if (error_message) |slot| slot.* = "Cannot read properties of null (reading 'messages')";
        return error.InvalidMistralMessages;
    }
    var wire = try j.objectSpread(a, payload);
    inline for (.{ .{ "topP", "top_p" }, .{ "maxTokens", "max_tokens" }, .{ "randomSeed", "random_seed" }, .{ "responseFormat", "response_format" }, .{ "toolChoice", "tool_choice" }, .{ "presencePenalty", "presence_penalty" }, .{ "frequencyPenalty", "frequency_penalty" }, .{ "parallelToolCalls", "parallel_tool_calls" }, .{ "reasoningEffort", "reasoning_effort" }, .{ "promptMode", "prompt_mode" }, .{ "promptCacheKey", "prompt_cache_key" }, .{ "safePrompt", "safe_prompt" } }) |pair| try remap(a, &wire, pair[0], pair[1]);
    const messages = wire.object.getPtr("messages") orelse {
        if (error_message) |slot| slot.* = "Cannot read properties of undefined (reading 'map')";
        return error.InvalidMistralMessages;
    };
    if (messages.* != .array) {
        if (error_message) |slot| slot.* = if (messages.* == .null) "Cannot read properties of null (reading 'map')" else "payload.messages.map is not a function";
        return error.InvalidMistralMessages;
    }
    // Subsequent wire transformations own their tree; object spread above
    // only establishes the source's primitive/null behavior and property order.
    messages.* = try j.clone(a, messages.*);
    for (messages.array.items) |*message| {
        if (message.* == .null) {
            if (error_message) |slot| slot.* = "Cannot read properties of null (reading 'content')";
            return error.InvalidMistralMessage;
        }
        if (message.* != .object) message.* = try j.objectSpread(a, message.*);
        try remap(a, message, "toolCalls", "tool_calls");
        try remap(a, message, "toolCallId", "tool_call_id");
        if (message.object.getPtr("content")) |content| if (content.* == .array) {
            for (content.array.items) |*chunk| {
                if (chunk.* != .object) chunk.* = try j.objectSpread(a, chunk.*);
                inline for (.{ .{ "imageUrl", "image_url" }, .{ "documentUrl", "document_url" }, .{ "documentName", "document_name" }, .{ "fileId", "file_id" }, .{ "referenceIds", "reference_ids" }, .{ "inputAudio", "input_audio" } }) |pair| try remap(a, chunk, pair[0], pair[1]);
            }
        };
    }
    if (wire.object.getPtr("response_format")) |format| if (format.* == .object) {
        format.* = try j.clone(a, format.*);
        try remap(a, format, "jsonSchema", "json_schema");
        if (format.object.getPtr("json_schema")) |schema| try remap(a, schema, "schemaDefinition", "schema");
    };
    return wire;
}
pub fn consume(self: *@import("reducer.zig").Machine, event: Value) !void {
    const chunk = event;
    if (self.message.responseId == null or self.message.responseId.?.len == 0) self.message.responseId = j.field(chunk, "id");
    if (j.get(chunk, "usage")) |usage| if (j.truthy(usage)) {
        const accounting = @import("../utils/usage.zig");
        const a = self.allocator;
        const prompt = try accounting.number(a, accounting.orZero(usage, "prompt_tokens"));
        const raw = j.getNonNull(j.get(usage, "promptTokensDetails") orelse .null, "cachedTokens") orelse j.getNonNull(j.get(usage, "prompt_tokens_details") orelse .null, "cached_tokens") orelse j.getNonNull(j.get(usage, "promptTokenDetails") orelse .null, "cachedTokens") orelse j.getNonNull(j.get(usage, "prompt_token_details") orelse .null, "cached_tokens") orelse j.getNonNull(usage, "numCachedTokens") orelse j.getNonNull(usage, "num_cached_tokens");
        const cached = if (raw) |v| j.number(v) orelse 0 else 0;
        const numbers = @import("../utils/numbers.zig");
        const reads = numbers.minimum(prompt, numbers.maximum(0, if (std.math.isFinite(cached)) cached else 0));
        try accounting.set(a, &self.message.usage, "input", j.num(numbers.maximum(0, prompt - reads)));
        try accounting.set(a, &self.message.usage, "output", accounting.orZero(usage, "completion_tokens"));
        try accounting.set(a, &self.message.usage, "cacheRead", j.num(reads));
        try accounting.set(a, &self.message.usage, "cacheWrite", j.num(0));
        if (j.get(usage, "total_tokens")) |total| {
            if (j.truthy(total)) try accounting.set(a, &self.message.usage, "totalTokens", total) else try accounting.total(a, &self.message.usage);
        } else try accounting.total(a, &self.message.usage);
        _ = try @import("../utils/cost.zig").calculateCost(self.allocator, self.model, &self.message.usage);
    };
    const choices = c.array(chunk, "choices");
    if (choices.len == 0) return;
    const choice = choices[0];
    if (j.field(choice, "finish_reason")) |reason| if (reason.len > 0) {
        self.terminal = true;
        self.message.rawStopReason = reason;
        self.message.stopReason = if (std.mem.eql(u8, reason, "stop")) .stop else if (std.mem.eql(u8, reason, "length") or std.mem.eql(u8, reason, "model_length")) .length else if (std.mem.eql(u8, reason, "tool_calls")) .toolUse else .@"error";
        if (self.message.stopReason == .@"error") {
            self.message.errorMessage = if (std.mem.eql(u8, reason, "error")) "Provider stopped with: error (server error)" else try std.fmt.allocPrint(self.allocator, "Provider stopped with: {s}", .{reason});
            self.mistralFinishReasonMessage = self.message.errorMessage;
        }
    };
    const delta = j.get(choice, "delta") orelse return;
    if (j.get(delta, "content")) |content| {
        if (content == .string) {
            const bytes = try sanitizeText(self.allocator, content.string);
            if (bytes.len > 0) try self.delta(try self.textSlot(false, true), bytes);
        } else if (content == .array) for (content.array.items) |item| {
            if (item == .string) {
                const bytes = try sanitizeText(self.allocator, item.string);
                if (bytes.len > 0) try self.delta(try self.textSlot(false, true), bytes);
                continue;
            }
            const kind = j.field(item, "type") orelse continue;
            var bytes: []const u8 = "";
            const thinking = std.mem.eql(u8, kind, "thinking");
            if (thinking) {
                var parts: std.ArrayList([]const u8) = .empty;
                for (c.array(item, "thinking")) |part| try parts.append(self.allocator, j.field(part, "text") orelse "");
                bytes = try std.mem.join(self.allocator, "", parts.items);
            } else if (std.mem.eql(u8, kind, "text")) bytes = j.field(item, "text") orelse "";
            const clean = try sanitizeText(self.allocator, bytes);
            if (clean.len > 0) try self.delta(try self.textSlot(thinking, true), clean);
        };
    }
    for (c.array(delta, "tool_calls")) |call| {
        if (self.current) |index| if (self.blocks.items[index] != .toolCall) try self.end(index);
        const function = j.get(call, "function") orelse return error.InvalidMistralToolCall;
        const provided = j.field(call, "id");
        const wire_number = j.number(j.get(call, "index") orelse .null);
        // JavaScript's Map keys are numbers, including fractional/negative
        // values. Preserve their identity without a trapping unsigned cast.
        const wire_index: ?u64 = if (wire_number) |number| c.numberMapKey(number) else null;
        const id = if (provided != null and !std.mem.eql(u8, provided.?, "null") and provided.?.len > 0) provided.? else try deriveToolCallId(self.allocator, try std.fmt.allocPrint(self.allocator, "toolcall:{s}", .{try @import("../utils/numbers.zig").toString(self.allocator, wire_number orelse 0)}), 0);
        const existing = if (wire_index) |key| self.slots.get(key) else self.completionToolIds.get(id);
        const index = existing orelse blk: {
            const name = j.field(function, "name") orelse "";
            const fields = try j.from(self.allocator, .{ .type = "toolCall", .id = id, .name = name, .arguments = j.map(self.allocator), .partialArgs = "" });
            const created = try self.begin(.{ .toolCall = .{ .id = id, .name = name, .arguments = j.map(self.allocator), .jsonFields = fields } });
            if (wire_index) |key| try self.slots.put(self.allocator, key, created) else try self.completionToolIds.put(self.allocator, id, created);
            break :blk created;
        };
        const arguments = j.get(function, "arguments") orelse j.map(self.allocator);
        const args_delta = if (arguments == .string) arguments.string else try @import("../json/value.zig").stringify(self.allocator, if (j.truthy(arguments)) arguments else j.map(self.allocator), .{});
        try j.put(self.allocator, &self.blocks.items[index].toolCall.jsonFields, "partialArgs", j.str(try std.mem.concat(self.allocator, u8, &.{ self.states.items[index].arguments.items, args_delta })));
        try self.delta(index, args_delta);
        self.current = null;
    }
}

pub fn mistralConversationsApi() @import("../models.zig").ApiImplementation {
    return @import("driver.zig").implementation("mistral-conversations");
}
pub fn stream(operation: @import("../models.zig").OperationContext, model: t.Model, context: t.TranscriptContext, options: MistralOptions) !*@import("../utils/event_stream.zig").AssistantMessageEventStream {
    if (!std.mem.eql(u8, model.api, "mistral-conversations")) return error.MismatchedApi;
    var arena = std.heap.ArenaAllocator.init(operation.allocator);
    defer arena.deinit();
    return @import("driver.zig").stream(null, operation, model, context, try options.streamOptions(arena.allocator()));
}
pub fn streamSimple(operation: @import("../models.zig").OperationContext, model: t.Model, context: t.TranscriptContext, options: t.SimpleStreamOptions) !*@import("../utils/event_stream.zig").AssistantMessageEventStream {
    if (!std.mem.eql(u8, model.api, "mistral-conversations")) return error.MismatchedApi;
    return @import("driver.zig").streamSimple(null, operation, model, context, options);
}
