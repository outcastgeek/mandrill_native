//! Provider-specific option records. Known fields are typed; SDK-defined tool
//! choice objects remain std.json.Value and are never assembled as raw JSON.
const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
pub const ReasoningEffort = enum { minimal, low, medium, high, xhigh, max };
pub const ServiceTier = enum { auto, default, flex, priority, scale };
pub const AnthropicEffort = enum { low, medium, high, xhigh, max };
pub const AnthropicThinkingDisplay = enum { summarized, omitted };
pub const BedrockThinkingDisplay = enum { summarized, omitted };
fn convert(a: std.mem.Allocator, self: anytype) !t.StreamOptions {
    var out = self.request;
    var extra = if (out.extra) |value| try j.clone(a, value) else j.map(a);
    inline for (@typeInfo(@TypeOf(self)).@"struct".field_names) |field| {
        if (comptime std.mem.eql(u8, field, "request")) continue;
        const optional = @field(self, field);
        if (optional) |present| {
            if (comptime std.mem.eql(u8, field, "reasoning")) out.reasoning = present else if (comptime std.mem.eql(u8, field, "thinkingBudgets")) out.thinkingBudgets = present else if (comptime std.mem.eql(u8, field, "client")) {
                out.httpTransport = present;
                out.authFromClient = true;
            } else if (comptime std.mem.eql(u8, field, "reasoningEffort")) {
                out.reasoning = std.meta.stringToEnum(t.ThinkingLevel, @tagName(present));
                try j.put(a, &extra, field, try j.from(a, present));
            } else try j.put(a, &extra, field, try j.from(a, present));
        }
    }
    out.extra = extra;
    return out;
}
pub const OpenAICompletionsOptions = struct {
    request: t.StreamOptions = .{},
    toolChoice: ?std.json.Value = null,
    reasoningEffort: ?ReasoningEffort = null,
    thinkingBudgets: ?t.ThinkingBudgets = null,
    pub fn streamOptions(self: @This(), a: std.mem.Allocator) !t.StreamOptions {
        return convert(a, self);
    }
};
pub const OpenAIResponsesOptions = struct {
    request: t.StreamOptions = .{},
    reasoningEffort: ?ReasoningEffort = null,
    reasoningSummary: ?std.json.Value = null,
    serviceTier: ?ServiceTier = null,
    toolChoice: ?std.json.Value = null,
    pub fn streamOptions(self: @This(), a: std.mem.Allocator) !t.StreamOptions {
        return convert(a, self);
    }
};
pub const AzureOpenAIResponsesOptions = struct {
    request: t.StreamOptions = .{},
    reasoningEffort: ?ReasoningEffort = null,
    reasoningSummary: ?std.json.Value = null,
    serviceTier: ?ServiceTier = null,
    toolChoice: ?std.json.Value = null,
    azureBaseUrl: ?[]const u8 = null,
    azureApiVersion: ?[]const u8 = null,
    azureResourceName: ?[]const u8 = null,
    azureDeploymentName: ?[]const u8 = null,
    pub fn streamOptions(self: @This(), a: std.mem.Allocator) !t.StreamOptions {
        return convert(a, self);
    }
};
pub const OpenAICodexResponsesOptions = struct {
    request: t.StreamOptions = .{},
    reasoningEffort: ?enum { none, minimal, low, medium, high, xhigh, max } = null,
    reasoningSummary: ?std.json.Value = null,
    serviceTier: ?ServiceTier = null,
    textVerbosity: ?enum { low, medium, high } = null,
    toolChoice: ?enum { auto, none, required } = null,
    pub fn streamOptions(self: @This(), a: std.mem.Allocator) !t.StreamOptions {
        return convert(a, self);
    }
};
pub const AnthropicOptions = struct {
    request: t.StreamOptions = .{},
    thinkingEnabled: ?bool = null,
    thinkingBudgetTokens: ?f64 = null,
    effort: ?AnthropicEffort = null,
    thinkingDisplay: ?AnthropicThinkingDisplay = null,
    interleavedThinking: ?bool = null,
    toolChoice: ?std.json.Value = null,
    client: ?@import("../transport/http.zig").Transport = null,
    pub fn streamOptions(self: @This(), a: std.mem.Allocator) !t.StreamOptions {
        return convert(a, self);
    }
};
pub const BedrockOptions = struct {
    request: t.StreamOptions = .{},
    region: ?[]const u8 = null,
    profile: ?[]const u8 = null,
    toolChoice: ?std.json.Value = null,
    reasoning: ?t.ThinkingLevel = null,
    thinkingBudgets: ?t.ThinkingBudgets = null,
    interleavedThinking: ?bool = null,
    thinkingDisplay: ?BedrockThinkingDisplay = null,
    requestMetadata: ?std.json.Value = null,
    bearerToken: ?[]const u8 = null,
    pub fn streamOptions(self: @This(), a: std.mem.Allocator) !t.StreamOptions {
        return convert(a, self);
    }
};
pub const MistralOptions = struct {
    request: t.StreamOptions = .{},
    toolChoice: ?std.json.Value = null,
    promptMode: ?enum { reasoning } = null,
    reasoningEffort: ?enum { none, low, medium, high, max } = null,
    pub fn streamOptions(self: @This(), a: std.mem.Allocator) !t.StreamOptions {
        return convert(a, self);
    }
};
pub const PiMessagesOptions = struct {
    request: t.StreamOptions = .{},
    reasoning: ?t.ThinkingLevel = null,
    toolChoice: ?std.json.Value = null,
    debug: ?bool = null,
    pub fn streamOptions(self: @This(), a: std.mem.Allocator) !t.StreamOptions {
        return convert(a, self);
    }
};
