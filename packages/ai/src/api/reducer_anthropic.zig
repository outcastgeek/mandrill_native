const std = @import("std");
const j = @import("../json/value.zig");
const c = @import("common.zig");
const Value = std.json.Value;
fn receiver(self: *@import("reducer.zig").Machine, value: Value, key: []const u8, property: []const u8) !Value {
    const child = j.get(value, key);
    if (child == null or child.? == .null) {
        self.message.errorMessage = try std.fmt.allocPrint(self.allocator, "Cannot read properties of {s} (reading '{s}')", .{ if (child == null) "undefined" else "null", property });
        return error.InvalidAnthropicEvent;
    }
    return child.?;
}
pub fn consume(self: *@import("reducer.zig").Machine, event: Value) !void {
    const kind = j.field(event, "type") orelse return;
    if (std.mem.eql(u8, kind, "message_start")) {
        const message = try receiver(self, event, "message", "id");
        self.message.responseId = j.field(message, "id");
        const response_model = j.field(message, "model");
        if (response_model) |model| if (!std.mem.eql(u8, model, self.model.id)) {
            self.message.responseModel = model;
        };
        if (j.get(message, "input_transformations")) |transformations| if (transformations == .array) {
            self.inputTransformations = transformations;
        };
        self.usageModel = null;
        if (response_model) |actual_model| if (!std.mem.eql(u8, actual_model, self.model.id)) for (c.array(self.model.compat orelse .null, "allowedFallbackModels")) |fallback| {
            if (!std.mem.eql(u8, j.field(fallback, "provider") orelse "", self.model.provider) or !std.mem.eql(u8, j.field(fallback, "model") orelse "", actual_model)) continue;
            if (j.get(fallback, "cost")) |cost| if (j.truthy(cost)) {
                var model = self.model;
                model.id = actual_model;
                model.cost = try std.json.parseFromValueLeaky(@import("../types.zig").ModelCost, self.allocator, cost, .{});
                self.usageModel = model;
            };
            break;
        };
        const usage = try receiver(self, message, "usage", "input_tokens");
        const accounting = @import("../utils/usage.zig");
        const a = self.allocator;
        try accounting.set(a, &self.message.usage, "input", accounting.orZero(usage, "input_tokens"));
        try accounting.set(a, &self.message.usage, "output", accounting.orZero(usage, "output_tokens"));
        try accounting.set(a, &self.message.usage, "cacheRead", accounting.orZero(usage, "cache_read_input_tokens"));
        try accounting.set(a, &self.message.usage, "cacheWrite", accounting.orZero(usage, "cache_creation_input_tokens"));
        try accounting.set(a, &self.message.usage, "cacheWrite1h", accounting.orZero(j.get(usage, "cache_creation") orelse .null, "ephemeral_1h_input_tokens"));
        try costs(self);
        return;
    }
    const wire_index = @import("reducer.zig").Machine.ProviderIndex.from(event, "index");
    if (std.mem.eql(u8, kind, "content_block_start")) {
        const block = try receiver(self, event, "content_block", "type");
        const block_type = j.field(block, "type") orelse return;
        if (std.mem.eql(u8, block_type, "fallback")) {
            if (self.blocks.items.len > 0) return error.MidOutputAnthropicFallback;
            return;
        }
        const input = j.get(block, "input") orelse .null;
        _ = if (std.mem.eql(u8, block_type, "text")) try self.beginProviderBlock(.{ .text = .{ .text = j.field(block, "text") orelse "" } }, wire_index) else if (std.mem.eql(u8, block_type, "thinking")) try self.beginProviderBlock(.{ .thinking = .{ .thinking = j.field(block, "thinking") orelse "", .thinkingSignature = j.field(block, "signature") orelse "" } }, wire_index) else if (std.mem.eql(u8, block_type, "redacted_thinking")) try self.beginProviderBlock(.{ .thinking = .{ .thinking = "[Reasoning redacted]", .thinkingSignature = j.field(block, "data"), .redacted = true } }, wire_index) else if (std.mem.eql(u8, block_type, "tool_use")) try self.beginProviderBlock(.{ .toolCall = .{ .id = j.field(block, "id") orelse "", .name = if (self.anthropicOAuth) @import("anthropic_messages.zig").fromClaudeCodeName(j.field(block, "name") orelse "", self.tools) else j.field(block, "name") orelse "", .arguments = if (input == .null) j.map(self.allocator) else input } }, wire_index) else return;
        return;
    }
    if (std.mem.eql(u8, kind, "content_block_delta")) {
        const delta = try receiver(self, event, "delta", "type");
        const delta_type = j.field(delta, "type") orelse return;
        const index = self.findWireIndex(wire_index) orelse return;
        if (std.mem.eql(u8, delta_type, "text_delta") and self.blocks.items[index] == .text) try self.providerDelta(index, j.field(delta, "text") orelse "") else if (std.mem.eql(u8, delta_type, "thinking_delta") and self.blocks.items[index] == .thinking) try self.providerDelta(index, j.field(delta, "thinking") orelse "") else if (std.mem.eql(u8, delta_type, "input_json_delta") and self.blocks.items[index] == .toolCall) try self.providerDelta(index, j.field(delta, "partial_json") orelse "") else if (std.mem.eql(u8, delta_type, "signature_delta") and self.blocks.items[index] == .thinking) {
            const thinking = &self.blocks.items[index].thinking;
            thinking.thinkingSignature = try std.mem.concat(self.allocator, u8, &.{ thinking.thinkingSignature orelse "", j.field(delta, "signature") orelse "" });
        }
        return;
    }
    if (std.mem.eql(u8, kind, "content_block_stop")) {
        if (self.findWireIndex(wire_index)) |index| {
            try self.endProviderBlock(index);
        }
        return;
    }
    if (std.mem.eql(u8, kind, "message_delta")) {
        if (j.get(event, "input_transformations")) |transformations| if (transformations == .array) {
            self.inputTransformations = transformations;
        };
        const delta = try receiver(self, event, "delta", "stop_reason");
        const reason = j.field(delta, "stop_reason") orelse "";
        if (reason.len > 0) {
            self.message.rawStopReason = reason;
            self.message.stopReason = @import("anthropic_messages.zig").mapStopReason(reason) catch |err| {
                self.message.errorMessage = try std.fmt.allocPrint(self.allocator, "Unhandled stop reason: {s}", .{reason});
                return err;
            };
            if (self.message.stopReason == .@"error") {
                const explanation = j.field(j.get(delta, "stop_details") orelse .null, "explanation") orelse "";
                self.message.errorMessage = if (std.mem.eql(u8, reason, "refusal")) (if (explanation.len > 0) explanation else "The model refused to complete the request") else "Provider stopped with: sensitive";
                self.finishReasonMessage = self.message.errorMessage;
            }
        }
        if (j.get(event, "usage")) |usage| if (j.truthy(usage)) {
            const accounting = @import("../utils/usage.zig");
            const a = self.allocator;
            inline for (.{ .{ "input_tokens", "input" }, .{ "output_tokens", "output" }, .{ "cache_read_input_tokens", "cacheRead" }, .{ "cache_creation_input_tokens", "cacheWrite" } }) |names| {
                if (j.getNonNull(usage, names[0])) |raw| try accounting.set(a, &self.message.usage, names[1], raw);
            }
            if (j.getNonNull(j.get(usage, "cache_creation") orelse .null, "ephemeral_1h_input_tokens")) |raw| try accounting.set(a, &self.message.usage, "cacheWrite1h", raw);
            if (j.getNonNull(j.get(usage, "output_tokens_details") orelse .null, "thinking_tokens")) |raw| try accounting.set(a, &self.message.usage, "reasoning", raw);
        };
        try costs(self);
        return;
    }
    if (std.mem.eql(u8, kind, "message_stop")) {
        self.terminal = true;
        return;
    }
}
fn costs(self: *@import("reducer.zig").Machine) !void {
    try @import("../utils/usage.zig").total(self.allocator, &self.message.usage);
    _ = try @import("../utils/cost.zig").calculateCost(self.allocator, self.usageModel orelse self.model, &self.message.usage);
}
