const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const c = @import("common.zig");
const Value = std.json.Value;
pub const PiMessagesOptions = @import("options.zig").PiMessagesOptions;
pub const PiMessagesRewriteImpact = struct { policyId: []const u8, policyVersion: f64, changed: bool, tokenCountChange: f64, messageCountChange: f64, systemPromptChanged: bool };
pub const PiMessagesEvent = union(enum) {
    start: struct {
        jsonFields: Value = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    text_start: struct {
        contentIndex: f64,
        jsonFields: Value = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    thinking_start: struct {
        contentIndex: f64,
        jsonFields: Value = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    text_delta: struct {
        contentIndex: f64,
        delta: []const u8,
        jsonFields: Value = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    thinking_delta: struct {
        contentIndex: f64,
        delta: []const u8,
        jsonFields: Value = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    toolcall_delta: struct {
        contentIndex: f64,
        delta: []const u8,
        jsonFields: Value = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    text_end: struct {
        contentIndex: f64,
        content: []const u8,
        contentSignature: ?[]const u8 = null,
        jsonFields: Value = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    thinking_end: struct {
        contentIndex: f64,
        content: []const u8,
        contentSignature: ?[]const u8 = null,
        redacted: ?bool = null,
        jsonFields: Value = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    toolcall_start: struct {
        contentIndex: f64,
        id: []const u8,
        toolName: []const u8,
        jsonFields: Value = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    toolcall_end: struct {
        contentIndex: f64,
        toolCall: t.ToolCall,
        jsonFields: Value = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    done: struct {
        reason: enum { stop, length, toolUse },
        usage: t.Usage,
        responseId: ?[]const u8 = null,
        providerThinkingLevel: ?[]const u8 = null,
        rewrite: ?PiMessagesRewriteImpact = null,
        jsonFields: Value = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    @"error": struct {
        reason: enum { aborted, @"error" },
        usage: t.Usage,
        errorMessage: ?[]const u8 = null,
        responseId: ?[]const u8 = null,
        providerThinkingLevel: ?[]const u8 = null,
        rewrite: ?PiMessagesRewriteImpact = null,
        jsonFields: Value = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    pub const jsonStringify = j.Tagged(@This(), "type").jsonStringify;
    pub const jsonParse = j.Tagged(@This(), "type").jsonParse;
    pub const jsonParseFromValue = j.Tagged(@This(), "type").jsonParseFromValue;
};
pub const PiMessagesResponseError = struct { message: []const u8, code: ?[]const u8 = null, diagnosticDetails: Value };
pub fn endpoint(a: std.mem.Allocator, base: []const u8, debug: bool) !std.Uri {
    const uri = @import("../transport/uri.zig");
    var url = try std.Uri.parse(try std.fmt.allocPrint(a, "{s}/messages", .{std.mem.trimEnd(u8, base, "/")}));
    if (debug) {
        const existing = if (url.query) |query| try uri.decodeFormPairs(a, query) else &.{};
        var pairs: std.ArrayList(uri.Pair) = .empty;
        var replaced = false;
        for (existing) |pair| {
            if (std.mem.eql(u8, pair.name, "debug")) {
                if (!replaced) try pairs.append(a, .{ .name = "debug", .value = "1" });
                replaced = true;
            } else try pairs.append(a, pair);
        }
        if (!replaced) try pairs.append(a, .{ .name = "debug", .value = "1" });
        url.query = .{ .percent_encoded = try uri.encodeForm(a, pairs.items) };
    }
    return url;
}
pub fn requestHeaders(a: std.mem.Allocator, options: t.StreamOptions) !t.ProviderHeaders {
    const key = options.apiKey orelse return error.MissingAuthentication;
    if (key.len == 0) return error.MissingAuthentication;
    const headers = @import("../utils/headers.zig");
    var record = try j.from(a, .{ .authorization = try std.fmt.allocPrint(a, "Bearer {s}", .{key}), .accept = "text/event-stream", .@"content-type" = "application/json" });
    if (try headers.providerHeadersToRecord(a, &.{options.headers})) |overrides| {
        var iterator = overrides.object.iterator();
        while (iterator.next()) |entry| try j.put(a, &record, entry.key_ptr.*, entry.value_ptr.*);
    }
    // The source spreads the override record before Fetch normalizes names.
    return headers.fromValue(a, try headers.headersToRecord(a, try headers.fromValue(a, record)));
}
pub fn createPiMessagesResponseError(allocator: std.mem.Allocator, io: std.Io, model: t.Model, url: std.Uri, status: u16, statusText: []const u8, body: []const u8) !PiMessagesResponseError {
    var temporary = std.heap.ArenaAllocator.init(allocator);
    defer temporary.deinit();
    const a = temporary.allocator();
    const parsed = @import("../json/value.zig").parseFromSliceLeaky(Value, a, body, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |err| blk: {
        if (err == error.OutOfMemory) return err;
        break :blk null;
    };
    const error_value = if (parsed) |value| j.get(value, "error") else null;
    const structured = error_value != null and error_value.? == .object;
    const code = if (structured) j.field(error_value.?, "code") else null;
    const suffix = if (structured) j.field(error_value.?, "message") orelse body else body;
    const message = try std.fmt.allocPrint(a, "{d} {s}: {s}{s}", .{ status, statusText, suffix, if (code != null and code.?.len > 0) try std.fmt.allocPrint(a, " ({s})", .{code.?}) else "" });
    var details = try j.from(a, .{ .version = 1, .provider = model.provider, .model = model.id, .url = try @import("../transport/uri.zig").render(a, url), .status = status, .statusText = statusText });
    if (structured) try j.put(a, &details, "error", error_value.?) else {
        const units = try std.unicode.wtf8ToWtf16LeAlloc(a, body);
        const truncated = if (units.len > 8192) try std.fmt.allocPrint(a, "{s}…", .{try std.unicode.wtf16LeToWtf8Alloc(a, units[0..8192])}) else body;
        try j.put(a, &details, "body", j.str(truncated));
    }
    try j.put(a, &details, "timestampMs", j.int(std.Io.Clock.real.now(io).toMilliseconds()));
    return t.cloneInto(PiMessagesResponseError, allocator, .{ .message = message, .code = code, .diagnosticDetails = details });
}
pub fn buildPayload(a: std.mem.Allocator, _: std.Io, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !Value {
    var payload = try j.from(a, .{ .model = model.id, .context = context });
    const retention = options.cacheRetention orelse if (options.env.get("PI_CACHE_RETENTION")) |v| (if (std.mem.eql(u8, v, "long")) @as(?t.CacheRetention, .long) else null) else null;
    var wire_options = try j.from(a, .{ .temperature = options.temperature, .maxTokens = options.maxTokens, .reasoning = options.reasoning, .cacheRetention = retention, .sessionId = options.sessionId, .toolChoice = options.toolChoice });
    inline for (.{ "temperature", "maxTokens", "reasoning", "sessionId", "toolChoice" }) |name| {
        if (try j.fieldValue(a, options, name)) |value| try j.put(a, &wire_options, name, value);
    }
    if (c.extra(options, "toolChoice")) |choice| try j.put(a, &wire_options, "toolChoice", choice);
    try j.put(a, &payload, "options", wire_options);
    return payload;
}
const Slot = struct { block: *t.Content, state: *@import("reducer.zig").Machine.BlockState, key: ?[]const u8 = null };
fn contentSlot(self: *@import("reducer.zig").Machine, index: f64, create: bool) !Slot {
    const key = try @import("../utils/numbers.zig").toString(self.allocator, index);
    if (j.arrayIndex(key)) |array_index| {
        const position: usize = @intCast(array_index);
        if (create) {
            const length = std.math.add(usize, position, 1) catch return error.InvalidPiMessagesContentIndex;
            try self.blocks.ensureTotalCapacity(self.allocator, length);
            try self.states.ensureTotalCapacity(self.allocator, length);
            while (self.blocks.items.len < length) {
                self.blocks.appendAssumeCapacity(.{ .unknown = .null });
                self.states.appendAssumeCapacity(.{});
            }
        } else if (position >= self.blocks.items.len) return error.InvalidPiMessagesContentIndex;
        return .{ .block = &self.blocks.items[position], .state = &self.states.items[position] };
    }
    if (create) {
        const entry = try self.piProperties.getOrPut(self.allocator, key);
        if (!entry.found_existing) entry.value_ptr.* = .{};
        return .{ .block = &entry.value_ptr.block, .state = &entry.value_ptr.state, .key = key };
    }
    const entry = self.piProperties.getPtr(key) orelse return error.InvalidPiMessagesContentIndex;
    return .{ .block = &entry.block, .state = &entry.state, .key = key };
}
fn publishProperty(self: *@import("reducer.zig").Machine, slot: Slot) !void {
    if (slot.key) |key| {
        if (self.message.jsonContentProperties == .null) self.message.jsonContentProperties = j.map(self.allocator);
        try j.put(self.allocator, &self.message.jsonContentProperties, key, try j.toValue(self.allocator, slot.block.*, .{ .emit_null_optional_fields = false }));
    }
}
fn blockObject(self: *@import("reducer.zig").Machine, slot: Slot) !Value {
    const value = try j.toValue(self.allocator, slot.block.*, .{ .emit_null_optional_fields = false });
    if (value != .object) return error.InvalidPiMessagesContentIndex;
    return value;
}
fn replaceBlock(self: *@import("reducer.zig").Machine, slot: Slot, value: Value) !void {
    slot.block.* = try std.json.parseFromValueLeaky(t.Content, self.allocator, value, .{ .ignore_unknown_fields = true });
    try publishProperty(self, slot);
}
fn assignOptional(a: std.mem.Allocator, target: *Value, event: Value, source: []const u8, destination: []const u8) !void {
    if (j.get(event, source)) |value| try j.put(a, target, destination, value) else _ = target.object.orderedRemove(destination);
}
fn forward(self: *@import("reducer.zig").Machine, event: Value) !void {
    var fields = try j.objectSpread(self.allocator, event);
    try j.put(self.allocator, &fields, "partial", try j.toValue(self.allocator, self.snapshot(), .{ .emit_null_optional_fields = false }));
    var converted = t.AssistantMessageEvent.jsonParseFromValue(self.allocator, fields, .{ .ignore_unknown_fields = true }) catch |err| blk: {
        if (err == error.OutOfMemory) return err;
        break :blk t.AssistantMessageEvent{ .unknown = .{ .partial = self.snapshot(), .jsonFields = try j.objectSpread(self.allocator, event) } };
    };
    switch (converted) {
        inline else => |*value| if (@hasField(@TypeOf(value.*), "partial")) {
            value.partial = self.snapshot();
        },
    }
    try self.stream.push(converted);
}
pub fn consume(self: *@import("reducer.zig").Machine, event: Value) !void {
    if (event == .null) return error.InvalidPiMessagesEvent;
    if (self.message.jsonFields == .null) self.message.jsonFields = try j.toValue(self.allocator, self.snapshot(), .{ .emit_null_optional_fields = false });
    const kind = j.field(event, "type") orelse "";
    const Indexed = enum { text_start, thinking_start, toolcall_start, text_delta, thinking_delta, toolcall_delta, text_end, thinking_end, toolcall_end };
    const index: f64 = if (std.meta.stringToEnum(Indexed, kind) != null) j.number(j.get(event, "contentIndex") orelse return error.InvalidPiMessagesContentIndex) orelse return error.InvalidPiMessagesContentIndex else 0;
    if (std.mem.eql(u8, kind, "start")) {
        self.started = true;
        try forward(self, event);
        return;
    }
    if (std.mem.eql(u8, kind, "text_start") or std.mem.eql(u8, kind, "thinking_start") or std.mem.eql(u8, kind, "toolcall_start")) {
        const slot = try contentSlot(self, index, true);
        slot.state.closed = false;
        if (std.mem.eql(u8, kind, "text_start")) {
            slot.block.* = .{ .text = .{ .text = "" } };
            try publishProperty(self, slot);
            try forward(self, event);
        } else if (std.mem.eql(u8, kind, "thinking_start")) {
            slot.block.* = .{ .thinking = .{ .thinking = "" } };
            try publishProperty(self, slot);
            try forward(self, event);
        } else {
            var block = j.map(self.allocator);
            try j.put(self.allocator, &block, "type", j.str("toolCall"));
            try assignOptional(self.allocator, &block, event, "id", "id");
            try assignOptional(self.allocator, &block, event, "toolName", "name");
            try j.put(self.allocator, &block, "arguments", j.map(self.allocator));
            try replaceBlock(self, slot, block);
            slot.state.*.arguments.clearRetainingCapacity();
            slot.state.*.argumentsComplete = false;
            try publishProperty(self, slot);
            try forward(self, event);
        }
        return;
    }
    if (std.mem.eql(u8, kind, "text_delta") or std.mem.eql(u8, kind, "thinking_delta") or std.mem.eql(u8, kind, "toolcall_delta")) {
        const slot = try contentSlot(self, index, false);
        var block = try blockObject(self, slot);
        const delta = j.field(event, "delta") orelse "";
        if (std.mem.eql(u8, kind, "toolcall_delta")) {
            try slot.state.*.arguments.appendSlice(self.allocator, delta);
            try j.put(self.allocator, &block, "arguments", try @import("../json/partial.zig").parseStreamingJson(self.allocator, slot.state.*.arguments.items));
        } else {
            const field = if (std.mem.eql(u8, kind, "text_delta")) "text" else "thinking";
            const previous = if (j.get(block, field)) |value| try @import("../utils/diagnostics.zig").formatThrownValue(self.allocator, .{ .value = value }) else "undefined";
            try j.put(self.allocator, &block, field, j.str(try std.mem.concat(self.allocator, u8, &.{ previous, delta })));
        }
        try replaceBlock(self, slot, block);
        slot.state.*.closed = false;
        try forward(self, event);
        return;
    }
    if (std.mem.eql(u8, kind, "text_end")) {
        const slot = try contentSlot(self, index, false);
        var block = try blockObject(self, slot);
        try assignOptional(self.allocator, &block, event, "content", "text");
        try assignOptional(self.allocator, &block, event, "contentSignature", "textSignature");
        try replaceBlock(self, slot, block);
        slot.state.*.closed = true;
        try forward(self, event);
        return;
    }
    if (std.mem.eql(u8, kind, "thinking_end")) {
        const slot = try contentSlot(self, index, false);
        var block = try blockObject(self, slot);
        try assignOptional(self.allocator, &block, event, "content", "thinking");
        try assignOptional(self.allocator, &block, event, "contentSignature", "thinkingSignature");
        try assignOptional(self.allocator, &block, event, "redacted", "redacted");
        try replaceBlock(self, slot, block);
        slot.state.*.closed = true;
        try forward(self, event);
        return;
    }
    if (std.mem.eql(u8, kind, "toolcall_end")) {
        const slot = try contentSlot(self, index, false);
        var merged = try blockObject(self, slot);
        const supplied = try j.objectSpread(self.allocator, j.get(event, "toolCall") orelse .null);
        for (try j.objectEntries(self.allocator, supplied.object)) |entry| try j.put(self.allocator, &merged, entry.key, entry.node.*);
        try replaceBlock(self, slot, merged);
        slot.state.*.arguments.clearRetainingCapacity();
        slot.state.*.argumentsComplete = true;
        slot.state.*.closed = true;
        try self.stream.push(.{ .toolcall_end = .{ .contentIndex = index, .toolCall = slot.block.*, .partial = self.snapshot() } });
        return;
    }
    if (std.mem.eql(u8, kind, "done") or std.mem.eql(u8, kind, "error")) {
        self.piTerminal = if (std.mem.eql(u8, kind, "done")) .done else .@"error";
        self.terminal = true;
        self.message.stopReason = std.meta.stringToEnum(t.StopReason, j.field(event, "reason") orelse "error") orelse return error.InvalidPiStopReason;
        if (j.get(event, "usage")) |usage| {
            self.message.usage = std.json.parseFromValueLeaky(t.Usage, self.allocator, usage, .{}) catch |err| blk: {
                if (err == error.OutOfMemory) return err;
                try j.retainField(self.allocator, &self.message, "usage", usage);
                break :blk t.Usage{};
            };
        } else j.markAbsent(&self.message, "usage");
        self.message.responseId = j.field(event, "responseId");
        if (j.get(event, "responseId")) |value| {
            if (j.text(value) == null) try j.retainField(self.allocator, &self.message, "responseId", value);
        } else j.markAbsent(&self.message, "responseId");
        if (j.get(event, "providerThinkingLevel")) |value| {
            self.message.providerThinkingLevel = j.text(value);
            if (j.text(value) == null) try j.retainField(self.allocator, &self.message, "providerThinkingLevel", value);
        }
        if (std.mem.eql(u8, kind, "error")) {
            self.message.errorMessage = j.field(event, "errorMessage");
            if (j.get(event, "errorMessage")) |value| {
                if (j.text(value) == null) try j.retainField(self.allocator, &self.message, "errorMessage", value);
            } else j.markAbsent(&self.message, "errorMessage");
        }
        if (j.get(event, "rewrite")) |rewrite| if (j.truthy(rewrite)) {
            const diagnostic = try j.from(self.allocator, .{ .type = "pi_messages_rewrite", .timestamp = @as(f64, @floatFromInt(std.Io.Clock.real.now(self.io).toMilliseconds())), .details = try j.objectSpread(self.allocator, rewrite) });
            const old = self.message.diagnostics orelse &.{};
            const out = try self.allocator.alloc(Value, old.len + 1);
            @memcpy(out[0..old.len], old);
            out[old.len] = diagnostic;
            self.message.diagnostics = out;
        };
        return;
    }
    try forward(self, event);
}

pub fn piMessagesApi() @import("../models.zig").ApiImplementation {
    return @import("driver.zig").implementation("pi-messages");
}
pub fn stream(operation: @import("../models.zig").OperationContext, model: t.Model, context: t.TranscriptContext, options: PiMessagesOptions) !*@import("../utils/event_stream.zig").AssistantMessageEventStream {
    if (!std.mem.eql(u8, model.api, "pi-messages")) return error.MismatchedApi;
    var arena = std.heap.ArenaAllocator.init(operation.allocator);
    defer arena.deinit();
    return @import("driver.zig").stream(null, operation, model, context, try options.streamOptions(arena.allocator()));
}
pub fn streamSimple(operation: @import("../models.zig").OperationContext, model: t.Model, context: t.TranscriptContext, options: t.SimpleStreamOptions) !*@import("../utils/event_stream.zig").AssistantMessageEventStream {
    if (!std.mem.eql(u8, model.api, "pi-messages")) return error.MismatchedApi;
    return @import("driver.zig").streamSimple(null, operation, model, context, options);
}
