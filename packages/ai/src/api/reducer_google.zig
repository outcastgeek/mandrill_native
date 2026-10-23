const std = @import("std");
const j = @import("../json/value.zig");
const c = @import("common.zig");
const Value = std.json.Value;
pub fn consume(self: *@import("reducer.zig").Machine, event: Value) !void {
    if (!j.truthy((try j.fieldValue(self.allocator, self.message, "responseId")) orelse .null)) {
        if (j.get(event, "responseId")) |id| {
            if (j.text(id)) |text| {
                self.message.responseId = text;
                j.markPresent(&self.message, "responseId");
            } else try j.retainField(self.allocator, &self.message, "responseId", id);
        } else j.markAbsent(&self.message, "responseId");
    }
    const candidates = c.array(event, "candidates");
    if (candidates.len > 0) {
        const candidate = candidates[0];
        const content = j.get(candidate, "content") orelse .null;
        for (c.array(content, "parts")) |part| {
            if (part == .null) {
                self.message.errorMessage = "Cannot read properties of null (reading 'text')";
                return error.InvalidGooglePart;
            }
            if (j.get(part, "text")) |raw_text| {
                const shared = @import("google_shared.zig");
                const thinking = shared.isThinkingPart(part);
                const index = try self.textSlot(thinking, true);
                try self.rawTextDelta(index, raw_text, j.field(part, "thoughtSignature"));
            }
            if (j.get(part, "functionCall")) |call| if (j.truthy(call)) {
                if (self.current) |index| try self.end(index);
                const Index = @import("reducer.zig").Machine.ProviderIndex;
                var id = Index.from(call, "id");
                const duplicate = for (self.states.items) |state| {
                    if (state.googleToolId) |prior| if (Index.eql(prior, id)) break true;
                } else false;
                if (!j.truthy(id.value orelse .null) or duplicate) {
                    const ids = self.googleToolCallIds orelse &self.localGoogleToolCallIds;
                    const name = if (j.get(call, "name")) |value| try @import("../utils/diagnostics.zig").formatThrownValue(self.allocator, .{ .value = value }) else "undefined";
                    const timestamp = std.Io.Clock.real.now(self.io).toMilliseconds();
                    const counter = try @import("../utils/numbers.zig").toString(self.allocator, try ids.next(self.io, self.model.api));
                    id = .{ .value = j.str(try std.fmt.allocPrint(self.allocator, "{s}_{d}_{s}", .{ name, timestamp, counter })) };
                }
                const arguments = j.get(call, "args") orelse j.map(self.allocator);
                const supplied_name = j.get(call, "name") orelse .null;
                const name = if (j.truthy(supplied_name)) supplied_name else j.str("");
                var tool = @import("../types.zig").ToolCall{ .id = j.text(id.value.?) orelse "", .name = j.text(name) orelse "", .arguments = if (arguments == .null) j.map(self.allocator) else arguments };
                if (id.value.? != .string) try j.retainField(self.allocator, &tool, "id", id.value.?);
                if (name != .string) try j.retainField(self.allocator, &tool, "name", name);
                if (j.get(part, "thoughtSignature")) |signature| if (j.truthy(signature)) {
                    if (j.text(signature)) |text| {
                        tool.thoughtSignature = text;
                        j.markPresent(&tool, "thoughtSignature");
                    } else try j.retainField(self.allocator, &tool, "thoughtSignature", signature);
                };
                const retained_id = try id.owned(self.allocator);
                const index = try self.begin(.{ .toolCall = tool });
                self.states.items[index].googleToolId = retained_id;
                self.states.items[index].argumentsComplete = true;
                const encoded = try @import("../json/value.zig").stringify(self.allocator, self.blocks.items[index].toolCall.arguments, .{});
                // Google's complete call is already a tree at its start event.
                try self.stream.push(.{ .toolcall_delta = .{ .contentIndex = @floatFromInt(index), .delta = encoded, .partial = self.snapshot() } });
                try self.end(index);
            };
        }
        if (j.get(candidate, "finishReason")) |raw_reason| if (j.truthy(raw_reason)) {
            const reason = j.text(raw_reason) orelse {
                try j.retainField(self.allocator, &self.message, "rawStopReason", raw_reason);
                self.message.errorMessage = try std.fmt.allocPrint(self.allocator, "Unhandled stop reason: {s}", .{try @import("../utils/diagnostics.zig").formatThrownValue(self.allocator, .{ .value = raw_reason })});
                return error.UnhandledGoogleStopReason;
            };
            self.message.rawStopReason = reason;
            j.markPresent(&self.message, "rawStopReason");
            self.message.stopReason = @import("google_shared.zig").mapStopReason(reason) catch |err| {
                self.message.errorMessage = try std.fmt.allocPrint(self.allocator, "Unhandled stop reason: {s}", .{reason});
                return err;
            };
            self.terminal = true;
            if (self.message.stopReason == .stop and self.hasTools()) self.message.stopReason = .toolUse;
        };
    }
    if (j.get(event, "usageMetadata")) |usage| if (j.truthy(usage)) {
        const accounting = @import("../utils/usage.zig");
        const a = self.allocator;
        const reads = accounting.orZero(usage, "cachedContentTokenCount");
        const thoughts = accounting.orZero(usage, "thoughtsTokenCount");
        self.message.usage = .{ .input = try accounting.number(a, accounting.orZero(usage, "promptTokenCount")) - try accounting.number(a, reads) };
        try accounting.set(a, &self.message.usage, "output", try @import("../utils/cost.zig").addValues(a, accounting.orZero(usage, "candidatesTokenCount"), thoughts));
        try accounting.set(a, &self.message.usage, "cacheRead", reads);
        try accounting.set(a, &self.message.usage, "reasoning", thoughts);
        try accounting.set(a, &self.message.usage, "totalTokens", accounting.orZero(usage, "totalTokenCount"));
        _ = try @import("../utils/cost.zig").calculateCost(self.allocator, self.model, &self.message.usage);
    };
}
