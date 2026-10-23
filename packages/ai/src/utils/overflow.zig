const std = @import("std");
const t = @import("../types.zig");
const patterns = @import("patterns.zig");
const regexp = @import("../regex/regexp.zig");
pub const Pattern = struct {
    source: []const u8,
    flags: regexp.Flags = .{ .ignoreCase = true },
    pub fn @"test"(self: @This(), a: std.mem.Allocator, text: []const u8) !bool {
        return regexp.matches(a, null, self.source, text, self.flags);
    }
};
pub const overflowPatterns: []const []const u8 = &.{ "prompt (?:is )?too long", "prompt exceeds max length", "request_too_large", "input is too long for requested model", "exceeds the context window", "exceeds (?:the )?(?:model'?s )?maximum context length(?: of [\\d,]+ tokens?|\\s*\\([\\d,]+\\))", "input token count.*exceeds the maximum", "maximum prompt length is \\d+", "reduce the length of the messages", "maximum context length is \\d+ tokens", "exceeds (?:the )?maximum allowed input length of [\\d,]+ tokens?", "input \\(\\d+ tokens\\) is longer than the model'?s context length \\(\\d+ tokens\\)", "exceeds the limit of \\d+", "exceeds the available context size", "greater than the context length", "context window exceeds limit", "exceeded model token limit", "too large for model with \\d+ maximum context length", "prompt has [\\d,]+ tokens?, but the configured context size is [\\d,]+ tokens?", "model_context_window_exceeded", "prompt too long; exceeded (?:max )?context length", "range of input length should be", "context[_ ]length[_ ]exceeded", "too many tokens", "token limit exceeded" };
pub const nonOverflowPatterns: []const []const u8 = &.{ "^(Throttling error|Service unavailable):", "rate limit", "too many requests" };
const pattern_values = values: {
    var result: [overflowPatterns.len]Pattern = undefined;
    for (overflowPatterns, 0..) |source, index| result[index] = .{ .source = source };
    break :values result;
};
pub fn getOverflowPatterns() []const Pattern {
    return &pattern_values;
}
pub fn isContextOverflow(a: std.mem.Allocator, message: t.AssistantMessage, contextWindow: ?f64) !bool {
    if (message.stopReason == .@"error") if (message.errorMessage) |text| {
        if (text.len > 0 and !(try patterns.nonOverflow(a, null, text))) {
            if (try patterns.overflow(a, null, text)) return true;
            if (std.mem.eql(u8, message.provider, "cerebras") and try patterns.cerebrasBodyless(a, null, text)) return true;
        }
    };
    if (contextWindow) |window| if (window != 0) {
        if (message.stopReason == .stop and message.usage.input + message.usage.cacheRead > window) return true;
        if (message.stopReason == .length and message.usage.output == 0 and message.usage.input + message.usage.cacheRead >= window * 0.99) return true;
    };
    return false;
}
pub fn isRecoverableLength(message: t.AssistantMessage, desiredMaxOutput: f64) bool {
    return message.stopReason == .length and desiredMaxOutput > 0 and message.usage.output < desiredMaxOutput;
}
