const std = @import("std");
const j = @import("json/value.zig");
pub const JsonValue = std.json.Value;
pub const JsonObject = std.json.ObjectMap;
pub const KnownApi = enum { @"openai-completions", @"mistral-conversations", @"openai-responses", @"azure-openai-responses", @"openai-codex-responses", @"anthropic-messages", @"bedrock-converse-stream", @"google-generative-ai", @"google-vertex", @"pi-messages" };
pub const KnownImageApi = enum { @"openrouter-images" };
pub const KnownClassifierApi = enum { @"typesafe-system-one", @"cloudflare-workers-ai-system-one", @"llama-cpp-classify", @"openai-decisions" };
pub const KnownProvider = enum { @"amazon-bedrock", @"ant-ling", anthropic, google, @"google-vertex", openai, azure, @"openai-codex", radius, typesafe, nvidia, deepseek, @"github-copilot", xai, groq, cerebras, openrouter, @"vercel-ai-gateway", zai, @"zai-coding-cn", mistral, minimax, @"minimax-cn", moonshotai, @"moonshotai-cn", huggingface, fireworks, together, baseten, opencode, @"opencode-go", @"kimi-coding", meta, @"cloudflare-workers-ai", @"cloudflare-ai-gateway", @"qwen-token-plan", @"qwen-token-plan-cn", @"qwen-token-plan-individual", xiaomi, @"xiaomi-token-plan-cn", @"xiaomi-token-plan-ams", @"xiaomi-token-plan-sgp" };
pub const Api = []const u8;
pub const ImageApi = Api;
pub const ClassifierApi = Api;
pub const ProviderId = []const u8;
pub const ThinkingLevel = enum { off, minimal, low, medium, high, xhigh, max };
pub const ModelThinkingLevel = ThinkingLevel;
pub const StopReason = enum { pending, stop, length, toolUse, @"error", aborted, deferred };
pub const ModelType = enum { chat, image, classifier, unknown };
pub const CacheRetention = enum { none, short, long };
pub const Transport = enum { sse, websocket, @"websocket-cached", auto };
pub const ToolChoice = enum { auto, none, any, required };
pub const ProviderHeader = struct { name: []const u8, value: ?[]const u8 };
pub const ProviderHeaders = []const ProviderHeader;
pub const ProviderEnv = std.StringArrayHashMapUnmanaged([]const u8);
/// Owns only diagnostic copies explicitly placed here. An error-message slot
/// remains a borrowed view and may also point to a literal or another owner.
pub const ErrorMessageStorage = struct {
    allocator: std.mem.Allocator,
    bytes: ?[]u8 = null,
    pub fn copy(self: *@This(), text: []const u8) ![]const u8 {
        const retained = try self.allocator.dupe(u8, text);
        if (self.bytes) |previous| self.allocator.free(previous);
        self.bytes = retained;
        return retained;
    }
    pub fn deinit(self: *@This()) void {
        if (self.bytes) |bytes| self.allocator.free(bytes);
        self.bytes = null;
    }
};
pub const ModelCostRates = struct {
    input: f64 = 0,
    output: f64 = 0,
    cacheRead: f64 = 0,
    cacheWrite: f64 = 0,
    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const ModelCostTier = struct {
    inputTokensAbove: f64,
    input: f64,
    output: f64,
    cacheRead: f64,
    cacheWrite: f64,
    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const ModelCost = struct {
    input: f64 = 0,
    output: f64 = 0,
    cacheRead: f64 = 0,
    cacheWrite: f64 = 0,
    tiers: ?[]const ModelCostTier = null,
    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const UsageCost = struct {
    input: f64 = 0,
    output: f64 = 0,
    cacheRead: f64 = 0,
    cacheWrite: f64 = 0,
    total: f64 = 0,
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const Usage = struct {
    input: f64 = 0,
    output: f64 = 0,
    cacheRead: f64 = 0,
    cacheWrite: f64 = 0,
    cacheWrite1h: ?f64 = null,
    reasoning: ?f64 = null,
    totalTokens: f64 = 0,
    cost: UsageCost = .{},

    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const TextContent = struct {
    type: enum { text } = .text,
    text: []const u8,
    textSignature: ?[]const u8 = null,
    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const ThinkingContent = struct {
    type: enum { thinking } = .thinking,
    thinking: []const u8,
    thinkingSignature: ?[]const u8 = null,
    redacted: ?bool = null,
    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const ImageContent = struct {
    type: enum { image } = .image,
    data: []const u8,
    mimeType: []const u8,
    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const ToolCall = struct {
    type: enum { toolCall } = .toolCall,
    id: []const u8,
    name: []const u8,
    arguments: JsonValue,
    thoughtSignature: ?[]const u8 = null,
    namespace: ?[]const u8 = null,
    argumentsError: ?[]const u8 = null,
    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const Content = union(enum) {
    text: TextContent,
    thinking: ThinkingContent,
    image: ImageContent,
    toolCall: ToolCall,
    /// Preserve unrecognized blocks from untyped/older transcripts as trees.
    /// Each provider retains its own treatment of unsupported block kinds.
    unknown: JsonValue,
    pub const jsonDiscriminator = "type";
    pub fn jsonToValue(self: @This(), a: std.mem.Allocator, options: std.json.Stringify.Options) !JsonValue {
        return switch (self) {
            .unknown => |value| value,
            inline else => |value| try j.toValue(a, value, options),
        };
    }
    pub fn jsonStringify(self: @This(), writer: *std.json.Stringify) !void {
        switch (self) {
            inline else => |value| try j.writeCompatible(writer, value),
        }
    }
    pub fn jsonParse(a: std.mem.Allocator, source: anytype, options: std.json.ParseOptions) !@This() {
        var configured = options;
        configured.duplicate_field_behavior = .use_last;
        configured.parse_numbers = false;
        var value = try JsonValue.jsonParse(a, source, configured);
        try j.normalizeParsed(&value);
        return jsonParseFromValue(a, value, options);
    }
    pub fn jsonParseFromValue(a: std.mem.Allocator, value: JsonValue, options: std.json.ParseOptions) !@This() {
        const tag = j.field(value, "type") orelse return .{ .unknown = value };
        for ([_][]const u8{ "text", "thinking", "image", "toolCall" }) |known| {
            if (std.mem.eql(u8, tag, known)) return j.taggedFromValue(@This(), "type", a, value, options) catch |err| {
                if (err == error.OutOfMemory) return err;
                return .{ .unknown = value };
            };
        }
        return .{ .unknown = value };
    }
};
pub const MessageContent = union(enum) {
    string: []const u8,
    blocks: []const Content,
    missing,
    nullContent,
    pub const jsonTransparent = true;
    pub const jsonAcceptsNull = true;
    pub fn jsonIsUndefined(self: @This()) bool {
        return self == .missing;
    }
    pub fn jsonToValue(self: @This(), a: std.mem.Allocator, options: std.json.Stringify.Options) !JsonValue {
        return switch (self) {
            .missing, .nullContent => .null,
            .string => |text| j.str(text),
            .blocks => |values| try j.toValue(a, values, options),
        };
    }
    pub fn jsonStringify(self: @This(), writer: *std.json.Stringify) !void {
        switch (self) {
            .missing, .nullContent => try writer.write(null),
            .string => |text| try j.writeCompatible(writer, text),
            .blocks => |values| try j.writeCompatible(writer, values),
        }
    }
    pub fn jsonParse(a: std.mem.Allocator, source: anytype, options: std.json.ParseOptions) !@This() {
        var configured = options;
        configured.duplicate_field_behavior = .use_last;
        configured.parse_numbers = false;
        var value = try JsonValue.jsonParse(a, source, configured);
        try j.normalizeParsed(&value);
        return jsonParseFromValue(a, value, options);
    }
    pub fn jsonParseFromValue(a: std.mem.Allocator, value: JsonValue, options: std.json.ParseOptions) !@This() {
        return switch (value) {
            .null => .nullContent,
            .string => |text| .{ .string = text },
            .array => .{ .blocks = try std.json.innerParseFromValue([]const Content, a, value, options) },
            else => error.UnexpectedToken,
        };
    }
};
pub const ConstrainedSamplingConfig = union(enum) {
    json_schema: struct {
        strict: enum { prefer, require },
        jsonFields: JsonValue = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    grammar: struct {
        variants: JsonValue,
        jsonFields: JsonValue = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    pub const jsonDiscriminator = "type";
    pub const jsonStringify = j.Tagged(@This(), "type").jsonStringify;
    pub const jsonParse = j.Tagged(@This(), "type").jsonParse;
    pub const jsonParseFromValue = j.Tagged(@This(), "type").jsonParseFromValue;
};
pub const Tool = struct {
    name: []const u8,
    description: []const u8,
    parameters: @import("schema/type.zig").TSchema,
    constrainedSampling: ?JsonValue = null,
    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const ToolReference = struct {
    name: []const u8,
    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const SystemMessage = struct {
    role: enum { system } = .system,
    content: MessageContent = .missing,
    sections: ?JsonValue = null,
    toolsAdded: ?[]const Tool = null,
    toolsRemoved: ?[]const ToolReference = null,
    timestamp: f64,
    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const UserMessage = struct {
    role: enum { user } = .user,
    content: MessageContent = .missing,
    timestamp: f64,
    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const DeferredHandle = struct {
    provider: []const u8,
    modelId: []const u8,
    api: []const u8,
    id: []const u8,
    expiresAt: ?f64 = null,
    pollAfterMs: ?f64 = null,
    data: ?JsonValue = null,
    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const Diagnostic = struct {
    type: []const u8,
    message: ?[]const u8 = null,
    data: ?JsonValue = null,
    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const AssistantMessage = struct {
    role: enum { assistant } = .assistant,
    content: []const Content = &.{},
    api: Api,
    provider: ProviderId,
    model: []const u8,
    responseModel: ?[]const u8 = null,
    responseId: ?[]const u8 = null,
    providerThinkingLevel: ?[]const u8 = null,
    thinkingLevel: ?ThinkingLevel = null,
    diagnostics: ?[]const JsonValue = null,
    usage: Usage = .{},
    stopReason: StopReason = .pending,
    deferred: ?DeferredHandle = null,
    errorMessage: ?[]const u8 = null,
    rawStopReason: ?[]const u8 = null,
    endTurn: ?bool = null,
    timestamp: f64,
    durationMs: ?f64 = null,
    jsonContentProperties: JsonValue = .null,

    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const NestedToolCallRecord = struct {
    id: []const u8,
    name: []const u8,
    arguments: ?JsonValue = null,
    argumentsBytes: ?f64 = null,
    status: enum { ok, @"error", unfinished },
    durationMs: ?f64 = null,
    @"error": ?[]const u8 = null,
    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const NestedToolCalls = struct {
    calls: []const NestedToolCallRecord,
    complete: bool,
    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const ToolResultMessage = struct {
    role: enum { toolResult } = .toolResult,
    toolCallId: []const u8,
    toolName: []const u8,
    content: []const Content = &.{},
    details: ?JsonValue = null,
    usage: ?Usage = null,
    nestedCalls: ?NestedToolCalls = null,
    isError: bool,
    timestamp: f64,
    durationMs: ?f64 = null,
    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const Message = union(enum) {
    system: SystemMessage,
    user: UserMessage,
    assistant: AssistantMessage,
    toolResult: ToolResultMessage,
    pub const jsonDiscriminator = "role";
    pub const jsonStringify = j.Tagged(@This(), "role").jsonStringify;
    pub fn jsonParse(a: std.mem.Allocator, source: anytype, options: std.json.ParseOptions) !@This() {
        var configured = options;
        configured.duplicate_field_behavior = .use_last;
        configured.parse_numbers = false;
        var value = try JsonValue.jsonParse(a, source, configured);
        try j.normalizeParsed(&value);
        return jsonParseFromValue(a, value, options);
    }
    pub fn jsonParseFromValue(a: std.mem.Allocator, value: JsonValue, options: std.json.ParseOptions) !@This() {
        return j.taggedFromValue(@This(), "role", a, value, options);
    }
};
pub const Context = struct {
    systemPrompt: ?[]const u8 = null,
    messages: []const Message,
    tools: ?[]const Tool = null,
    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const TranscriptContext = struct {
    messages: []const Message,
    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const AssistantMessageEvent = union(enum) {
    start: struct {
        partial: AssistantMessage,
        jsonFields: JsonValue = .null,
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
        partial: AssistantMessage,
        jsonFields: JsonValue = .null,
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
        partial: AssistantMessage,
        jsonFields: JsonValue = .null,
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
        partial: AssistantMessage,
        contentSignature: ?[]const u8 = null,
        jsonFields: JsonValue = .null,
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
        partial: AssistantMessage,
        jsonFields: JsonValue = .null,
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
        partial: AssistantMessage,
        jsonFields: JsonValue = .null,
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
        partial: AssistantMessage,
        contentSignature: ?[]const u8 = null,
        redacted: ?bool = null,
        jsonFields: JsonValue = .null,
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
        partial: AssistantMessage,
        id: ?[]const u8 = null,
        toolName: ?[]const u8 = null,
        jsonFields: JsonValue = .null,
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
        partial: AssistantMessage,
        jsonFields: JsonValue = .null,
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
        toolCall: Content,
        partial: AssistantMessage,
        jsonFields: JsonValue = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    done: struct {
        reason: StopReason,
        message: AssistantMessage,
        jsonFields: JsonValue = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    @"error": struct {
        reason: StopReason,
        @"error": AssistantMessage,
        jsonFields: JsonValue = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    unknown: struct {
        partial: AssistantMessage,
        jsonFields: JsonValue = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    pub const jsonDiscriminator = "type";
    pub fn jsonToValue(self: @This(), a: std.mem.Allocator, options: std.json.Stringify.Options) !JsonValue {
        return switch (self) {
            .unknown => |value| try j.Record(@TypeOf(value)).jsonToValue(value, a, options),
            else => try j.taggedToValue(self, "type", a, options),
        };
    }
    pub fn jsonStringify(self: @This(), writer: *std.json.Stringify) !void {
        switch (self) {
            .unknown => |value| try writer.write(value),
            else => try j.taggedStringify(self, "type", writer),
        }
    }
    pub fn jsonParse(a: std.mem.Allocator, source: anytype, options: std.json.ParseOptions) !@This() {
        var configured = options;
        configured.duplicate_field_behavior = .use_last;
        configured.parse_numbers = false;
        var value = try JsonValue.jsonParse(a, source, configured);
        try j.normalizeParsed(&value);
        return jsonParseFromValue(a, value, options);
    }
    pub fn jsonParseFromValue(a: std.mem.Allocator, value: JsonValue, options: std.json.ParseOptions) !@This() {
        const tag = j.field(value, "type") orelse "";
        inline for (@typeInfo(@This()).@"union".field_names, @typeInfo(@This()).@"union".field_types) |name, Field| {
            if (comptime std.mem.eql(u8, name, "unknown")) continue;
            if (std.mem.eql(u8, tag, name)) return @unionInit(@This(), name, try std.json.innerParseFromValue(Field, a, value, options));
        }
        return .{ .unknown = try std.json.innerParseFromValue(@FieldType(@This(), "unknown"), a, value, options) };
    }
};
pub const ModelModality = enum { text, image };
const ModelFields = struct {
    id: []const u8,
    name: []const u8,
    api: Api,
    provider: ProviderId,
    baseUrl: []const u8,
    type: ?JsonValue = null,
    input: []const ModelModality = &.{.text},
    inputLimits: ?JsonValue = null,
    cost: ModelCost = .{},
    headers: ?JsonValue = null,
    reasoning: bool = false,
    thinkingLevelMap: ?JsonValue = null,
    promptCache: ?JsonValue = null,
    contextWindow: f64 = 0,
    maxTokens: f64 = 0,
    samplingParams: ?JsonValue = null,
    samplingParamsByThinkingLevel: ?JsonValue = null,
    compat: ?JsonValue = null,
    output: ?[]const ModelModality = null,
    extra: JsonValue = .null,
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub fn jsonExtra(self: @This()) JsonValue {
        return self.extra;
    }
};
pub const Model = struct {
    id: []const u8,
    name: []const u8,
    api: Api,
    provider: ProviderId,
    baseUrl: []const u8,
    type: ?ModelType = null,
    input: []const ModelModality = &.{.text},
    inputLimits: ?JsonValue = null,
    cost: ModelCost = .{},
    headers: ?JsonValue = null,
    reasoning: bool = false,
    thinkingLevelMap: ?JsonValue = null,
    promptCache: ?JsonValue = null,
    contextWindow: f64 = 0,
    maxTokens: f64 = 0,
    samplingParams: ?JsonValue = null,
    samplingParamsByThinkingLevel: ?JsonValue = null,
    compat: ?JsonValue = null,
    output: ?[]const ModelModality = null,
    /// The original wire value for an unknown kind; retained until the refresh
    /// boundary filters it. This native field is not an extra JSON property.
    unknownType: ?JsonValue = null,
    /// Unmodeled wire properties, including provider-specific catalog metadata.
    /// Known fields above always take precedence when serialized.
    extra: JsonValue = .null,
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    fn knownField(name: []const u8) bool {
        inline for (@typeInfo(ModelFields).@"struct".field_names) |field| {
            if (comptime std.mem.eql(u8, field, "extra") or std.mem.eql(u8, field, "jsonFields") or std.mem.eql(u8, field, "jsonAssigned") or std.mem.eql(u8, field, "jsonOmitted") or std.mem.eql(u8, field, "jsonRetained")) continue;
            if (std.mem.eql(u8, name, field)) return true;
        }
        return false;
    }
    pub fn modelType(self: @This()) ModelType {
        return self.type orelse .chat;
    }
    pub fn hasKnownType(self: @This()) bool {
        return self.modelType() != .unknown;
    }
    fn wireFields(self: @This()) ModelFields {
        var fields: ModelFields = undefined;
        inline for (@typeInfo(ModelFields).@"struct".field_names) |name| {
            if (comptime std.mem.eql(u8, name, "type")) fields.type = if (self.unknownType) |raw| raw else if (self.type) |kind| j.str(@tagName(kind)) else null else @field(fields, name) = @field(self, name);
        }
        return fields;
    }
    pub fn jsonToValue(self: @This(), a: std.mem.Allocator, options: std.json.Stringify.Options) !JsonValue {
        return j.Record(ModelFields).jsonToValue(self.wireFields(), a, options);
    }
    pub fn jsonStringify(self: @This(), writer: *std.json.Stringify) !void {
        try j.Record(ModelFields).jsonStringify(self.wireFields(), writer);
    }
    pub fn jsonParse(a: std.mem.Allocator, source: anytype, options: std.json.ParseOptions) !@This() {
        var configured = options;
        configured.duplicate_field_behavior = .use_last;
        configured.parse_numbers = false;
        var value = try JsonValue.jsonParse(a, source, configured);
        try j.normalizeParsed(&value);
        return jsonParseFromValue(a, value, options);
    }
    pub fn jsonParseFromValue(a: std.mem.Allocator, value: JsonValue, options: std.json.ParseOptions) !@This() {
        const raw = j.get(value, "type");
        const kind = if (raw) |present| (if (j.text(present)) |name| std.meta.stringToEnum(ModelType, name) else null) else null;
        const unknown = if (raw) |present| present != .null and (kind == null or kind.? == .unknown) else false;
        if (value != .object) return error.UnexpectedToken;
        var input = j.map(a);
        var extra = j.map(a);
        for (try j.objectEntries(a, value.object)) |entry| {
            if (knownField(entry.key)) try j.put(a, &input, entry.key, entry.node.*) else try j.put(a, &extra, entry.key, entry.node.*);
        }
        const fields = try j.Record(ModelFields).jsonParseFromValue(a, input, options);
        var result: @This() = undefined;
        inline for (@typeInfo(ModelFields).@"struct".field_names) |name| {
            if (comptime std.mem.eql(u8, name, "type")) result.type = if (unknown) .unknown else kind else @field(result, name) = @field(fields, name);
        }
        result.jsonFields = try j.clone(a, value);
        result.unknownType = if (unknown) raw else null;
        result.extra = if (extra.object.count() > 0) extra else .null;
        return result;
    }
};
pub const AnyModel = Model;
pub const ImageModel = Model;
pub const ClassifierModel = Model;
pub const ImagesContext = struct {
    input: []const Content,
    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const AssistantImages = struct {
    api: Api,
    provider: ProviderId,
    model: []const u8,
    output: []const Content = &.{},
    responseId: ?[]const u8 = null,
    usage: ?Usage = null,
    stopReason: StopReason = .stop,
    errorMessage: ?[]const u8 = null,
    timestamp: f64,
    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const ClassifierQuestion = union(enum) {
    choice: struct {
        instructions: []const u8,
        criteria: JsonValue,
        jsonFields: JsonValue = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    score: struct {
        instructions: []const u8,
        criteria: []const []const u8,
        jsonFields: JsonValue = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    bool: struct {
        instructions: []const u8,
        criteria: struct {
            true: []const u8,
            false: []const u8,
            jsonFields: JsonValue = .null,
            jsonAssigned: u64 = 0,
            jsonOmitted: u64 = 0,
            jsonRetained: u64 = 0,
            pub const jsonToValue = j.Record(@This()).jsonToValue;
            pub const jsonStringify = j.Record(@This()).jsonStringify;
            pub const jsonParse = j.Record(@This()).jsonParse;
            pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
        },
        jsonFields: JsonValue = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    pub const jsonDiscriminator = "type";
    pub const jsonStringify = j.Tagged(@This(), "type").jsonStringify;
    pub const jsonParse = j.Tagged(@This(), "type").jsonParse;
    pub const jsonParseFromValue = j.Tagged(@This(), "type").jsonParseFromValue;
};
pub const ClassifierContext = struct {
    state: JsonValue,
    images: ?[]const ImageContent = null,
    questions: JsonValue,
    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const ClassifierAnswer = union(enum) {
    choice: struct {
        choice: []const u8,
        probabilities: JsonValue,
        confidence: f64,
        jsonFields: JsonValue = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    score: struct {
        score: f64,
        confidence: f64,
        jsonFields: JsonValue = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    bool: struct {
        probability: f64,
        jsonFields: JsonValue = .null,
        jsonAssigned: u64 = 0,
        jsonOmitted: u64 = 0,
        jsonRetained: u64 = 0,
        pub const jsonToValue = j.Record(@This()).jsonToValue;
        pub const jsonStringify = j.Record(@This()).jsonStringify;
        pub const jsonParse = j.Record(@This()).jsonParse;
        pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
    },
    pub const jsonDiscriminator = "type";
    pub const jsonStringify = j.Tagged(@This(), "type").jsonStringify;
    pub const jsonParse = j.Tagged(@This(), "type").jsonParse;
    pub const jsonParseFromValue = j.Tagged(@This(), "type").jsonParseFromValue;
};
pub const ClassifierResult = struct {
    api: Api,
    provider: ProviderId,
    model: []const u8,
    answers: JsonValue,
    usage: ?Usage = null,
    stopReason: StopReason = .stop,
    errorMessage: ?[]const u8 = null,
    timestamp: f64,
    /// Decoded property presence/order and unmodeled fields; owned with this record.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const ThinkingBudgets = struct {
    minimal: ?f64 = null,
    low: ?f64 = null,
    medium: ?f64 = null,
    high: ?f64 = null,
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const ProviderResponse = struct { status: u16, headers: ProviderHeaders };
pub const Callbacks = struct {
    context: ?*anyopaque = null,
    /// Borrow the working tree until the callback returns. Mutate it in place
    /// or return a replacement, allocating retained fields with this allocator.
    /// Optional null keeps the payload; a present JsonValue.null replaces it
    /// with JSON null, preserving the source's undefined/null distinction.
    onPayload: ?*const fn (?*anyopaque, std.mem.Allocator, *JsonValue, Model) anyerror!?JsonValue = null,
    onResponse: ?*const fn (?*anyopaque, ProviderResponse, Model) anyerror!void = null,
    /// Borrow the decoder's tree until return and treat it as read-only.
    onProviderStreamEvent: ?*const fn (?*anyopaque, JsonValue, Model) anyerror!void = null,
};
pub const StreamOptions = struct {
    apiKey: ?[]const u8 = null,
    env: ProviderEnv = .{},
    headers: ProviderHeaders = &.{},
    /// Opaque caller-owned context, forwarded unchanged to provider callbacks.
    /// Keep it alive through operation completion/producer join. No exporter
    /// or telemetry runtime is installed or invoked by the package.
    telemetryContext: ?*anyopaque = null,
    callbacks: Callbacks = .{},
    /// Optional native diagnostic slot, borrowed for the operation's lifetime.
    /// Its text may be static or borrow a payload region; no ownership transfers.
    /// Temporary adapters retain surviving text in OperationContext.errorStorage.
    errorMessage: ?*?[]const u8 = null,
    timeoutMs: ?f64 = null,
    maxRetries: ?f64 = null,
    maxRetryDelayMs: ?f64 = null,
    httpTransport: ?@import("transport/http.zig").Transport = null,
    /// Set by AnthropicOptions.client, whose adapter owns client authentication.
    /// Injecting only httpTransport retains normal provider auth resolution.
    authFromClient: bool = false,
    temperature: ?f64 = null,
    samplingParams: ?JsonValue = null,
    maxTokens: ?f64 = null,
    transport: ?Transport = null,
    cacheRetention: ?CacheRetention = null,
    sessionId: ?[]const u8 = null,
    websocketConnectTimeoutMs: ?f64 = null,
    metadata: ?JsonValue = null,
    toolChoice: ?ToolChoice = null,
    reasoning: ?ThinkingLevel = null,
    deferred: ?JsonValue = null,
    thinkingBudgets: ?ThinkingBudgets = null,
    extra: ?JsonValue = null,
    /// Presence marks support explicit nulls without changing borrowed native interfaces.
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
};
pub const SimpleStreamOptions = StreamOptions;
pub const ProviderStreamOptions = StreamOptions;
pub const ImagesOptions = StreamOptions;
pub const ClassifierOptions = StreamOptions;
pub const DeferredFetchOptions = struct { request: StreamOptions = .{}, wait: ?f64 = null };
pub const DeferredCancelOptions = StreamOptions;
/// Deep copy into a caller-owned allocation region. Use an arena when the
/// returned type contains nested allocations; Owned provides separate disposal.
/// Callback functions, opaque contexts, and pointers without a length remain
/// borrowed. This copies native data directly, preserving values and metadata
/// that have no JSON representation.
pub fn cloneInto(comptime T: type, a: std.mem.Allocator, value: T) anyerror!T {
    if (T == JsonValue) return j.clone(a, value);
    return switch (@typeInfo(T)) {
        .optional => |info| if (value) |present| try cloneInto(info.child, a, present) else null,
        .pointer => |info| switch (info.size) {
            .slice => blk: {
                const alignment = if (info.attrs.@"align") |bytes| std.mem.Alignment.fromByteUnits(bytes) else null;
                const copy = try a.allocWithOptions(info.child, value.len, alignment, info.sentinel());
                for (copy, value) |*out, in| out.* = try cloneInto(info.child, a, in);
                break :blk copy;
            },
            .one => switch (@typeInfo(info.child)) {
                .@"fn", .@"opaque" => value,
                else => blk: {
                    const alignment = if (info.attrs.@"align") |bytes| std.mem.Alignment.fromByteUnits(bytes) else null;
                    const copy = try a.allocWithOptions(info.child, 1, alignment, null);
                    copy[0] = try cloneInto(info.child, a, value.*);
                    break :blk &copy[0];
                },
            },
            .many, .c => value,
        },
        .array => |info| blk: {
            var copy = value;
            for (0..info.len) |index| copy[index] = try cloneInto(info.child, a, value[index]);
            break :blk copy;
        },
        .@"struct" => |info| blk: {
            var copy = value;
            inline for (info.field_names, info.field_types, info.field_attrs) |name, Field, attrs| {
                if (!attrs.@"comptime") @field(copy, name) = try cloneInto(Field, a, @field(value, name));
            }
            break :blk copy;
        },
        .@"union" => |info| blk: {
            if (info.tag_type == null) @compileError("cloneInto requires a tagged union");
            break :blk switch (value) {
                inline else => |payload, tag| @unionInit(T, @tagName(tag), if (comptime @typeInfo(@TypeOf(payload)) == .error_set) payload else try cloneInto(@TypeOf(payload), a, payload)),
            };
        },
        else => value,
    };
}
/// An independently owned deep value. value() borrows it until deinit(); opaque
/// contexts/function pointers remain borrowed as described by cloneInto.
pub fn Owned(comptime T: type) type {
    return struct {
        parsed: std.json.Parsed(T),
        pub fn clone(a: std.mem.Allocator, original: T) !@This() {
            const arena = try a.create(std.heap.ArenaAllocator);
            arena.* = std.heap.ArenaAllocator.init(a);
            errdefer {
                arena.deinit();
                a.destroy(arena);
            }
            return .{ .parsed = .{ .arena = arena, .value = try cloneInto(T, arena.allocator(), original) } };
        }
        pub fn value(self: *const @This()) T {
            return self.parsed.value;
        }
        pub fn deinit(self: *@This()) void {
            self.parsed.deinit();
            self.* = undefined;
        }
    };
}

// Native equivalents of the root's type-only exports. Open JSON metadata
// retains its decoded tree; these aliases do not narrow custom wire fields.
pub const SamplingParams = JsonValue;
pub const SamplingParamsByThinkingLevel = JsonValue;
pub const ThinkingLevelMap = JsonValue;
pub const ModelInputModality = ModelModality;
pub const ModelPromptCache = JsonValue;
pub const ModelImageResizeOptions = JsonValue;
pub const ModelImageInputLimits = JsonValue;
pub const ModelInputLimits = JsonValue;
pub const AnthropicAllowedFallbackModel = JsonValue;
pub const AnthropicMessagesCompat = JsonValue;
pub const BedrockCompat = JsonValue;
pub const ChatTemplateKwargValue = JsonValue;
pub const MistralConversationsCompat = JsonValue;
pub const OpenAICompletionsCompat = JsonValue;
pub const OpenAIResponsesCompat = JsonValue;
pub const OpenRouterRouting = JsonValue;
pub const SessionAffinityFormat = enum { openai, @"openai-nosession", openrouter };
pub const ThinkingTokenBudgetField = enum { thinking_token_budget, thinking_budget, thinking_budget_tokens };
pub const VercelGatewayRouting = JsonValue;
pub const FetchFunction = @import("transport/http.zig").Transport;
pub const ProviderRequestOptions = StreamOptions;
pub const ProviderImagesOptions = ImagesOptions;
pub const ProviderImages = @import("models.zig").ApiImplementation;
pub const ProviderClassifier = @import("models.zig").ApiImplementation;
pub const StreamFunction = @typeInfo(@FieldType(ProviderImages, "stream")).optional.child;
pub const ImagesFunction = @typeInfo(@FieldType(ProviderImages, "generateImages")).optional.child;
pub const ClassifierFunction = @typeInfo(@FieldType(ProviderClassifier, "classify")).optional.child;
pub const TextSignatureV1 = struct {
    v: u8 = 1,
    id: []const u8,
    phase: ?enum { commentary, final_answer } = null,
    jsonFields: JsonValue = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub fn JsonRepresentation(comptime T: type) type {
    _ = T;
    return JsonValue;
}
// Existing operations use Content and StopReason as common native carriers.
pub const ImagesInputContent = Content;
pub const ImagesOutputContent = Content;
pub const ImagesStopReason = StopReason;
pub const ClassifierStopReason = StopReason;
pub const ClassifierChoiceQuestion = @FieldType(ClassifierQuestion, "choice");
pub const ClassifierScoreQuestion = @FieldType(ClassifierQuestion, "score");
pub const ClassifierBoolQuestion = @FieldType(ClassifierQuestion, "bool");
pub const ClassifierChoiceAnswer = @FieldType(ClassifierAnswer, "choice");
pub const ClassifierScoreAnswer = @FieldType(ClassifierAnswer, "score");
pub const ClassifierBoolAnswer = @FieldType(ClassifierAnswer, "bool");
pub const GrammarFormat = enum { openai_lark, openai_regex };
pub const GrammarVariants = JsonValue;
pub const BaseModel = Model;
pub const ModelTypeMap = struct {
    pub const chat = Model;
    pub const image = ImageModel;
    pub const classifier = ClassifierModel;
};
pub const ApiOptionsMap = struct {
    pub const @"anthropic-messages" = @import("api/options.zig").AnthropicOptions;
    pub const @"openai-completions" = @import("api/options.zig").OpenAICompletionsOptions;
    pub const @"openai-responses" = @import("api/options.zig").OpenAIResponsesOptions;
    pub const @"openai-codex-responses" = @import("api/options.zig").OpenAICodexResponsesOptions;
    pub const @"azure-openai-responses" = @import("api/options.zig").AzureOpenAIResponsesOptions;
    pub const @"google-generative-ai" = @import("api/google_generative_ai.zig").GoogleOptions;
    pub const @"google-vertex" = @import("api/google_vertex.zig").GoogleVertexOptions;
    pub const @"mistral-conversations" = @import("api/options.zig").MistralOptions;
    pub const @"bedrock-converse-stream" = @import("api/options.zig").BedrockOptions;
    pub const @"pi-messages" = @import("api/options.zig").PiMessagesOptions;
};
pub fn ApiStreamOptions(comptime api_id: []const u8) type {
    return if (@hasDecl(ApiOptionsMap, api_id)) @field(ApiOptionsMap, api_id) else StreamOptions;
}
