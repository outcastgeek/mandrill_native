//! Public Responses conversions and decoded-event stream processing.
const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const reducer = @import("reducer.zig");
const events = @import("../utils/event_stream.zig");
pub const ConvertResponsesMessagesOptions = @import("openai_responses.zig").ConvertResponsesMessagesOptions;
pub const ConvertResponsesToolsOptions = @import("openai_responses.zig").ConvertResponsesToolsOptions;
pub const convertResponsesMessages = @import("openai_responses.zig").convertResponsesMessages;
pub const convertResponsesTools = @import("openai_responses.zig").convertResponsesTools;
pub const OpenAIResponsesStreamOptions = struct {
    context: ?*anyopaque = null,
    onProviderStreamEvent: ?*const fn (?*anyopaque, std.json.Value, t.Model) anyerror!void = null,
    serviceTier: ?[]const u8 = null,
    grammarToolInputProperties: std.StringHashMapUnmanaged([]const u8) = .{},
    resolveServiceTier: ?*const fn (?*anyopaque, ?[]const u8, ?[]const u8) anyerror!?[]const u8 = null,
    applyServiceTierPricing: ?*const fn (?*anyopaque, *t.Usage, ?[]const u8) anyerror!void = null,
    errorMessage: ?*?[]const u8 = null,
};
/// Each decoded event uses the supplied allocator; the iterator context is
/// borrowed until processing returns. The caller owns framing and stream end.
pub const ResponsesEventIterator = struct { context: ?*anyopaque, next: *const fn (?*anyopaque, std.mem.Allocator, std.Io) anyerror!?std.json.Value };
const PricingBridge = struct {
    machine: *reducer.Machine,
    output: *t.AssistantMessage,
    options: OpenAIResponsesStreamOptions,
    fn resolve(ctx: ?*anyopaque, response_tier: ?[]const u8, request_tier: ?[]const u8) !?[]const u8 {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        self.output.* = self.machine.snapshot();
        return self.options.resolveServiceTier.?(self.options.context, response_tier, request_tier);
    }
    fn apply(ctx: ?*anyopaque, usage: *t.Usage, tier: ?[]const u8) !void {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        self.output.* = self.machine.snapshot();
        defer usage.* = self.output.usage;
        return self.options.applyServiceTierPricing.?(self.options.context, &self.output.usage, tier);
    }
};
/// Emits content events and updates output, preserving partial state on error.
/// Like the source helper this emits no start/done/error event and does not end
/// the caller's stream. Result storage belongs to a.
pub fn processResponsesStream(a: std.mem.Allocator, io: std.Io, input: ResponsesEventIterator, output: *t.AssistantMessage, stream: *events.AssistantMessageEventStream, model: t.Model, options: OpenAIResponsesStreamOptions) !void {
    if (options.errorMessage) |slot| slot.* = null;
    var response_model = model;
    response_model.api = "openai-responses";
    var machine = reducer.Machine.init(a, io, response_model, stream);
    machine.message = try t.cloneInto(t.AssistantMessage, a, output.*);
    machine.started = true;
    try machine.blocks.appendSlice(a, machine.message.content);
    for (machine.message.content) |_| try machine.states.append(a, .{ .closed = true });
    machine.grammarToolInputProperties = options.grammarToolInputProperties;
    var pricing = PricingBridge{ .machine = &machine, .output = output, .options = options };
    if (options.applyServiceTierPricing != null) machine.responsePricing = .{ .context = &pricing, .serviceTier = options.serviceTier, .resolve = if (options.resolveServiceTier != null) PricingBridge.resolve else null, .apply = PricingBridge.apply };
    defer output.* = machine.snapshot();
    while (true) {
        try io.checkCancel();
        const event = (try input.next(input.context, a, io)) orelse break;
        if (options.onProviderStreamEvent) |observe| try observe(options.context, event, model);
        const previous_error = machine.message.errorMessage;
        machine.consume(event) catch |err| {
            if (options.errorMessage) |slot| if (slot.* == null) {
                slot.* = if (err == error.ProviderError or err == error.InvalidResponsesEvent or err == error.InvalidStopReason) machine.message.errorMessage else @errorName(err);
            };
            machine.message.errorMessage = previous_error;
            return err;
        };
        output.* = machine.snapshot();
    }
    if (!machine.terminal) {
        if (options.errorMessage) |slot| slot.* = "OpenAI Responses stream ended before a terminal response event";
        return error.StreamEndedWithoutTerminalEvent;
    }
    if (machine.message.stopReason == .toolUse) for (machine.blocks.items, machine.states.items) |block, state| {
        if (block != .toolCall) continue;
        if (!state.closed or j.get(block.toolCall.jsonFields, "partialJson") != null or j.get(block.toolCall.jsonFields, "customInput") != null) {
            if (options.errorMessage) |slot| slot.* = try std.fmt.allocPrint(a, "OpenAI Responses stream completed with an unfinished tool call: {s} ({s})", .{ block.toolCall.name, block.toolCall.id });
            return error.UnfinishedResponsesToolCall;
        }
    };
}
