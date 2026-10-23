//! Allocator-owned equivalents of the pinned provider compatibility schemas.
const std = @import("std");
const s = @import("../schema/type.zig");
const Type = s.Type;
const j = @import("../json/value.zig");
const h = @import("schema_helpers.zig");
const models = @import("model_schema.zig");
const t = @import("../types.zig");
pub const ChatTemplateKwargValue = t.ChatTemplateKwargValue;
pub const ThinkingTokenBudgetField = t.ThinkingTokenBudgetField;
pub const SessionAffinityFormat = t.SessionAffinityFormat;
pub const OpenRouterRouting = t.OpenRouterRouting;
pub const VercelGatewayRouting = t.VercelGatewayRouting;
pub const AnthropicAllowedFallbackModel = t.AnthropicAllowedFallbackModel;
pub const OpenAICompletionsCompat = t.OpenAICompletionsCompat;
pub const OpenAIResponsesCompat = t.OpenAIResponsesCompat;
pub const AnthropicMessagesCompat = t.AnthropicMessagesCompat;
pub const BedrockCompat = t.BedrockCompat;
pub const MistralConversationsCompat = t.MistralConversationsCompat;
fn sessionFormat(a: std.mem.Allocator, description: []const u8) !s.TSchema {
    return h.literals(a, &.{ "openai", "openai-nosession", "openrouter" }, .{ .description = description });
}
pub fn SessionAffinityFormatSchema(a: std.mem.Allocator) !s.TSchema {
    return sessionFormat(a, "Session-affinity header format used to route related requests consistently.");
}
fn budgetField(a: std.mem.Allocator, description: []const u8) !s.TSchema {
    return h.literals(a, &.{ "thinking_token_budget", "thinking_budget", "thinking_budget_tokens" }, .{ .description = description });
}
pub fn ThinkingTokenBudgetFieldSchema(a: std.mem.Allocator) !s.TSchema {
    return budgetField(a, "Top-level request field name used by OpenAI-compatible endpoints to cap reasoning tokens. \"thinking_token_budget\" is used by vLLM, \"thinking_budget\" by Qwen, DashScope, or SGLang, and \"thinking_budget_tokens\" by llama.cpp.");
}
pub fn ChatTemplateKwargValueSchema(a: std.mem.Allocator) !s.TSchema {
    const variable = try h.object(a, &.{
        .{ .name = "$var", .schema = try h.literals(a, &.{ "thinking.enabled", "thinking.effort", "thinking.budget" }, .{}), .required = true },
        .{ .name = "omitWhenOff", .schema = try Type.Boolean(a, .{}) },
    }, .{});
    return Type.Union(a, &.{ try Type.String(a, .{}), try Type.Number(a, .{}), try Type.Boolean(a, .{}), try Type.Null(a), variable });
}
fn boolean(a: std.mem.Allocator, name: []const u8, description: []const u8, default: ?bool) !h.Field {
    return .{ .name = name, .schema = try Type.Boolean(a, .{ .description = description, .default = if (default) |value| .{ .bool = value } else null }) };
}
fn strings(a: std.mem.Allocator, name: []const u8, description: []const u8) !h.Field {
    return .{ .name = name, .schema = try Type.Array(a, try Type.String(a, .{}), .{ .description = description }) };
}
fn numberOrString(a: std.mem.Allocator, name: []const u8, description: []const u8) !h.Field {
    return .{ .name = name, .schema = try Type.UnionWithOptions(a, &.{ try Type.Number(a, .{}), try Type.String(a, .{}) }, .{ .description = description }) };
}
fn percentileCutoffs(a: std.mem.Allocator, metric: []const u8) !s.TSchema {
    var fields: [4]h.Field = undefined;
    for ([_][]const u8{ "p50", "p75", "p90", "p99" }, [_]u8{ 50, 75, 90, 99 }, &fields) |name, percentile, *field| field.* = .{ .name = name, .schema = try Type.Number(a, .{ .description = try std.fmt.allocPrint(a, "{s} at the {d}th percentile.", .{ metric, percentile }) }) };
    return h.object(a, &fields, .{});
}
pub fn OpenRouterRoutingSchema(a: std.mem.Allocator) !s.TSchema {
    const sort = try Type.UnionWithOptions(a, &.{ try Type.String(a, .{}), try h.object(a, &.{
        .{ .name = "by", .schema = try Type.String(a, .{ .description = "The sorting metric, such as \"price\", \"throughput\", or \"latency\"." }) },
        .{ .name = "partition", .schema = try Type.UnionWithOptions(a, &.{ try Type.String(a, .{}), try Type.Null(a) }, .{ .description = "Partitioning strategy: \"model\" or \"none\".", .default = j.str("model") }) },
    }, .{}) }, .{ .description = "Sorting strategy. Can be a string such as \"price\", \"throughput\", or \"latency\", or an object." });
    const max_price = try h.object(a, &.{
        try numberOrString(a, "prompt", "Price per million prompt tokens."), try numberOrString(a, "completion", "Price per million completion tokens."),
        try numberOrString(a, "image", "Price per image."),                  try numberOrString(a, "audio", "Price per audio unit."),
        try numberOrString(a, "request", "Price per request."),
    }, .{ .description = "Maximum price per million tokens in USD." });
    return h.object(a, &.{
        try boolean(a, "allow_fallbacks", "Whether to allow backup providers to serve requests.", true),
        try boolean(a, "require_parameters", "Whether to filter providers to only those that support all parameters in the request.", false),
        .{ .name = "data_collection", .schema = try h.literals(a, &.{ "deny", "allow" }, .{ .description = "Data collection setting. \"allow\": allow providers that may store or train on data. \"deny\": only use providers that do not collect user data.", .default = j.str("allow") }) },
        try boolean(a, "zdr", "Whether to restrict routing to only ZDR (Zero Data Retention) endpoints.", null),
        try boolean(a, "enforce_distillable_text", "Whether to restrict routing to only models that allow text distillation.", null),
        try strings(a, "order", "An ordered list of provider names or slugs to try in sequence, falling back to the next if unavailable."),
        try strings(a, "only", "List of provider names or slugs to exclusively allow for this request."),
        try strings(a, "ignore", "List of provider names or slugs to skip for this request."),
        try strings(a, "quantizations", "A list of quantization levels to filter providers by, for example [\"fp16\", \"bf16\", \"fp8\", \"fp6\", \"int8\", \"int4\", \"fp4\", \"fp32\"]."),
        .{ .name = "sort", .schema = sort },
        .{ .name = "max_price", .schema = max_price },
        .{ .name = "preferred_min_throughput", .schema = try Type.UnionWithOptions(a, &.{ try Type.Number(a, .{}), try percentileCutoffs(a, "Minimum tokens per second") }, .{ .description = "Preferred minimum throughput in tokens per second. A number applies to p50." }) },
        .{ .name = "preferred_max_latency", .schema = try Type.UnionWithOptions(a, &.{ try Type.Number(a, .{}), try percentileCutoffs(a, "Maximum latency in seconds") }, .{ .description = "Preferred maximum latency in seconds. A number applies to p50." }) },
    }, .{ .description = "OpenRouter provider routing preferences. Controls which upstream providers OpenRouter routes requests to. Sent as the provider field in the OpenRouter API request body. See https://openrouter.ai/docs/guides/routing/provider-selection." });
}
pub fn VercelGatewayRoutingSchema(a: std.mem.Allocator) !s.TSchema {
    return h.object(a, &.{
        try strings(a, "only", "List of provider slugs to exclusively use for this request, for example [\"bedrock\", \"anthropic\"]."),
        try strings(a, "order", "List of provider slugs to try in order, for example [\"anthropic\", \"openai\"]."),
    }, .{ .description = "Vercel AI Gateway routing preferences. Controls which upstream providers the gateway routes requests to. See https://vercel.com/docs/ai-gateway/models-and-providers/provider-options." });
}
pub fn AnthropicAllowedFallbackModelSchema(a: std.mem.Allocator) !s.TSchema {
    return h.object(a, &.{
        .{ .name = "provider", .schema = try Type.String(a, .{ .extra = try j.from(a, .{ .minLength = 1 }) }), .required = true },
        .{ .name = "model", .schema = try Type.String(a, .{ .extra = try j.from(a, .{ .minLength = 1 }) }), .required = true },
        .{ .name = "cost", .schema = try models.ModelCostSchema(a), .required = true },
    }, .{ .description = "An Anthropic server-side refusal fallback model with local pricing metadata." });
}
pub fn OpenAICompletionsCompatSchema(a: std.mem.Allocator) !s.TSchema {
    return h.object(a, &.{
        try boolean(a, "supportsStore", "Whether the provider supports the store field. Default: auto-detected from URL.", null),
        try boolean(a, "supportsDeveloperRole", "Whether the provider supports the developer role instead of system. Default: auto-detected from URL.", null),
        try boolean(a, "supportsReasoningEffort", "Whether the provider supports reasoning_effort. Default: auto-detected from URL.", null),
        try boolean(a, "supportsUsageInStreaming", "Whether the provider supports stream_options.include_usage for token usage in streaming responses.", true),
        try boolean(a, "supportsFinishReason", "Whether streamed responses include finish_reason. When false, pi infers stop or toolUse when the stream ends.", true),
        .{ .name = "maxTokensField", .schema = try h.literals(a, &.{ "max_completion_tokens", "max_tokens" }, .{ .description = "Which field to use for max tokens. Default: auto-detected from URL." }) },
        try boolean(a, "requiresToolResultName", "Whether tool results require the name field. Default: auto-detected from URL.", null),
        try boolean(a, "requiresAssistantAfterToolResult", "Whether a user message after tool results requires an assistant message in between. Default: auto-detected from URL.", null),
        try boolean(a, "requiresThinkingAsText", "Whether thinking blocks must be converted to text blocks with <thinking> delimiters. Default: auto-detected from URL.", null),
        try boolean(a, "requiresReasoningContentOnAssistantMessages", "Whether all replayed assistant messages must include an empty reasoning_content field when reasoning is enabled. Default: auto-detected from URL.", null),
        .{ .name = "thinkingFormat", .schema = try h.literals(a, &.{ "openai", "openrouter", "deepseek", "together", "baseten", "zai", "qwen", "chat-template", "qwen-chat-template", "string-thinking", "ant-ling" }, .{ .description = "Format for reasoning or thinking parameters. When omitted, Pi auto-detects the format from the provider URL. \"openai\" uses reasoning_effort, \"openrouter\" uses reasoning.effort, \"deepseek\" uses thinking.type plus reasoning_effort when supported, \"together\" uses reasoning.enabled plus reasoning_effort when supported, \"baseten\" uses configurable chat_template_args plus reasoning_effort when supported, \"zai\" uses thinking.type, \"qwen\" uses top-level enable_thinking, \"qwen-chat-template\" uses chat_template_kwargs.enable_thinking and preserve_thinking, \"chat-template\" uses configurable chat_template_kwargs, \"string-thinking\" uses top-level thinking, and \"ant-ling\" uses reasoning.effort only when the mapped effort is non-null." }) },
        .{ .name = "chatTemplateKwargs", .schema = try h.stringRecord(a, try ChatTemplateKwargValueSchema(a), "Kwargs sent as chat_template_kwargs when thinkingFormat is \"chat-template\". Use $var with \"thinking.enabled\", \"thinking.effort\", or \"thinking.budget\" for pi-controlled values.") },
        .{ .name = "chatTemplateArgs", .schema = try h.stringRecord(a, try ChatTemplateKwargValueSchema(a), "Arguments sent as chat_template_args when thinkingFormat is \"baseten\". Use $var with \"thinking.enabled\", \"thinking.effort\", or \"thinking.budget\" for pi-controlled values.") },
        .{ .name = "openRouterRouting", .schema = try OpenRouterRoutingSchema(a) },
        .{ .name = "vercelGatewayRouting", .schema = try VercelGatewayRoutingSchema(a) },
        try boolean(a, "zaiToolStream", "Whether z.ai supports top-level tool_stream for streaming tool call deltas.", false),
        .{ .name = "thinkingTokenBudgetField", .schema = try budgetField(a, "Top-level request field used to cap reasoning tokens from thinkingBudgets. Reasoning and the answer share max_tokens on these endpoints. \"thinking_token_budget\" is vLLM, \"thinking_budget\" is Qwen, DashScope, or SGLang, and \"thinking_budget_tokens\" is llama.cpp. Off by default and not set on the generated catalog.") },
        try boolean(a, "supportsThinkingTokenBudget", "Alias for thinkingTokenBudgetField: \"thinking_token_budget\" (vLLM). Prefer thinkingTokenBudgetField.", false),
        try boolean(a, "supportsOpenAIGrammarTools", "Whether the provider supports OpenAI custom tools with Lark or regex grammar formats. When false, grammar-constrained tools fall back to normal function tools. The generated catalog enables this for capable models.", false),
        try boolean(a, "supportsMidConvoSystemMessages", "Whether the exact model accepts system or developer messages after the conversation has started. When false, later system messages are folded into the leading system message. The generated catalog enables this for verified models.", false),
        try boolean(a, "supportsMidConvoToolAdditions", "Whether system messages can introduce additional tools mid-conversation. Requires supportsMidConvoSystemMessages. The generated catalog enables this for capable models.", false),
        try boolean(a, "supportsStrictMode", "Whether the provider supports the strict field in tool definitions. Generated capable models enable it explicitly.", false),
        .{ .name = "cacheControlFormat", .schema = try Type.LiteralWithOptions(a, j.str("anthropic"), .{ .description = "Cache control convention for prompt caching. Anthropic applies cache_control markers to the system prompt, last tool definition, and last user, assistant, or tool-result text content." }) },
        try boolean(a, "sendSessionAffinityHeaders", "Whether to send session-affinity data from options.sessionId. Default: true for OpenRouter endpoints, false otherwise.", null),
        .{ .name = "sessionAffinityFormat", .schema = try sessionFormat(a, "Session-affinity header format. openai sends session_id, x-client-request-id, and x-session-affinity; openai-nosession sends x-client-request-id and x-session-affinity; openrouter sends x-session-id. Does not affect prompt_cache_key. Default: auto-detected.") },
        try boolean(a, "supportsLongCacheRetention", "Whether the provider supports long prompt cache retention (prompt_cache_retention: \"24h\" or Anthropic-style cache_control.ttl: \"1h\", depending on format). Default: auto-detected from provider and URL.", null),
        .{ .name = "vllmPriority", .schema = try Type.Number(a, .{ .description = "vLLM scheduler priority sent as the top-level priority request field. Lower values are handled earlier and the server default is 0. Only meaningful with --scheduling-policy priority. Off by default and not set on the generated catalog." }) },
    }, .{ .description = "Compatibility settings for OpenAI-compatible completions APIs. Use this to override URL-based auto-detection for custom providers.", .extra = try j.from(a, .{ .additionalProperties = true }) });
}
pub fn OpenAIResponsesCompatSchema(a: std.mem.Allocator) !s.TSchema {
    return h.object(a, &.{
        try boolean(a, "supportsDeveloperRole", "Whether the provider supports the developer role instead of system.", true),
        try boolean(a, "supportsMidConvoSystemMessages", "Whether the exact model accepts developer or system messages after the conversation has started. When false, later system messages are folded into the leading system message. The generated catalog enables this for verified models.", false),
        .{ .name = "sessionAffinityFormat", .schema = try sessionFormat(a, "Session-affinity header format. openai sends session_id and x-client-request-id; openai-nosession sends x-client-request-id; openrouter sends x-session-id. Does not affect prompt_cache_key. Default: auto-detected.") },
        try boolean(a, "supportsLongCacheRetention", "Whether the provider supports long prompt cache retention. This uses prompt_cache_options.ttl: \"30m\" on GPT-5.6+ and prompt_cache_retention: \"24h\" on earlier models.", true),
        try boolean(a, "supportsStrictMode", "Whether the provider supports strict JSON-schema function tools. Defaults are API-specific; generated OpenAI models enable it explicitly.", null),
        try boolean(a, "supportsOpenAIGrammarTools", "Whether to emit OpenAI custom tools with Lark or regex grammar formats. When false, grammar-constrained tools fall back to normal function tools. The generated catalog enables this for capable models.", false),
        try boolean(a, "supportsAdditionalTools", "Whether the model supports message-anchored additional_tools input items.", false),
        try boolean(a, "supportsToolSearch", "Whether the model supports client-executed tool search for transcript-anchored additions.", false),
        try boolean(a, "supportsExplicitPromptCacheMode", "Whether the model accepts prompt_cache_options. Older OpenAI models reject the parameter.", false),
        try boolean(a, "supportsMidConvoEffort", "Whether thinking level changes are sent as configuration_update input items instead of changing reasoning.effort, so a change keeps the prompt cache. Only used by the openai-responses API, and ignored with Sign in with ChatGPT on the openai provider.", false),
        try boolean(a, "supportsMaxOutputTokens", "Whether the provider accepts max_output_tokens. Some Codex-protocol gateways reject it.", true),
    }, .{ .description = "Compatibility settings for OpenAI Responses APIs.", .extra = try j.from(a, .{ .additionalProperties = true }) });
}
pub fn AnthropicMessagesCompatSchema(a: std.mem.Allocator) !s.TSchema {
    return h.object(a, &.{
        try boolean(a, "supportsEagerToolInputStreaming", "Whether the provider accepts per-tool eager_input_streaming. When false, the Anthropic provider omits tools[].eager_input_streaming and sends the legacy fine-grained-tool-streaming-2025-05-14 beta header for tool-enabled requests.", true),
        try boolean(a, "supportsLongCacheRetention", "Whether the provider supports Anthropic long cache retention through cache_control.ttl.", true),
        try boolean(a, "sendSessionAffinityHeaders", "Whether to send x-session-affinity from options.sessionId when caching is enabled. Required for providers like Fireworks that use session affinity for prompt cache routing; requests to the same replica maximize cache hits. Default: true for OpenRouter endpoints, false otherwise.", null),
        .{ .name = "sessionAffinityFormat", .schema = try Type.LiteralWithOptions(a, j.str("openrouter"), .{ .description = "Session-affinity format. openrouter sends x-session-id; when unset, sends x-session-affinity." }) },
        try boolean(a, "supportsCacheControlOnTools", "Whether the provider supports Anthropic-style cache_control markers on tool definitions. When false, cache_control is omitted from tool parameters. Some Anthropic-compatible providers, such as Fireworks, do not support this field on tools and may reject or ignore it.", true),
        try boolean(a, "supportsTemperature", "Whether the model accepts the Anthropic temperature request field. Claude Opus 4.7+ rejects non-default values.", true),
        try boolean(a, "forceAdaptiveThinking", "Whether to force adaptive thinking (thinking.type: adaptive plus output_config.effort) regardless of model ID. Built-in models that require adaptive thinking set this in generated metadata. Custom Anthropic-compatible providers can set this to true for any model whose upstream requires the adaptive format. Set false to opt out on overridden built-in models.", false),
        try boolean(a, "allowEmptySignature", "Whether to replay empty thinking signatures instead of converting thinking to text.", false),
        try boolean(a, "supportsStrictTools", "Whether the provider supports Anthropic strict tool schemas. Generated Anthropic models enable it explicitly.", false),
        try boolean(a, "supportsMidConvoEffort", "Whether the exact model transport supports effort-only system messages and thinking binding controls.", false),
        try boolean(a, "supportsMidConvoSystemMessages", "Whether the exact model accepts system-role messages inside the conversation. When false, later system messages are folded into the top-level system prompt.", false),
        try boolean(a, "supportsMidConvoToolChanges", "Whether the exact model accepts mid-conversation tool_addition and tool_removal blocks. Requires supportsMidConvoSystemMessages.", false),
        .{ .name = "allowedFallbackModels", .schema = try Type.Array(a, try AnthropicAllowedFallbackModelSchema(a), .{ .description = "Models Anthropic accepts for server-side refusal fallback, with local pricing metadata for returned fallback responses. When absent or empty, callers must omit fallbacks; Anthropic rejects the field for models with no permitted fallback targets.", .extra = try j.from(a, .{ .maxItems = 3 }) }) },
    }, .{ .description = "Compatibility settings for Anthropic Messages-compatible APIs.", .extra = try j.from(a, .{ .additionalProperties = true }) });
}
pub fn BedrockCompatSchema(a: std.mem.Allocator) !s.TSchema {
    return h.object(a, &.{try boolean(a, "supportsStrictMode", "Whether the model supports Bedrock strict tool schemas.", false)}, .{ .description = "Compatibility settings for Amazon Bedrock models.", .extra = try j.from(a, .{ .additionalProperties = true }) });
}
pub fn MistralConversationsCompatSchema(a: std.mem.Allocator) !s.TSchema {
    return h.object(a, &.{try boolean(a, "supportsMidConvoSystemMessages", "Whether the exact model accepts system messages after the conversation has started. When false, later system messages are folded into the leading system message.", false)}, .{ .description = "Compatibility settings for the Mistral chat API.", .extra = try j.from(a, .{ .additionalProperties = true }) });
}
pub const ApiSchemas = std.StringArrayHashMapUnmanaged(s.TSchema);
pub fn CompatSchemasByApi(a: std.mem.Allocator) !ApiSchemas {
    var result: ApiSchemas = .{};
    errdefer result.deinit(a);
    try result.put(a, "openai-completions", try OpenAICompletionsCompatSchema(a));
    const responses = try OpenAIResponsesCompatSchema(a);
    try result.put(a, "openai-responses", responses);
    try result.put(a, "azure-openai-responses", responses);
    try result.put(a, "openai-codex-responses", responses);
    try result.put(a, "anthropic-messages", try AnthropicMessagesCompatSchema(a));
    try result.put(a, "bedrock-converse-stream", try BedrockCompatSchema(a));
    try result.put(a, "mistral-conversations", try MistralConversationsCompatSchema(a));
    return result;
}
pub fn ProviderCompatSchema(a: std.mem.Allocator) !s.TSchema {
    const overrides = [_]h.Field{
        try boolean(a, "supportsDeveloperRole", "Whether the provider supports the developer role instead of system. Defaults are API-specific.", null),
        try boolean(a, "supportsMidConvoSystemMessages", "Whether the exact model accepts system or developer messages after the conversation has started. When false, later system messages are folded into the leading system message.", false),
        .{ .name = "sessionAffinityFormat", .schema = try sessionFormat(a, "Session-affinity header format. Defaults are API-specific or auto-detected.") },
        try boolean(a, "supportsMidConvoEffort", "Whether thinking level changes are sent inside the conversation instead of changing the request's effort, so a change keeps the prompt cache. Used by the anthropic-messages and openai-responses APIs.", false),
        try boolean(a, "supportsLongCacheRetention", "Whether the provider supports long prompt cache retention. Defaults are API-specific or auto-detected.", null),
        try boolean(a, "supportsStrictMode", "Whether the provider supports strict tool schemas. Defaults are API-specific.", null),
        try boolean(a, "supportsOpenAIGrammarTools", "Whether the provider supports OpenAI custom tools with Lark or regex grammar formats. When false, grammar-constrained tools fall back to normal function tools.", false),
        try boolean(a, "sendSessionAffinityHeaders", "Whether to send session-affinity data from options.sessionId. Defaults are API-specific.", null),
    };
    var by_api = try CompatSchemasByApi(a);
    defer by_api.deinit(a);
    var merged: ApiSchemas = .{};
    defer merged.deinit(a);
    var duplicates: std.StringHashMapUnmanaged(void) = .{};
    defer duplicates.deinit(a);
    for (by_api.keys(), by_api.values()) |api, group| {
        // These entries share the same Responses schema in the source map.
        if (std.mem.eql(u8, api, "azure-openai-responses") or std.mem.eql(u8, api, "openai-codex-responses")) continue;
        for (group.properties) |property| {
            if (merged.contains(property.name)) {
                try duplicates.put(a, property.name, {});
                const overridden = for (overrides) |field| {
                    if (std.mem.eql(u8, field.name, property.name)) break true;
                } else false;
                if (!overridden) return error.CompatibilitySchemaOverrideRequired;
            }
            try merged.put(a, property.name, property.schema);
        }
    }
    for (overrides) |field| {
        if (!duplicates.contains(field.name)) return error.CompatibilitySchemaOverrideWithoutDuplicate;
        try merged.put(a, field.name, field.schema);
    }
    const fields = try a.alloc(h.Field, merged.count());
    defer a.free(fields);
    for (merged.keys(), merged.values(), fields) |name, schema, *field| field.* = .{ .name = name, .schema = schema };
    return h.object(a, fields, .{ .description = "Provider and model compatibility overrides.", .extra = try j.from(a, .{ .additionalProperties = true }) });
}
