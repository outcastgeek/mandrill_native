const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const numbers = @import("../utils/numbers.zig");
pub const MIN_ANSWER_TOKENS: u64 = 1024;
pub const DEFAULT_THINKING_BUDGETS = t.ThinkingBudgets{ .minimal = 1024, .low = 2048, .medium = 8192, .high = 16384 };
pub fn clampReasoning(level: ?t.ThinkingLevel) ?t.ThinkingLevel {
    return if (level == .xhigh or level == .max) .high else level;
}
pub fn resolveSamplingParams(a: std.mem.Allocator, model: t.Model, level: t.ThinkingLevel, request: ?std.json.Value) !?std.json.Value {
    const effective = @import("../utils/thinking.zig").clampThinkingLevel(model, level);
    const mapped = if (model.samplingParamsByThinkingLevel) |levels| j.get(levels, @tagName(effective)) else null;
    if (!j.truthy(model.samplingParams orelse .null) and !j.truthy(mapped orelse .null) and !j.truthy(request orelse .null)) return null;
    var result = j.map(a);
    for ([_]?std.json.Value{ model.samplingParams, mapped, request }) |source| if (source) |value| {
        const fields = try j.objectSpread(a, value);
        for (fields.object.keys(), fields.object.values()) |name, field| try j.put(a, &result, name, field);
    };
    return result;
}
pub fn modelTokenLimit(value: f64) !f64 {
    return value;
}
/// Null optional means JavaScript undefined; a present JSON null remains
/// distinct until arithmetic requires its numeric conversion.
pub fn thinkingBudgetForLevel(a: std.mem.Allocator, level: t.ThinkingLevel, custom: ?t.ThinkingBudgets) !?std.json.Value {
    var budgets = try j.toValue(a, DEFAULT_THINKING_BUDGETS, .{ .emit_null_optional_fields = false });
    if (custom) |provided| {
        const fields = try j.toValue(a, provided, .{ .emit_null_optional_fields = false });
        for (fields.object.keys(), fields.object.values()) |name, value| try j.put(a, &budgets, name, value);
    }
    return j.get(budgets, @tagName(clampReasoning(level).?));
}
pub fn budgetNumber(a: std.mem.Allocator, value: ?std.json.Value) !f64 {
    return numbers.coerceValue(a, value);
}
fn addBudget(a: std.mem.Allocator, base: f64, value: ?std.json.Value) !f64 {
    if (value) |selected| switch (selected) {
        .string, .array, .object => {
            const text = try @import("../utils/diagnostics.zig").formatThrownValue(a, .{ .value = selected });
            const sum = try std.mem.concat(a, u8, &.{ try numbers.toString(a, base), text });
            return try numbers.number(a, null, sum) orelse std.math.nan(f64);
        },
        else => {},
    };
    return base + try budgetNumber(a, value);
}
pub fn clampThinkingBudgetToAnswerRoom(budget: f64, ceiling: f64) f64 {
    return numbers.minimum(budget, numbers.maximum(0, ceiling - 1024));
}
pub fn adjustMaxTokensForThinking(a: std.mem.Allocator, base: ?f64, model_max: f64, level: t.ThinkingLevel, custom: ?t.ThinkingBudgets) !struct { maxTokens: f64, thinkingBudget: ?std.json.Value } {
    var budget = try thinkingBudgetForLevel(a, level, custom);
    const max_tokens = if (base) |n| numbers.minimum(try addBudget(a, n, budget), model_max) else model_max;
    const numeric = try budgetNumber(a, budget);
    if (max_tokens <= numeric) budget = j.num(clampThinkingBudgetToAnswerRoom(numeric, max_tokens));
    return .{ .maxTokens = max_tokens, .thinkingBudget = budget };
}
pub fn clampMaxTokensToContext(a: std.mem.Allocator, model: t.Model, context: t.TranscriptContext, max_tokens: f64) !f64 {
    if (model.contextWindow <= 0) return @import("../utils/numbers.zig").maximum(1, max_tokens);
    const estimate = try @import("../utils/estimate.zig").estimateContextTokens(a, context);
    const tokens = try @import("../utils/numbers.zig").coerceValue(a, try j.fieldValue(a, estimate, "tokens"));
    return @import("../utils/numbers.zig").minimum(max_tokens, @import("../utils/numbers.zig").maximum(1, model.contextWindow - tokens - 4096));
}
pub fn buildBaseOptions(a: std.mem.Allocator, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !t.StreamOptions {
    var out = options;
    out.reasoning = if (options.reasoning) |level| @import("../utils/thinking.zig").clampThinkingLevel(model, level) else null;
    out.samplingParams = try resolveSamplingParams(a, model, options.reasoning orelse .off, options.samplingParams);
    out.maxTokens = try clampMaxTokensToContext(a, model, context, options.maxTokens orelse try modelTokenLimit(model.maxTokens));
    return out;
}
/// Native explicit-key form of the source helper's optional apiKey argument.
pub fn buildBaseOptionsWithApiKey(a: std.mem.Allocator, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions, api_key: ?[]const u8) !t.StreamOptions {
    var result = try buildBaseOptions(a, model, context, options);
    if (api_key) |key| if (key.len > 0) {
        result.apiKey = key;
    };
    return result;
}
pub fn forApi(a: std.mem.Allocator, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !t.StreamOptions {
    if (std.mem.eql(u8, model.api, "pi-messages")) return options;
    const c = @import("common.zig");
    var out = try buildBaseOptions(a, model, context, options);
    var extra = j.map(a);
    const level = out.reasoning orelse .off;
    if (std.mem.eql(u8, model.api, "anthropic-messages")) {
        try j.put(a, &extra, "thinkingEnabled", .{ .bool = options.reasoning != null });
        if (options.reasoning != null) {
            if (c.compatBool(model, "forceAdaptiveThinking", false)) {
                const requested = options.reasoning.?;
                const mapped = if (model.thinkingLevelMap) |map| j.field(map, @tagName(requested)) else null;
                const effort = mapped orelse switch (requested) {
                    .minimal, .low => "low",
                    .medium => "medium",
                    else => "high",
                };
                try j.put(a, &extra, "effort", j.str(effort));
            } else {
                const adjusted = try adjustMaxTokensForThinking(a, out.maxTokens, try modelTokenLimit(model.maxTokens), options.reasoning.?, options.thinkingBudgets);
                out.maxTokens = try clampMaxTokensToContext(a, model, context, adjusted.maxTokens);
                try j.put(a, &extra, "thinkingBudgetTokens", j.num(clampThinkingBudgetToAnswerRoom(try budgetNumber(a, adjusted.thinkingBudget), out.maxTokens.?)));
            }
        }
    } else if (std.mem.eql(u8, model.api, "google-generative-ai") or std.mem.eql(u8, model.api, "google-vertex")) {
        const google = @import("google_shared.zig");
        if (level == .off) try j.put(a, &extra, "thinking", try j.from(a, .{ .enabled = false })) else if (google.usesGoogleThinkingLevel(model.id)) try j.put(a, &extra, "thinking", try j.from(a, .{ .enabled = true, .level = google.toGoogleThinkingLevel(try google.resolveGoogleThinkingLevelWithError(a, model, level, options.errorMessage)) })) else {
            const mapped = try google.resolveGoogleThinkingLevelWithError(a, model, level, options.errorMessage);
            const mapped_level = std.meta.stringToEnum(t.ThinkingLevel, @tagName(mapped)).?;
            try j.put(a, &extra, "thinking", try j.from(a, .{ .enabled = true, .budgetTokens = try googleBudget(a, model, mapped_level, options.thinkingBudgets) }));
        }
    } else if (std.mem.eql(u8, model.api, "bedrock-converse-stream")) {
        const bedrock = @import("bedrock_converse_stream.zig");
        out.reasoning = options.reasoning;
        // Older Claude models share the answer ceiling with their thinking budget.
        if (options.reasoning) |reasoning| if (bedrock.isClaude(model) and !try bedrock.supportsAdaptiveThinking(a, model)) {
            const adjusted = try adjustMaxTokensForThinking(a, out.maxTokens, try modelTokenLimit(model.maxTokens), reasoning, options.thinkingBudgets);
            out.maxTokens = try clampMaxTokensToContext(a, model, context, adjusted.maxTokens);
            var budgets: t.ThinkingBudgets = options.thinkingBudgets orelse .{};
            const budget = clampThinkingBudgetToAnswerRoom(try budgetNumber(a, adjusted.thinkingBudget), out.maxTokens.?);
            switch (reasoning) {
                .minimal => budgets.minimal = budget,
                .low => budgets.low = budget,
                .medium => budgets.medium = budget,
                else => budgets.high = budget,
            }
            out.thinkingBudgets = budgets;
        };
    } else if (std.mem.eql(u8, model.api, "mistral-conversations")) {
        const effort_map = if (model.reasoning) model.thinkingLevelMap else null;
        // A supplied JSON null/empty scalar has the same falsy semantics as
        // the source's optional map; an empty object remains truthy.
        const has_map = j.truthy(effort_map orelse .null);
        if (model.reasoning and !has_map and level != .off) try j.put(a, &extra, "promptMode", j.str("reasoning"));
        if (has_map) {
            const map = effort_map.?;
            const selected = j.get(map, @tagName(level));
            const effort: ?std.json.Value = if (selected) |value| (if (value == .null) (if (level == .off) null else j.str("high")) else value) else if (level == .off) null else j.str("high");
            if (effort) |value| try j.put(a, &extra, "reasoningEffort", value);
        }
    }
    out.extra = extra;
    return out;
}
pub fn googleBudget(a: std.mem.Allocator, model: t.Model, level: t.ThinkingLevel, custom: ?t.ThinkingBudgets) !std.json.Value {
    if (custom) |budgets| {
        const fields = try j.toValue(a, budgets, .{ .emit_null_optional_fields = false });
        if (j.get(fields, @tagName(level))) |value| return value;
    }
    if (std.mem.find(u8, model.id, "2.5-pro") != null) return j.num(switch (level) {
        .minimal => @as(f64, 128),
        .low => 2048,
        .medium => 8192,
        else => 32768,
    });
    if (std.mem.find(u8, model.id, "2.5-flash-lite") != null and std.mem.eql(u8, model.api, "google-generative-ai")) return j.num(switch (level) {
        .minimal => @as(f64, 512),
        .low => 2048,
        .medium => 8192,
        else => 24576,
    });
    if (std.mem.find(u8, model.id, "2.5-flash") != null) return j.num(switch (level) {
        .minimal => @as(f64, 128),
        .low => 2048,
        .medium => 8192,
        else => 24576,
    });
    return j.num(-1);
}
