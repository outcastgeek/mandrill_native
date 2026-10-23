const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const c = @import("common.zig");
const partial = @import("../json/partial.zig");
const constrained = @import("../schema/constrained.zig");
const events = @import("../utils/event_stream.zig");
const Value = std.json.Value;
pub const Machine = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    model: t.Model,
    stream: *events.AssistantMessageEventStream,
    message: t.AssistantMessage,
    blocks: std.ArrayList(t.Content) = .empty,
    states: std.ArrayList(BlockState) = .empty,
    slots: std.AutoHashMapUnmanaged(u64, usize) = .{},
    current: ?usize = null,
    terminal: bool = false,
    started: bool = false,
    piAborted: bool = false,
    mistralAborted: bool = false,
    mistralFinishReasonMessage: ?[]const u8 = null,
    finishReasonMessage: ?[]const u8 = null,
    piTerminal: ?enum { done, @"error" } = null,
    piProperties: std.StringHashMapUnmanaged(PiBlock) = .{},
    googleToolCallIds: ?*@import("google_shared.zig").GoogleToolCallIds = null,
    localGoogleToolCallIds: @import("google_shared.zig").GoogleToolCallIds = .{},
    completionText: ?usize = null,
    completionThinking: ?usize = null,
    completionToolIds: std.StringHashMapUnmanaged(usize) = .{},
    reasoningDetails: std.ArrayList(Value) = .empty,
    grammarToolInputProperties: std.StringHashMapUnmanaged([]const u8) = .{},
    responseReasoningIds: std.StringHashMapUnmanaged(usize) = .{},
    usageModel: ?t.Model = null,
    inputTransformations: ?Value = null,
    responsePricing: ?ResponsePricing = null,
    bedrockRequestId: ?[]const u8 = null,
    bedrockErrorCode: ?[]const u8 = null,
    bedrockFailureStatus: ?u16 = null,
    bedrockServerTime: ?[]const u8 = null,
    anthropicOAuth: bool = false,
    tools: []const t.Tool = &.{},
    pub const ResponsePricing = struct {
        context: ?*anyopaque = null,
        serviceTier: ?[]const u8 = null,
        resolve: ?*const fn (?*anyopaque, ?[]const u8, ?[]const u8) anyerror!?[]const u8 = null,
        apply: *const fn (?*anyopaque, *t.Usage, ?[]const u8) anyerror!void,
    };
    const CustomInput = struct { property: []const u8, buffer: constrained.GrammarToolInputJsonBuffer = .{} };
    pub const ProviderIndex = struct {
        value: ?Value = null,
        identity: ?usize = null,
        pub fn from(event: Value, name: []const u8) @This() {
            if (event != .object) return .{};
            const node = event.object.getPtr(name) orelse return .{};
            return .{ .value = node.*, .identity = switch (node.*) {
                .array => |list| if (list.capacity > 0) @intFromPtr(list.items.ptr) else @intFromPtr(node),
                .object => |object| if (object.count() > 0) @intFromPtr(object.keys().ptr) else @intFromPtr(node),
                else => null,
            } };
        }
        pub fn owned(self: @This(), a: std.mem.Allocator) !@This() {
            return .{ .value = if (self.value) |value| try j.clone(a, value) else null, .identity = self.identity };
        }
        pub fn eql(left: @This(), right: @This()) bool {
            const l = left.value orelse return right.value == null;
            const r = right.value orelse return false;
            if (j.number(l)) |number| return if (j.number(r)) |other| number == other else false;
            if (std.meta.activeTag(l) != std.meta.activeTag(r)) return false;
            return switch (l) {
                .null => true,
                .bool => |value| value == r.bool,
                .string => |value| std.mem.eql(u8, value, r.string),
                .array, .object => left.identity != null and left.identity == right.identity,
                .integer, .float, .number_string => false,
            };
        }
    };
    pub const BlockState = struct { closed: bool = false, arguments: std.ArrayList(u8) = .empty, argumentsComplete: bool = false, providerArgumentsMissing: bool = false, item_id: ?[]const u8 = null, completionStreamIndex: ?u64 = null, wireIndex: ProviderIndex = .{}, googleToolId: ?ProviderIndex = null, customInput: ?CustomInput = null, redactedBytes: std.ArrayList(u8) = .empty, redactedPending: bool = false };
    pub const PiBlock = struct { block: t.Content = .{ .unknown = .null }, state: BlockState = .{} };
    pub fn init(a: std.mem.Allocator, io: std.Io, model: t.Model, stream: *events.AssistantMessageEventStream) @This() {
        return .{ .allocator = a, .io = io, .model = model, .stream = stream, .message = .{ .api = model.api, .provider = model.provider, .model = model.id, .timestamp = @as(f64, @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds())) } };
    }
    pub fn snapshot(self: *@This()) t.AssistantMessage {
        self.message.content = self.blocks.items;
        return self.message;
    }
    pub fn prepareFailure(self: *@This()) void {
        const previous = self.finishReasonMessage orelse return;
        if (self.message.errorMessage) |current| if (current.ptr == previous.ptr and current.len == previous.len) {
            self.message.errorMessage = null;
        };
    }
    pub fn start(self: *@This()) !void {
        if (self.started) return;
        self.started = true;
        try self.stream.push(.{ .start = .{ .partial = self.snapshot() } });
    }
    pub fn begin(self: *@This(), block: t.Content) !usize {
        switch (block) {
            .text, .thinking, .toolCall => {},
            else => return error.InvalidAssistantContent,
        }
        // Every content index must have matching reducer state, including
        // when allocation fails before its start event can be published.
        try self.blocks.ensureUnusedCapacity(self.allocator, 1);
        try self.states.ensureUnusedCapacity(self.allocator, 1);
        self.blocks.appendAssumeCapacity(block);
        self.states.appendAssumeCapacity(.{});
        const index = self.blocks.items.len - 1;
        self.current = index;
        const message_snapshot = self.snapshot();
        switch (block) {
            .text => try self.stream.push(.{ .text_start = .{ .contentIndex = @floatFromInt(index), .partial = message_snapshot } }),
            .thinking => try self.stream.push(.{ .thinking_start = .{ .contentIndex = @floatFromInt(index), .partial = message_snapshot } }),
            .toolCall => try self.stream.push(.{ .toolcall_start = .{ .contentIndex = @floatFromInt(index), .partial = message_snapshot } }),
            else => return error.InvalidAssistantContent,
        }
        return index;
    }
    pub fn beginProviderBlock(self: *@This(), block: t.Content, wire_index: ProviderIndex) !usize {
        const retained = try wire_index.owned(self.allocator);
        const index = try self.begin(block);
        self.states.items[index].wireIndex = retained;
        return index;
    }
    pub fn findWireIndex(self: *@This(), wire_index: ProviderIndex) ?usize {
        // These adapters use Array.findIndex and strict equality, including
        // undefined after deleting a block's provider index on stop.
        for (self.states.items, 0..) |state, index| if (ProviderIndex.eql(state.wireIndex, wire_index)) return index;
        return null;
    }
    pub fn providerDelta(self: *@This(), index: usize, bytes: []const u8) !void {
        return self.providerDeltaWithRaw(index, bytes, null);
    }
    fn providerDeltaWithRaw(self: *@This(), index: usize, bytes: []const u8, raw: ?Value) !void {
        const state = &self.states.items[index];
        const closed = state.closed;
        state.closed = false;
        defer state.closed = closed;
        if (self.blocks.items[index] == .toolCall) {
            if (state.providerArgumentsMissing and std.mem.eql(u8, self.model.api, "anthropic-messages")) try state.arguments.appendSlice(self.allocator, try @import("../utils/diagnostics.zig").formatThrownValue(self.allocator, .undefined_value));
            state.providerArgumentsMissing = false;
            state.argumentsComplete = false;
        }
        try self.appendDelta(index, bytes);
        try self.publishDelta(index, bytes, raw);
    }
    /// A provider delta may append String(value) while retaining its original
    /// JSON value in the event. Keep post-stop lookup behavior unchanged.
    pub fn providerValueDelta(self: *@This(), index: usize, value: Value) !void {
        if (self.blocks.items[index] == .toolCall) {
            const bytes = try @import("../utils/diagnostics.zig").formatThrownValue(self.allocator, .{ .value = value });
            return self.providerDeltaWithRaw(index, bytes, if (value == .string) null else value);
        }
        const state = &self.states.items[index];
        const closed = state.closed;
        state.closed = false;
        defer state.closed = closed;
        return self.rawTextDelta(index, value, null);
    }
    pub fn endProviderBlock(self: *@This(), index: usize) !void {
        self.states.items[index].wireIndex = .{};
        self.states.items[index].closed = false;
        if (self.blocks.items[index] == .toolCall) self.states.items[index].argumentsComplete = false;
        try self.endWithOptions(index, true);
        self.states.items[index].arguments.clearRetainingCapacity();
        self.states.items[index].providerArgumentsMissing = true;
    }
    pub fn delta(self: *@This(), index: usize, bytes: []const u8) !void {
        try self.appendDelta(index, bytes);
        try self.publishDelta(index, bytes, null);
    }
    pub fn rawTextDelta(self: *@This(), index: usize, value: Value, signature: ?[]const u8) !void {
        const bytes = try @import("../utils/diagnostics.zig").formatThrownValue(self.allocator, .{ .value = value });
        try self.appendDelta(index, bytes);
        const retain = @import("google_shared.zig").retainThoughtSignature;
        switch (self.blocks.items[index]) {
            .text => |*text| text.textSignature = retain(text.textSignature, signature),
            .thinking => |*thinking| thinking.thinkingSignature = retain(thinking.thinkingSignature, signature),
            else => unreachable,
        }
        try self.publishDelta(index, bytes, if (value == .string) null else value);
    }
    fn appendDelta(self: *@This(), index: usize, bytes: []const u8) !void {
        if (index >= self.blocks.items.len or self.states.items[index].closed) return error.InvalidContentIndex;
        const block = &self.blocks.items[index];
        switch (block.*) {
            .text => |*v| v.text = try std.mem.concat(self.allocator, u8, &.{ v.text, bytes }),
            .thinking => |*v| v.thinking = try std.mem.concat(self.allocator, u8, &.{ v.thinking, bytes }),
            .toolCall => |*v| {
                try self.states.items[index].arguments.appendSlice(self.allocator, bytes);
                v.arguments = try partial.parseStreamingJson(self.allocator, self.states.items[index].arguments.items);
            },
            else => return error.InvalidAssistantContent,
        }
    }
    fn publishDelta(self: *@This(), index: usize, bytes: []const u8, raw: ?Value) !void {
        const message_snapshot = self.snapshot();
        switch (self.blocks.items[index]) {
            .text => {
                var event: @FieldType(t.AssistantMessageEvent, "text_delta") = .{ .contentIndex = @floatFromInt(index), .delta = bytes, .partial = message_snapshot };
                if (raw) |value| try j.retainField(self.allocator, &event, "delta", value);
                try self.stream.push(.{ .text_delta = event });
            },
            .thinking => {
                var event: @FieldType(t.AssistantMessageEvent, "thinking_delta") = .{ .contentIndex = @floatFromInt(index), .delta = bytes, .partial = message_snapshot };
                if (raw) |value| try j.retainField(self.allocator, &event, "delta", value);
                try self.stream.push(.{ .thinking_delta = event });
            },
            .toolCall => {
                var event: @FieldType(t.AssistantMessageEvent, "toolcall_delta") = .{ .contentIndex = @floatFromInt(index), .delta = bytes, .partial = message_snapshot };
                if (raw) |value| try j.retainField(self.allocator, &event, "delta", value);
                try self.stream.push(.{ .toolcall_delta = event });
            },
            else => unreachable,
        }
    }
    pub fn end(self: *@This(), index: usize) !void {
        return self.endWithOptions(index, false);
    }
    fn endWithOptions(self: *@This(), index: usize, preserve_existing_error: bool) !void {
        if (index >= self.blocks.items.len) return error.InvalidContentIndex;
        if (self.states.items[index].closed) return;
        const block = &self.blocks.items[index];
        if (block.* == .thinking and std.mem.eql(u8, self.model.api, "openai-completions") and self.reasoningDetails.items.len > 0) block.thinking.thinkingSignature = try @import("../json/value.zig").stringify(self.allocator, self.reasoningDetails.items, .{});
        if (block.* == .toolCall and self.states.items[index].customInput != null) {
            try self.appendCustomToolInput(index, self.customToolInput(index), true, false);
            self.states.items[index].argumentsComplete = true;
        } else if (block.* == .toolCall and !self.states.items[index].argumentsComplete) {
            const final = try partial.parseToolCallArguments(self.allocator, self.states.items[index].arguments.items);
            block.toolCall.arguments = final.arguments;
            if (!preserve_existing_error or final.argumentsError != null) block.toolCall.argumentsError = final.argumentsError;
            self.states.items[index].argumentsComplete = true;
        }
        if (std.mem.eql(u8, self.model.api, "mistral-conversations") and block.* == .toolCall and block.toolCall.jsonFields == .object) _ = block.toolCall.jsonFields.object.fetchOrderedRemove("partialArgs");
        self.states.items[index].closed = true;
        const message_snapshot = self.snapshot();
        switch (block.*) {
            .text => |v| try self.stream.push(.{ .text_end = .{ .contentIndex = @floatFromInt(index), .content = v.text, .partial = message_snapshot } }),
            .thinking => |v| try self.stream.push(.{ .thinking_end = .{ .contentIndex = @floatFromInt(index), .content = v.thinking, .partial = message_snapshot } }),
            .toolCall => |v| try self.stream.push(.{ .toolcall_end = .{ .contentIndex = @floatFromInt(index), .toolCall = .{ .toolCall = v }, .partial = message_snapshot } }),
            else => {},
        }
        if (self.current == index) self.current = null;
    }
    pub fn textSlot(self: *@This(), thinking: bool, close_previous: bool) !usize {
        if (self.current) |index| {
            if (!self.states.items[index].closed and (if (thinking) self.blocks.items[index] == .thinking else self.blocks.items[index] == .text)) return index;
            if (close_previous) try self.end(index);
        }
        return self.begin(if (thinking) .{ .thinking = .{ .thinking = "" } } else .{ .text = .{ .text = "" } });
    }
    pub fn finish(self: *@This()) !void {
        if (self.piTerminal) |kind| {
            if (kind == .done) try self.stream.push(.{ .done = .{ .reason = self.message.stopReason, .message = self.snapshot() } }) else try self.stream.push(.{ .@"error" = .{ .reason = self.message.stopReason, .@"error" = self.snapshot() } });
            return;
        }
        const bedrock = std.mem.eql(u8, self.model.api, "bedrock-converse-stream");
        if (bedrock) try @import("bedrock_converse_stream.zig").finalizeBlocks(self);
        if (std.mem.eql(u8, self.model.api, "mistral-conversations")) if (self.current) |index| try self.end(index);
        for (self.states.items, 0..) |state, index| {
            // Pi forwards explicit block events and returns on a terminal event.
            if (std.mem.eql(u8, self.model.api, "pi-messages") or std.mem.eql(u8, self.model.api, "anthropic-messages")) continue;
            if (bedrock) continue;
            const responses_api = std.mem.eql(u8, self.model.api, "openai-responses") or std.mem.eql(u8, self.model.api, "azure-openai-responses") or std.mem.eql(u8, self.model.api, "openai-codex-responses");
            if (!state.closed and responses_api) {
                if (self.blocks.items[index] == .toolCall and self.message.stopReason == .toolUse) {
                    const call = self.blocks.items[index].toolCall;
                    self.message.errorMessage = try std.fmt.allocPrint(self.allocator, "OpenAI Responses stream completed with an unfinished tool call: {s} ({s})", .{ call.name, call.id });
                    return error.UnfinishedResponsesToolCall;
                }
                continue;
            }
            try self.end(index);
        }
        if (!std.mem.eql(u8, self.model.api, "pi-messages")) self.io.checkCancel() catch {
            self.message.errorMessage = "Request was aborted";
            return error.Canceled;
        };
        // These adapters finish normal-EOF blocks before checking whether the
        // provider supplied a terminal reason. A thrown stream error goes
        // directly through fail(), preserving the source's event ordering.
        if (!self.terminal) {
            if (std.mem.eql(u8, self.model.api, "openai-completions") and !j.truthy(j.getNonNull(self.model.compat orelse .null, "supportsFinishReason") orelse .{ .bool = true })) self.message.stopReason = if (self.hasTools()) .toolUse else .stop else return error.StreamEndedWithoutTerminalEvent;
        }
        if (self.message.stopReason == .pending) {
            if (std.mem.eql(u8, self.model.api, "anthropic-messages")) {
                self.message.errorMessage = "Anthropic stream ended without a stop reason";
                return error.MissingStopReason;
            }
            self.message.stopReason = .stop;
        }
        if (self.message.stopReason == .@"error" or self.message.stopReason == .aborted) {
            // Google composes this error after reading and closing the final
            // block. A finishReason on an earlier chunk is still replaceable.
            if (std.mem.eql(u8, self.model.api, "google-generative-ai") or std.mem.eql(u8, self.model.api, "google-vertex")) {
                if (self.message.rawStopReason) |reason| if (reason.len > 0) {
                    self.message.errorMessage = try std.fmt.allocPrint(self.allocator, "Provider stopped with: {s}", .{reason});
                };
            }
            if ((self.message.errorMessage == null or (std.mem.eql(u8, self.model.api, "openai-responses") and self.message.errorMessage.?.len == 0)) and !std.mem.eql(u8, self.model.api, "pi-messages")) self.message.errorMessage = "An unknown error occurred";
            if (bedrock and self.message.stopReason != .aborted) try @import("bedrock_converse_stream.zig").appendFailureDiagnostic(self);
            try self.formatResponsesFailure();
            try self.stream.push(.{ .@"error" = .{ .reason = self.message.stopReason, .@"error" = self.snapshot() } });
            return;
        }
        if (self.inputTransformations) |transformations| if (transformations == .array and transformations.array.items.len > 0) {
            var values = j.array(self.allocator);
            for (transformations.array.items) |transformation| {
                var value = j.map(self.allocator);
                for ([_][]const u8{ "type", "path", "reason" }) |key| if (j.get(transformation, key)) |field| if (field != .null) try j.put(self.allocator, &value, key, field);
                try values.array.append(value);
            }
            var detail = j.map(self.allocator);
            try j.put(self.allocator, &detail, "transformations", values);
            try @import("../utils/diagnostics.zig").appendAssistantMessageDiagnostic(self.allocator, &self.message, .{ .type = "anthropic_input_transformations", .timestamp = @as(f64, @floatFromInt(std.Io.Clock.real.now(self.io).toMilliseconds())), .details = detail });
        };
        try self.stream.push(.{ .done = .{ .reason = self.message.stopReason, .message = self.snapshot() } });
    }
    pub fn fail(self: *@This(), err: anyerror) void {
        if (std.mem.eql(u8, self.model.api, "bedrock-converse-stream")) {
            @import("bedrock_converse_stream.zig").finalizeBlocks(self) catch |finalize_err| {
                self.stream.fail(finalize_err);
                return;
            };
            if (err != error.Canceled) @import("bedrock_converse_stream.zig").appendFailureDiagnostic(self) catch |diagnostic_err| {
                self.stream.fail(diagnostic_err);
                return;
            };
        }
        if (std.mem.eql(u8, self.model.api, "pi-messages")) {
            const message = self.message.errorMessage;
            const diagnostics = if (self.piAborted or err == error.Canceled) null else self.message.diagnostics;
            self.blocks.clearRetainingCapacity();
            self.states.clearRetainingCapacity();
            self.current = null;
            self.message = .{ .api = self.model.api, .provider = self.model.provider, .model = self.model.id, .timestamp = @as(f64, @floatFromInt(std.Io.Clock.real.now(self.io).toMilliseconds())), .errorMessage = message, .diagnostics = diagnostics };
        }
        if (std.mem.eql(u8, self.model.api, "mistral-conversations")) for (self.blocks.items) |*block| {
            if (block.* == .toolCall and block.toolCall.jsonFields == .object) _ = block.toolCall.jsonFields.object.fetchOrderedRemove("partialArgs");
        };
        if (self.completionThinking) |index| if (self.reasoningDetails.items.len > 0) {
            self.blocks.items[index].thinking.thinkingSignature = @import("../json/value.zig").stringify(self.allocator, self.reasoningDetails.items, .{}) catch self.blocks.items[index].thinking.thinkingSignature;
        };
        self.message.stopReason = if (err == error.Canceled or (self.piAborted and std.mem.eql(u8, self.model.api, "pi-messages")) or (self.mistralAborted and std.mem.eql(u8, self.model.api, "mistral-conversations"))) .aborted else .@"error";
        if (self.message.errorMessage == null) self.message.errorMessage = switch (err) {
            error.MissingAuthentication => (if (std.mem.eql(u8, self.model.api, "pi-messages")) std.fmt.allocPrint(self.allocator, "No API key provided for provider \"{s}\"", .{self.model.provider}) else std.fmt.allocPrint(self.allocator, "No API key for provider: {s}", .{self.model.provider})) catch @errorName(err),
            error.FailedToExtractAccountId => "Failed to extract accountId from token",
            error.GoogleEphemeralTokenUnsupported => "Ephemeral tokens are only supported by the live API.",
            error.GoogleProjectRequired => "Vertex AI requires a project ID. Set GOOGLE_CLOUD_PROJECT/GCLOUD_PROJECT or pass project in options.",
            error.GoogleLocationRequired => "Vertex AI requires a location. Set GOOGLE_CLOUD_LOCATION or pass location in options.",
            error.AzureBaseUrlRequired => "Azure OpenAI base URL is required. Set AZURE_OPENAI_BASE_URL or AZURE_OPENAI_RESOURCE_NAME, or pass azureBaseUrl, azureResourceName, or model.baseUrl.",
            error.GoogleRequestTimeout => "This operation was aborted",
            error.MidOutputAnthropicFallback => "Anthropic performed an unsupported mid-output model fallback",
            error.InvalidMistralStreamingEvent => "Invalid Mistral streaming event",
            error.IncompleteSseEvent => "Incomplete JSON segment at the end",
            error.UnexpectedBedrockMessageRole => "Unexpected assistant message start but got user message start instead",
            error.StreamEndedWithoutTerminalEvent => if (std.mem.eql(u8, self.model.api, "openai-completions")) "Stream ended without finish_reason" else if (std.mem.eql(u8, self.model.api, "mistral-conversations")) "Mistral stream ended without a finish reason" else if (std.mem.eql(u8, self.model.api, "anthropic-messages")) "Anthropic stream ended before message_stop" else if (std.mem.eql(u8, self.model.api, "bedrock-converse-stream")) "Bedrock stream ended without a stop reason" else if (std.mem.eql(u8, self.model.api, "google-generative-ai")) "Google stream ended without a finish reason" else if (std.mem.eql(u8, self.model.api, "google-vertex")) "Google Vertex stream ended without a finish reason" else if (std.mem.eql(u8, self.model.api, "pi-messages")) std.fmt.allocPrint(self.allocator, "{s} stream ended without a terminal event", .{self.model.provider}) catch @errorName(err) else "OpenAI Responses stream ended before a terminal response event",
            else => @errorName(err),
        };
        self.formatResponsesFailure() catch |format_err| {
            self.stream.fail(format_err);
            return;
        };
        self.stream.push(.{ .@"error" = .{ .reason = self.message.stopReason, .@"error" = self.snapshot() } }) catch {
            self.stream.fail(err);
        };
    }
    fn formatResponsesFailure(self: *@This()) !void {
        if (!std.mem.eql(u8, self.model.api, "openai-responses")) return;
        const message = self.message.errorMessage orelse return;
        // Streamed Errors have no SDK HTTP status/body. Already formatted HTTP
        // failures pass through unchanged, including their existing usage hint.
        self.message.errorMessage = try @import("sdk_http_errors.zig").openAIResponsesFailure(self.allocator, self.model, .{ .sdk = .{ .message = message } });
    }
    pub fn hasTools(self: *@This()) bool {
        for (self.blocks.items) |block| if (block == .toolCall) return true;
        return false;
    }
    fn customArguments(self: *@This(), property: []const u8, input: []const u8) !Value {
        var arguments = j.map(self.allocator);
        try j.put(self.allocator, &arguments, property, j.str(input));
        return arguments;
    }
    fn beginCustomTool(self: *@This(), id: []const u8, name: []const u8, namespace: ?[]const u8, input: []const u8) !usize {
        const property = self.grammarToolInputProperties.get(name) orelse "input";
        const index = try self.begin(.{ .toolCall = .{ .id = id, .name = name, .namespace = namespace, .arguments = try self.customArguments(property, input) } });
        self.states.items[index].customInput = .{ .property = property };
        return index;
    }
    fn customToolInput(self: *@This(), index: usize) []const u8 {
        const custom = self.states.items[index].customInput orelse return "";
        return j.field(self.blocks.items[index].toolCall.arguments, custom.property) orelse "";
    }
    fn pushToolCallDelta(self: *@This(), index: usize, bytes: []const u8) !void {
        try self.stream.push(.{ .toolcall_delta = .{ .contentIndex = @floatFromInt(index), .delta = bytes, .partial = self.snapshot() } });
    }
    fn appendCustomToolInput(self: *@This(), index: usize, next_input: []const u8, close: bool, emit_empty: bool) !void {
        const custom = if (self.states.items[index].customInput) |*value| value else {
            if (emit_empty) try self.pushToolCallDelta(index, "");
            return;
        };
        const delta_value = try constrained.appendGrammarToolInputJsonDeltaWithError(self.allocator, &custom.buffer, custom.property, next_input, close, &self.message.errorMessage);
        self.blocks.items[index].toolCall.arguments = try self.customArguments(custom.property, next_input);
        if (delta_value) |bytes| try self.pushToolCallDelta(index, bytes) else if (emit_empty) try self.pushToolCallDelta(index, "");
    }
    pub fn consume(self: *@This(), event: Value) !void {
        if (std.mem.eql(u8, self.model.api, "openai-completions")) return self.completions(event);
        if (std.mem.eql(u8, self.model.api, "openai-responses") or std.mem.eql(u8, self.model.api, "azure-openai-responses") or std.mem.eql(u8, self.model.api, "openai-codex-responses")) return self.responses(event);
        if (std.mem.eql(u8, self.model.api, "anthropic-messages")) return @import("reducer_anthropic.zig").consume(self, event);
        if (std.mem.eql(u8, self.model.api, "google-generative-ai") or std.mem.eql(u8, self.model.api, "google-vertex")) return @import("reducer_google.zig").consume(self, event);
        if (std.mem.eql(u8, self.model.api, "pi-messages")) return @import("pi_messages.zig").consume(self, event);
        if (std.mem.eql(u8, self.model.api, "mistral-conversations")) return @import("mistral_conversations.zig").consume(self, event);
        if (std.mem.eql(u8, self.model.api, "bedrock-converse-stream")) return @import("bedrock_converse_stream.zig").consume(self, event);
        return error.UnsupportedApi;
    }
    fn completions(self: *@This(), event: Value) !void {
        if (event != .object and event != .array) return;
        if (j.get(event, "error")) |err| if (j.truthy(err)) {
            self.message.errorMessage = try @import("sdk_http_errors.zig").openAICompletionsStreamFailure(self.allocator, err);
            return error.ProviderError;
        };
        if (!j.truthy((try j.fieldValue(self.allocator, self.message, "responseId")) orelse .null)) try self.setResponseId(j.get(event, "id"));
        if (j.field(event, "model")) |model| if (model.len > 0 and !std.mem.eql(u8, model, self.model.id) and !j.truthy((try j.fieldValue(self.allocator, self.message, "responseModel")) orelse .null)) {
            self.message.responseModel = model;
            j.markPresent(&self.message, "responseModel");
        };
        if (j.get(event, "usage")) |usage| if (j.truthy(usage)) {
            self.message.usage = try @import("openai_completions.zig").parseChunkUsage(self.allocator, usage, self.model);
        };
        const choices = c.array(event, "choices");
        if (choices.len == 0) return;
        const choice = choices[0];
        if (!j.truthy(j.get(event, "usage") orelse .null)) if (j.get(choice, "usage")) |usage| if (j.truthy(usage)) {
            self.message.usage = try @import("openai_completions.zig").parseChunkUsage(self.allocator, usage, self.model);
        };
        if (j.field(choice, "finish_reason")) |reason| if (reason.len > 0) {
            self.terminal = true;
            self.message.rawStopReason = reason;
            self.message.stopReason = @import("openai_completions.zig").mapStopReason(reason);
            if (self.message.stopReason == .@"error") {
                self.message.errorMessage = try std.fmt.allocPrint(self.allocator, "Provider finish_reason: {s}", .{reason});
                self.finishReasonMessage = self.message.errorMessage;
            }
        };
        const delta_value = j.get(choice, "delta") orelse return;
        for ([_][]const u8{ "reasoning_content", "reasoning", "reasoning_text" }) |field| if (j.field(delta_value, field)) |bytes| if (bytes.len > 0) {
            const signature = if (c.is(self.model, "opencode-go") and std.mem.eql(u8, field, "reasoning")) "reasoning_content" else field;
            const index = try self.completionTextSlot(true, signature);
            try self.delta(index, bytes);
            break;
        };
        for (c.array(delta_value, "reasoning_details")) |detail| if (@import("openai_reasoning_details.zig").isOpenAIReasoningDetail(detail)) {
            _ = try self.completionTextSlot(true, "");
            try @import("openai_reasoning_details.zig").appendOpenAIReasoningDetail(self.allocator, &self.reasoningDetails, detail);
        };
        if (j.field(delta_value, "content")) |bytes| if (bytes.len > 0) try self.delta(try self.completionTextSlot(false, ""), bytes);
        for (c.array(delta_value, "tool_calls")) |call| {
            const wire_index: ?u64 = if (j.number(j.get(call, "index") orelse .null)) |number| c.numberMapKey(number) else null;
            const function = j.get(call, "function") orelse .null;
            const custom = j.get(call, "custom") orelse .null;
            const is_custom = j.truthy(custom) and !j.truthy(function);
            const name = j.field(function, "name") orelse j.field(custom, "name") orelse "";
            var found = if (wire_index) |slot| self.slots.get(slot) else null;
            if (found == null) if (j.field(call, "id")) |id| {
                found = self.completionToolIds.get(id);
            };
            const index = found orelse blk: {
                const created = if (is_custom) try self.beginCustomTool(j.field(call, "id") orelse "", name, null, "") else try self.begin(.{ .toolCall = .{ .id = j.field(call, "id") orelse "", .name = name, .arguments = j.map(self.allocator) } });
                break :blk created;
            };
            if (wire_index) |slot| if (self.states.items[index].completionStreamIndex == null) {
                self.states.items[index].completionStreamIndex = slot;
                try self.slots.put(self.allocator, slot, index);
            };
            if (j.field(call, "id")) |id| if (id.len > 0) try self.completionToolIds.put(self.allocator, id, index);
            const tool = &self.blocks.items[index].toolCall;
            if (tool.id.len == 0) if (j.field(call, "id")) |id| {
                tool.id = id;
            };
            if (tool.name.len == 0 and name.len > 0) tool.name = name;
            if (is_custom and self.states.items[index].customInput == null) {
                const property = self.grammarToolInputProperties.get(tool.name) orelse "input";
                tool.arguments = try self.customArguments(property, "");
                self.states.items[index].customInput = .{ .property = property };
                self.states.items[index].arguments.clearRetainingCapacity();
            }
            if (j.field(function, "arguments")) |bytes| if (bytes.len > 0) {
                try self.delta(index, bytes);
                continue;
            };
            if (j.field(custom, "input")) |bytes| if (bytes.len > 0) {
                const next_input = try std.mem.concat(self.allocator, u8, &.{ self.customToolInput(index), bytes });
                try self.appendCustomToolInput(index, next_input, false, true);
                continue;
            };
            try self.pushToolCallDelta(index, "");
        }
    }
    fn completionTextSlot(self: *@This(), thinking: bool, signature: []const u8) !usize {
        const slot = if (thinking) &self.completionThinking else &self.completionText;
        if (slot.*) |index| return index;
        const index = try self.begin(if (thinking) .{ .thinking = .{ .thinking = "", .thinkingSignature = signature } } else .{ .text = .{ .text = "" } });
        slot.* = index;
        return index;
    }
    fn responseNamespace(self: *@This(), tool: *t.ToolCall, item: Value) !void {
        if (j.get(item, "namespace")) |namespace| {
            if (j.text(namespace)) |text| {
                tool.namespace = text;
                j.markPresent(tool, "namespace");
            } else try j.retainField(self.allocator, tool, "namespace", namespace);
        }
    }
    fn beginResponseTool(self: *@This(), item: Value, custom: bool) !usize {
        const name = j.field(item, "name") orelse "";
        const property = self.grammarToolInputProperties.get(name) orelse "input";
        var tool = t.ToolCall{
            .id = try std.fmt.allocPrint(self.allocator, "{s}|{s}", .{ j.field(item, "call_id") orelse "", j.field(item, "id") orelse "" }),
            .name = name,
            .arguments = if (custom) try self.customArguments(property, j.field(item, "input") orelse "") else j.map(self.allocator),
        };
        try self.responseNamespace(&tool, item);
        const index = try self.begin(.{ .toolCall = tool });
        if (custom) self.states.items[index].customInput = .{ .property = property };
        return index;
    }
    fn setResponseId(self: *@This(), value: ?Value) !void {
        if (value) |id| {
            if (j.text(id)) |text| {
                self.message.responseId = text;
                j.markPresent(&self.message, "responseId");
            } else try j.retainField(self.allocator, &self.message, "responseId", id);
        } else j.markAbsent(&self.message, "responseId");
    }
    fn setResponseStopReason(self: *@This(), value: ?Value) !void {
        if (value) |reason| {
            if (j.text(reason)) |text| {
                self.message.rawStopReason = text;
                j.markPresent(&self.message, "rawStopReason");
            } else try j.retainField(self.allocator, &self.message, "rawStopReason", reason);
        } else j.markAbsent(&self.message, "rawStopReason");
    }
    fn responseReceiver(self: *@This(), event: Value, property: []const u8) !Value {
        const response = j.get(event, "response");
        if (response == null or response.? == .null) {
            self.message.errorMessage = try std.fmt.allocPrint(self.allocator, "Cannot read properties of {s} (reading '{s}')", .{ if (response == null) "undefined" else "null", property });
            return error.InvalidResponsesEvent;
        }
        return response.?;
    }
    fn createResponseSlot(self: *@This(), output_index: u64, item: Value) !?usize {
        const kind = j.field(item, "type") orelse return null;
        self.applyResponsePhase(item);
        const index = if (std.mem.eql(u8, kind, "reasoning")) try self.begin(.{ .thinking = .{ .thinking = "" } }) else if (std.mem.eql(u8, kind, "message")) try self.begin(.{ .text = .{ .text = "" } }) else if (std.mem.eql(u8, kind, "function_call")) try self.beginResponseTool(item, false) else if (std.mem.eql(u8, kind, "custom_tool_call")) try self.beginResponseTool(item, true) else return null;
        self.states.items[index].item_id = j.field(item, "id");
        if (self.blocks.items[index] == .toolCall and self.states.items[index].customInput == null) if (j.field(item, "arguments")) |bytes| try self.states.items[index].arguments.appendSlice(self.allocator, bytes);
        try self.slots.put(self.allocator, output_index, index);
        return index;
    }
    fn applyResponsePhase(self: *@This(), item: Value) void {
        if (std.mem.eql(u8, j.field(item, "type") orelse "", "message") and std.mem.eql(u8, j.field(item, "phase") orelse "", "final_answer")) self.message.stopReason = .stop;
    }
    fn responses(self: *@This(), event: Value) !void {
        if (event == .null) {
            self.message.errorMessage = "Cannot read properties of null (reading 'type')";
            return error.InvalidResponsesEvent;
        }
        const kind = j.field(event, "type") orelse return;
        const output_index = c.numericIndex(j.get(event, "output_index"));
        if (std.mem.eql(u8, kind, "response.created")) {
            try self.setResponseId(j.get(try self.responseReceiver(event, "id"), "id"));
            return;
        }
        if (std.mem.eql(u8, kind, "response.output_item.added")) {
            _ = try self.createResponseSlot(output_index, j.get(event, "item") orelse return error.MissingResponseItem);
            return;
        }
        if (std.mem.eql(u8, kind, "response.custom_tool_call_input.delta") or std.mem.eql(u8, kind, "response.custom_tool_call_input.done")) {
            const index = self.slots.get(output_index) orelse return;
            if (self.blocks.items[index] != .toolCall or self.states.items[index].customInput == null) return;
            const close = std.mem.eql(u8, kind, "response.custom_tool_call_input.done");
            const next_input = if (close) j.field(event, "input") orelse "" else try std.mem.concat(self.allocator, u8, &.{ self.customToolInput(index), j.field(event, "delta") orelse "" });
            try self.appendCustomToolInput(index, next_input, close, false);
            return;
        }
        if (std.mem.eql(u8, kind, "response.reasoning_summary_text.delta") or std.mem.eql(u8, kind, "response.reasoning_text.delta") or std.mem.eql(u8, kind, "response.output_text.delta") or std.mem.eql(u8, kind, "response.refusal.delta") or std.mem.eql(u8, kind, "response.function_call_arguments.delta")) {
            const index = self.slots.get(output_index) orelse return;
            if (std.mem.eql(u8, kind, "response.function_call_arguments.delta") and self.states.items[index].customInput != null) return;
            const expected = if (std.mem.eql(u8, kind, "response.function_call_arguments.delta")) self.blocks.items[index] == .toolCall else if (std.mem.eql(u8, kind, "response.output_text.delta") or std.mem.eql(u8, kind, "response.refusal.delta")) self.blocks.items[index] == .text else self.blocks.items[index] == .thinking;
            if (expected) try self.delta(index, j.field(event, "delta") orelse "");
            return;
        }
        if (std.mem.eql(u8, kind, "response.reasoning_summary_part.done")) {
            if (self.slots.get(output_index)) |index| if (self.blocks.items[index] == .thinking) try self.delta(index, "\n\n");
            return;
        }
        if (std.mem.eql(u8, kind, "response.function_call_arguments.done")) {
            const index = self.slots.get(output_index) orelse return;
            const arguments = j.field(event, "arguments") orelse "";
            if (self.blocks.items[index] != .toolCall or self.states.items[index].customInput != null) return;
            const previous = self.states.items[index].arguments.items;
            if (std.mem.startsWith(u8, arguments, previous) and arguments.len > previous.len) try self.delta(index, arguments[previous.len..]);
            self.states.items[index].arguments.clearRetainingCapacity();
            try self.states.items[index].arguments.appendSlice(self.allocator, arguments);
            self.blocks.items[index].toolCall.arguments = try partial.parseStreamingJson(self.allocator, arguments);
            return;
        }
        if (std.mem.eql(u8, kind, "response.output_item.done")) {
            const item = j.get(event, "item") orelse return error.MissingResponseItem;
            self.applyResponsePhase(item);
            const index = self.slots.get(output_index) orelse ((try self.createResponseSlot(output_index, item)) orelse return);
            const block = &self.blocks.items[index];
            const item_type = j.field(item, "type") orelse "";
            if (!switch (block.*) {
                .thinking => std.mem.eql(u8, item_type, "reasoning"),
                .text => std.mem.eql(u8, item_type, "message"),
                .toolCall => if (self.states.items[index].customInput != null) std.mem.eql(u8, item_type, "custom_tool_call") else std.mem.eql(u8, item_type, "function_call"),
                else => false,
            }) return;
            switch (block.*) {
                .thinking => |*thinking| {
                    var parts: std.ArrayList([]const u8) = .empty;
                    for (c.array(item, "summary")) |part| try parts.append(self.allocator, j.field(part, "text") orelse "");
                    var text = try std.mem.join(self.allocator, "\n\n", parts.items);
                    if (text.len == 0) {
                        parts.clearRetainingCapacity();
                        for (c.array(item, "content")) |part| try parts.append(self.allocator, j.field(part, "text") orelse "");
                        text = try std.mem.join(self.allocator, "\n\n", parts.items);
                    }
                    if (text.len > 0) thinking.thinking = text;
                    thinking.thinkingSignature = try @import("../json/value.zig").stringify(self.allocator, item, .{});
                    if (j.field(item, "id")) |id| try self.responseReasoningIds.put(self.allocator, id, index);
                },
                .text => |*text| {
                    var parts: std.ArrayList([]const u8) = .empty;
                    for (c.array(item, "content")) |part| try parts.append(self.allocator, j.field(part, if (std.mem.eql(u8, j.field(part, "type") orelse "", "output_text")) "text" else "refusal") orelse "");
                    text.text = try std.mem.join(self.allocator, "", parts.items);
                    var signature = try j.from(self.allocator, .{ .v = 1 });
                    if (j.get(item, "id")) |id| try j.put(self.allocator, &signature, "id", id);
                    if (j.get(item, "phase")) |phase| if (j.truthy(phase)) {
                        try j.put(self.allocator, &signature, "phase", phase);
                    };
                    text.textSignature = try j.stringify(self.allocator, signature, .{});
                },
                .toolCall => |*call| {
                    if (self.states.items[index].customInput != null) {
                        try self.appendCustomToolInput(index, j.field(item, "input") orelse self.customToolInput(index), true, false);
                    } else if (j.field(item, "arguments")) |arguments| if (arguments.len > 0) {
                        self.states.items[index].arguments.clearRetainingCapacity();
                        try self.states.items[index].arguments.appendSlice(self.allocator, arguments);
                    };
                    try self.responseNamespace(call, item);
                },
                else => {},
            }
            try self.end(index);
            _ = self.slots.remove(output_index);
            return;
        }
        if (std.mem.eql(u8, kind, "response.failed")) {
            self.terminal = true;
            const response = j.get(event, "response") orelse .null;
            try self.setResponseStopReason(j.get(response, "status"));
            const failure = j.get(response, "error") orelse .null;
            const reason = j.get(j.get(response, "incomplete_details") orelse .null, "reason") orelse .null;
            const code = j.get(failure, "code") orelse .null;
            const message = j.get(failure, "message") orelse .null;
            const describe = @import("../utils/diagnostics.zig").formatThrownValue;
            self.message.errorMessage = if (j.truthy(failure)) try std.fmt.allocPrint(self.allocator, "{s}: {s}", .{ if (j.truthy(code)) try describe(self.allocator, .{ .value = code }) else "unknown", if (j.truthy(message)) try describe(self.allocator, .{ .value = message }) else "no message" }) else if (j.truthy(reason)) try std.fmt.allocPrint(self.allocator, "incomplete: {s}", .{try describe(self.allocator, .{ .value = reason })}) else "Unknown error (no error details in response)";
            return error.ProviderError;
        }
        if (std.mem.eql(u8, kind, "response.completed") or std.mem.eql(u8, kind, "response.incomplete")) {
            self.terminal = true;
            const response = try self.responseReceiver(event, "output");
            if (j.get(response, "id")) |id| if (j.truthy(id)) {
                try self.setResponseId(id);
            };
            for (c.array(response, "output")) |item| {
                if (!std.mem.eql(u8, j.field(item, "type") orelse "", "reasoning")) continue;
                const encrypted = j.field(item, "encrypted_content") orelse continue;
                if (encrypted.len == 0) continue;
                const id = j.field(item, "id") orelse continue;
                const index = self.responseReasoningIds.get(id) orelse continue;
                const signature = self.blocks.items[index].thinking.thinkingSignature orelse continue;
                var stored = try j.parseFromSliceLeaky(Value, self.allocator, signature, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always });
                const existing = j.field(stored, "encrypted_content");
                if (existing == null or existing.?.len == 0) {
                    try j.put(self.allocator, &stored, "encrypted_content", j.str(encrypted));
                    self.blocks.items[index].thinking.thinkingSignature = try @import("../json/value.zig").stringify(self.allocator, stored, .{});
                }
            }
            if (j.get(response, "usage")) |raw| if (j.truthy(raw)) {
                const accounting = @import("../utils/usage.zig");
                const a = self.allocator;
                const input_details = j.get(raw, "input_tokens_details") orelse .null;
                const reads = accounting.orZero(input_details, "cached_tokens");
                const writes = accounting.orZero(input_details, "cache_write_tokens");
                self.message.usage = .{ .input = @import("../utils/numbers.zig").maximum(0, try accounting.number(a, accounting.orZero(raw, "input_tokens")) - try accounting.number(a, reads) - try accounting.number(a, writes)) };
                try accounting.set(a, &self.message.usage, "output", accounting.orZero(raw, "output_tokens"));
                try accounting.set(a, &self.message.usage, "cacheRead", reads);
                try accounting.set(a, &self.message.usage, "cacheWrite", writes);
                try accounting.set(a, &self.message.usage, "reasoning", accounting.orZero(j.get(raw, "output_tokens_details") orelse .null, "reasoning_tokens"));
                try accounting.set(a, &self.message.usage, "totalTokens", accounting.orZero(raw, "total_tokens"));
            };
            _ = try @import("../utils/cost.zig").calculateCost(self.allocator, self.model, &self.message.usage);
            if (self.responsePricing) |pricing| {
                const response_tier = j.field(response, "service_tier");
                const tier = if (pricing.resolve) |resolve| try resolve(pricing.context, response_tier, pricing.serviceTier) else response_tier orelse pricing.serviceTier;
                try pricing.apply(pricing.context, &self.message.usage, tier);
            }
            const status_value = j.get(response, "status");
            const raw_status = if (status_value) |value| j.text(value) else null;
            const status = raw_status orelse "";
            const reason = j.field(j.get(response, "incomplete_details") orelse .null, "reason");
            if (reason != null and reason.?.len > 0) {
                const description = try @import("../utils/diagnostics.zig").formatThrownValue(self.allocator, if (status_value) |value| .{ .value = value } else .undefined_value);
                self.message.rawStopReason = try std.fmt.allocPrint(self.allocator, "{s}.{s}", .{ description, reason.? });
                j.markPresent(&self.message, "rawStopReason");
            } else try self.setResponseStopReason(status_value);
            self.message.errorMessage = null;
            if (!j.truthy(status_value orelse .null) or std.mem.eql(u8, status, "completed") or std.mem.eql(u8, status, "queued") or std.mem.eql(u8, status, "in_progress")) self.message.stopReason = if (self.hasTools()) .toolUse else .stop else if (std.mem.eql(u8, status, "incomplete")) {
                if (reason != null and std.mem.eql(u8, reason.?, "max_output_tokens")) self.message.stopReason = .length else {
                    self.message.stopReason = .@"error";
                    self.message.errorMessage = if (reason != null and reason.?.len > 0) try std.fmt.allocPrint(self.allocator, "Response incomplete: {s}", .{reason.?}) else "Response incomplete without a provider reason";
                    self.finishReasonMessage = self.message.errorMessage;
                }
            } else if (std.mem.eql(u8, status, "failed") or std.mem.eql(u8, status, "cancelled")) self.message.stopReason = .@"error" else {
                self.message.errorMessage = try std.fmt.allocPrint(self.allocator, "Unhandled stop reason: {s}", .{try @import("../utils/diagnostics.zig").formatThrownValue(self.allocator, .{ .value = status_value.? })});
                return error.InvalidStopReason;
            }
            return;
        }
        if (std.mem.eql(u8, kind, "error")) {
            const describe = @import("../utils/diagnostics.zig").formatThrownValue;
            self.message.errorMessage = try std.fmt.allocPrint(self.allocator, "Error Code {s}: {s}", .{ try describe(self.allocator, if (j.get(event, "code")) |value| .{ .value = value } else .undefined_value), try describe(self.allocator, if (j.get(event, "message")) |value| .{ .value = value } else .undefined_value) });
            return error.ProviderError;
        }
    }
};
