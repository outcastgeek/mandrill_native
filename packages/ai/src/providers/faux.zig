const std = @import("std");
const t = @import("../types.zig");
const m = @import("../models.zig");
const auth = @import("../auth/types.zig");
const j = @import("../json/value.zig");
const events = @import("../utils/event_stream.zig");
pub const FauxContentBlock = t.Content;
pub const FauxModelDefinition = struct { id: []const u8, name: ?[]const u8 = null, reasoning: bool = false, input: @FieldType(t.Model, "input") = &.{ .text, .image }, inputLimits: ?std.json.Value = null, cost: t.ModelCost = .{}, contextWindow: f64 = 128000, maxTokens: f64 = 16384 };
pub const RegisterFauxProviderOptions = struct { api: ?[]const u8 = null, provider: []const u8 = "faux", models: []const FauxModelDefinition = &.{}, deferred: struct { pendingFetches: f64 = 0, pollAfterMs: ?f64 = null } = .{}, tokensPerSecond: ?f64 = null, tokenSize: struct { min: f64 = 3, max: f64 = 5 } = .{} };
pub const FauxProviderState = struct { callCount: f64 = 0, deferredFetchCount: f64 = 0, cancelledDeferred: std.ArrayList(t.DeferredHandle) = .empty };
pub const FauxResponseFactory = struct { context: ?*anyopaque = null, call: *const fn (?*anyopaque, std.mem.Allocator, std.Io, t.TranscriptContext, t.StreamOptions, *FauxProviderState, t.Model) anyerror!t.AssistantMessage };
pub const FauxResponseStep = union(enum) { message: t.AssistantMessage, factory: FauxResponseFactory };
pub fn fauxText(text: []const u8) t.Content {
    return .{ .text = .{ .text = text } };
}
pub fn fauxThinking(thinking: []const u8) t.Content {
    return .{ .thinking = .{ .thinking = thinking } };
}
fn randomId(a: std.mem.Allocator, io: std.Io, prefix: []const u8) ![]const u8 {
    const random = try @import("../utils/uuid.zig").randomV4(io);
    return std.fmt.allocPrint(a, "{s}:{d}:{s}", .{ prefix, std.Io.Clock.real.now(io).toMilliseconds(), random });
}
pub fn fauxToolCall(a: std.mem.Allocator, io: std.Io, name: []const u8, arguments: std.json.Value, id: ?[]const u8) !t.Content {
    return .{ .toolCall = .{ .id = id orelse try randomId(a, io, "tool"), .name = name, .arguments = arguments } };
}
pub const FauxAssistantOptions = struct { stopReason: t.StopReason = .stop, deferred: ?t.DeferredHandle = null, errorMessage: ?[]const u8 = null, responseId: ?[]const u8 = null, timestamp: ?f64 = null };
pub fn fauxAssistantMessage(a: std.mem.Allocator, io: std.Io, content: t.MessageContent, options: FauxAssistantOptions) !t.AssistantMessage {
    if (content == .missing or content == .nullContent) return error.InvalidMessageContent;
    const blocks = if (content == .string) try a.dupe(t.Content, &.{fauxText(content.string)}) else content.blocks;
    return .{ .api = "faux", .provider = "faux", .model = "faux-1", .content = blocks, .stopReason = options.stopReason, .deferred = options.deferred, .errorMessage = options.errorMessage, .responseId = options.responseId, .timestamp = options.timestamp orelse @as(f64, @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds())) };
}
fn renderBlocks(a: std.mem.Allocator, blocks: []const t.Content) ![]const u8 {
    var parts: std.ArrayList([]const u8) = .empty;
    for (blocks) |block| try parts.append(a, switch (block) {
        .text => |value| value.text,
        .thinking => |value| value.thinking,
        .image => |value| try std.fmt.allocPrint(a, "[image:{s}:{d}]", .{ value.mimeType, try @import("../utils/unicode.zig").utf16Length(value.data) }),
        .toolCall => |value| try std.fmt.allocPrint(a, "{s}:{s}", .{ value.name, try @import("../json/value.zig").stringify(a, value.arguments, .{}) }),
        .unknown => return error.UnsupportedFauxContent,
    });
    return std.mem.join(a, "\n", parts.items);
}
fn renderMessage(a: std.mem.Allocator, message: t.Message) ![]const u8 {
    const content = switch (message) {
        .user => |value| switch (value.content) {
            .string => |text| text,
            .blocks => |blocks| try renderBlocks(a, blocks),
            .missing, .nullContent => return error.InvalidMessageContent,
        },
        .assistant => |value| try renderBlocks(a, value.content),
        .toolResult => |value| if (value.content.len == 0) value.toolName else try std.fmt.allocPrint(a, "{s}\n{s}", .{ value.toolName, try renderBlocks(a, value.content) }),
        .system => |value| blk: {
            var parts: std.ArrayList([]const u8) = .empty;
            const text = try @import("../utils/text.zig").getSystemMessageText(a, value);
            if (text.len > 0) try parts.append(a, text);
            for (value.toolsRemoved orelse &.{}) |tool| try parts.append(a, try std.fmt.allocPrint(a, "tool-:{s}", .{try @import("../json/value.zig").stringify(a, tool, .{ .emit_null_optional_fields = false })}));
            for (value.toolsAdded orelse &.{}) |tool| try parts.append(a, try std.fmt.allocPrint(a, "tool+:{s}", .{try @import("../json/value.zig").stringify(a, tool, .{ .emit_null_optional_fields = false })}));
            break :blk try std.mem.join(a, "\n", parts.items);
        },
    };
    return std.fmt.allocPrint(a, "{s}:{s}", .{ @tagName(message), content });
}
pub const FauxProviderHandle = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    arena: std.heap.ArenaAllocator,
    mutex: std.Io.Mutex = .init,
    references: std.atomic.Value(usize) = .init(1),
    provider: m.Provider,
    api: []const u8,
    models: []const t.Model,
    options: RegisterFauxProviderOptions,
    state: FauxProviderState = .{},
    pending: std.ArrayList(FauxResponseStep) = .empty,
    promptCache: std.StringHashMapUnmanaged([]const u16) = .{},
    deferredResponses: std.StringHashMapUnmanaged(*Deferred) = .{},
    const Deferred = struct { mutex: std.Io.Mutex = .init, handle: t.DeferredHandle, step: FauxResponseStep, context: t.TranscriptContext, options: t.StreamOptions, model: t.Model, pending: f64, cancelled: std.atomic.Value(bool) = .init(false), final: ?t.AssistantMessage = null };
    fn retain(ctx: ?*anyopaque, _: std.mem.Allocator, _: std.Io) !?*anyopaque {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        _ = self.references.fetchAdd(1, .monotonic);
        return self;
    }
    pub fn retainHandle(self: *@This()) void {
        _ = self.references.fetchAdd(1, .monotonic);
    }
    fn release(ctx: ?*anyopaque, _: std.mem.Allocator) void {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        self.deinit();
    }
    pub fn deinit(self: *@This()) void {
        if (self.references.fetchSub(1, .acq_rel) != 1) return;
        const a = self.allocator;
        self.arena.deinit();
        a.destroy(self);
    }
    pub fn getModel(self: *@This(), id: ?[]const u8) ?t.Model {
        if (id) |name| {
            if (name.len == 0) return self.models[0];
            for (self.models) |model| if (std.mem.eql(u8, name, model.id)) return model;
            return null;
        }
        return self.models[0];
    }
    fn cloneStep(self: *@This(), step: FauxResponseStep) !FauxResponseStep {
        return switch (step) {
            .message => |value| .{ .message = try t.cloneInto(t.AssistantMessage, self.arena.allocator(), value) },
            .factory => step,
        };
    }
    pub fn setResponses(self: *@This(), responses: []const FauxResponseStep) !void {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        var replacement: std.ArrayList(FauxResponseStep) = .empty;
        for (responses) |step| try replacement.append(self.arena.allocator(), try self.cloneStep(step));
        self.pending = replacement;
    }
    pub fn appendResponses(self: *@This(), responses: []const FauxResponseStep) !void {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        for (responses) |step| try self.pending.append(self.arena.allocator(), try self.cloneStep(step));
    }
    pub fn getPendingResponseCount(self: *@This()) !usize {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        return self.pending.items.len;
    }
    fn usage(self: *@This(), a: std.mem.Allocator, message: *t.AssistantMessage, context: t.TranscriptContext, options: t.StreamOptions) !void {
        var parts: std.ArrayList([]const u8) = .empty;
        for (context.messages) |item| try parts.append(a, try renderMessage(a, item));
        const prompt = try std.unicode.wtf8ToWtf16LeAlloc(a, try std.mem.join(a, "\n\n", parts.items));
        const output = try @import("../utils/unicode.zig").utf16Length(try renderBlocks(a, message.content));
        const tokens = @ceil(@as(f64, @floatFromInt(prompt.len)) / 4);
        var input = tokens;
        var reads: f64 = 0;
        var writes: f64 = 0;
        if (options.sessionId) |session| if (session.len > 0 and options.cacheRetention != .none) {
            try self.mutex.lock(self.io);
            defer self.mutex.unlock(self.io);
            if (self.promptCache.get(session)) |previous| {
                const prefix = std.mem.findDiff(u16, previous, prompt) orelse @min(previous.len, prompt.len);
                reads = @ceil(@as(f64, @floatFromInt(prefix)) / 4);
                writes = @ceil(@as(f64, @floatFromInt(prompt.len - prefix)) / 4);
                input = @max(0, tokens - reads);
            } else writes = tokens;
            try self.promptCache.put(self.arena.allocator(), try self.arena.allocator().dupe(u8, session), try self.arena.allocator().dupe(u16, prompt));
        };
        message.usage = .{ .input = input, .output = @ceil(@as(f64, @floatFromInt(output)) / 4), .cacheRead = reads, .cacheWrite = writes };
        message.usage.totalTokens = input + message.usage.output + reads + writes;
    }
    fn resolve(self: *@This(), a: std.mem.Allocator, step: FauxResponseStep, context: t.TranscriptContext, options: t.StreamOptions, model: t.Model) !t.AssistantMessage {
        const resolved = switch (step) {
            .message => |message| message,
            .factory => |factory| try factory.call(factory.context, a, self.io, context, options, &self.state, model),
        };
        var value = try t.cloneInto(t.AssistantMessage, a, resolved);
        value.api = self.api;
        value.provider = self.provider.id;
        value.model = model.id;
        try self.usage(a, &value, context, options);
        return value;
    }
    fn failure(self: *@This(), model: t.Model, err: anyerror) t.AssistantMessage {
        return .{ .api = self.api, .provider = self.provider.id, .model = model.id, .stopReason = if (err == error.Canceled) .aborted else .@"error", .errorMessage = switch (err) {
            error.Canceled => "Request was aborted",
            error.FauxResponseEndedWithoutStopReason => "Faux response ended without a stop reason",
            else => @errorName(err),
        }, .timestamp = @as(f64, @floatFromInt(std.Io.Clock.real.now(self.io).toMilliseconds())) };
    }
    fn delay(self: *@This(), text: []const u8) !void {
        if (self.options.tokensPerSecond) |rate| if (rate > 0) {
            const units = try @import("../utils/unicode.zig").utf16Length(text);
            const milliseconds = @ceil(@as(f64, @floatFromInt(units)) / 4) / rate * 1000;
            // The source passes this f64 directly to setTimeout. Normalize before
            // integer conversion so a tiny positive rate/overflow delays by 1ms
            // rather than introducing an error absent from the pinned helper.
            const normalized: u64 = if (!std.math.isFinite(milliseconds) or milliseconds < 1 or milliseconds > std.math.maxInt(i32)) 1 else @intFromFloat(milliseconds);
            try @import("../utils/provider_retry.zig").sleep(self.io, normalized);
            return;
        };
        try self.io.checkCancel();
    }
    fn chunks(self: *@This(), a: std.mem.Allocator, text: []const u8) ![]const []const u8 {
        var out: std.ArrayList([]const u8) = .empty;
        const units = try std.unicode.wtf8ToWtf16LeAlloc(a, text);
        defer a.free(units);
        var offset: f64 = 0;
        const lower = self.options.tokenSize.min;
        const upper = self.options.tokenSize.max;
        const minimum = if (std.math.isNan(lower) or std.math.isNan(upper)) std.math.nan(f64) else @max(1, @min(lower, upper));
        const maximum = if (std.math.isNan(minimum) or std.math.isNan(upper)) std.math.nan(f64) else @max(minimum, upper);
        while (offset < @as(f64, @floatFromInt(units.len))) {
            var random: [8]u8 = undefined;
            try self.io.randomSecure(&random);
            const fraction = @as(f64, @floatFromInt(std.mem.readInt(u64, &random, .little) >> 11)) / 9007199254740992.0;
            const tokens = minimum + @floor(fraction * (maximum - minimum + 1));
            const count = if (std.math.isNan(tokens)) std.math.nan(f64) else @max(1, tokens * 4);
            const first = sliceIndex(offset, units.len);
            const end = @max(first, sliceIndex(offset + count, units.len));
            try out.append(a, try std.unicode.wtf16LeToWtf8Alloc(a, units[first..end]));
            offset += count;
        }
        if (out.items.len == 0) try out.append(a, "");
        return out.toOwnedSlice(a);
    }
    fn sliceIndex(value: f64, length: usize) usize {
        if (std.math.isNan(value) or value <= 0) return 0;
        if (value >= @as(f64, @floatFromInt(length))) return length;
        return @intFromFloat(value);
    }
    fn emit(self: *@This(), output_stream: *events.AssistantMessageEventStream, final: t.AssistantMessage) !void {
        const a = output_stream.arena.allocator();
        var partial = final;
        partial.content = &.{};
        partial.stopReason = .pending;
        self.io.checkCancel() catch {
            const protection = self.io.swapCancelProtection(.blocked);
            defer _ = self.io.swapCancelProtection(protection);
            partial.stopReason = .aborted;
            partial.errorMessage = "Request was aborted";
            partial.timestamp = @as(f64, @floatFromInt(std.Io.Clock.real.now(self.io).toMilliseconds()));
            try output_stream.push(.{ .@"error" = .{ .reason = .aborted, .@"error" = partial } });
            self.io.recancel();
            return;
        };
        try output_stream.push(.{ .start = .{ .partial = partial } });
        var blocks: std.ArrayList(t.Content) = .empty;
        for (final.content, 0..) |block, index| {
            try self.io.checkCancel();
            const initial: t.Content = switch (block) {
                .text => .{ .text = .{ .text = "" } },
                .thinking => .{ .thinking = .{ .thinking = "" } },
                .toolCall => |value| .{ .toolCall = .{ .id = value.id, .name = value.name, .arguments = j.map(a) } },
                else => return error.UnsupportedFauxContent,
            };
            try blocks.append(a, initial);
            partial.content = blocks.items;
            switch (block) {
                .text => try output_stream.push(.{ .text_start = .{ .contentIndex = @floatFromInt(index), .partial = partial } }),
                .thinking => try output_stream.push(.{ .thinking_start = .{ .contentIndex = @floatFromInt(index), .partial = partial } }),
                .toolCall => try output_stream.push(.{ .toolcall_start = .{ .contentIndex = @floatFromInt(index), .partial = partial } }),
                else => unreachable,
            }
            const text = switch (block) {
                .text => |value| value.text,
                .thinking => |value| value.thinking,
                .toolCall => |value| try @import("../json/value.zig").stringify(a, value.arguments, .{}),
                else => unreachable,
            };
            const text_units: []const u16 = if (block == .text or block == .thinking) try std.unicode.wtf8ToWtf16LeAlloc(a, text) else &.{};
            var emitted_units: usize = 0;
            for (try self.chunks(a, text)) |chunk| {
                try self.delay(chunk);
                try self.io.checkCancel();
                if (block == .text or block == .thinking) emitted_units += try @import("../utils/unicode.zig").utf16Length(chunk);
                switch (block) {
                    .text => {
                        blocks.items[index].text.text = try std.unicode.wtf16LeToWtf8Alloc(a, text_units[0..emitted_units]);
                        try output_stream.push(.{ .text_delta = .{ .contentIndex = @floatFromInt(index), .delta = chunk, .partial = partial } });
                    },
                    .thinking => {
                        blocks.items[index].thinking.thinking = try std.unicode.wtf16LeToWtf8Alloc(a, text_units[0..emitted_units]);
                        try output_stream.push(.{ .thinking_delta = .{ .contentIndex = @floatFromInt(index), .delta = chunk, .partial = partial } });
                    },
                    .toolCall => try output_stream.push(.{ .toolcall_delta = .{ .contentIndex = @floatFromInt(index), .delta = chunk, .partial = partial } }),
                    else => unreachable,
                }
            }
            switch (block) {
                .text => try output_stream.push(.{ .text_end = .{ .contentIndex = @floatFromInt(index), .content = text, .partial = partial } }),
                .thinking => try output_stream.push(.{ .thinking_end = .{ .contentIndex = @floatFromInt(index), .content = text, .partial = partial } }),
                .toolCall => |value| {
                    blocks.items[index].toolCall.arguments = value.arguments;
                    try output_stream.push(.{ .toolcall_end = .{ .contentIndex = @floatFromInt(index), .toolCall = .{ .toolCall = value }, .partial = partial } });
                },
                else => unreachable,
            }
        }
        if (final.stopReason == .pending) return error.FauxResponseEndedWithoutStopReason;
        if (final.stopReason == .@"error" or final.stopReason == .aborted) try output_stream.push(.{ .@"error" = .{ .reason = final.stopReason, .@"error" = final } }) else try output_stream.push(.{ .done = .{ .reason = final.stopReason, .message = final } });
    }
    const Request = struct { owner: *FauxProviderHandle, handle: *events.AssistantMessageEventStream, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions, step: ?FauxResponseStep = null, deferred: ?t.DeferredHandle = null, errorMessage: ?[]const u8 = null };
    fn producer(request: *Request) void {
        const self = request.owner;
        defer self.deinit();
        self.execute(request) catch |err| {
            const protection = self.io.swapCancelProtection(.blocked);
            defer _ = self.io.swapCancelProtection(protection);
            var message = self.failure(request.model, err);
            if (request.errorMessage) |text| message.errorMessage = text;
            if (err == error.Canceled) if (request.handle.latestPartial()) |partial| {
                message = partial;
                message.stopReason = .aborted;
                message.errorMessage = "Request was aborted";
                message.timestamp = @as(f64, @floatFromInt(std.Io.Clock.real.now(self.io).toMilliseconds()));
            };
            request.handle.push(.{ .@"error" = .{ .reason = message.stopReason, .@"error" = message } }) catch request.handle.fail(err);
            if (err == error.Canceled) self.io.recancel();
        };
    }
    fn deferredMessage(self: *@This(), model: t.Model, handle: t.DeferredHandle) t.AssistantMessage {
        return .{ .api = self.api, .provider = self.provider.id, .model = model.id, .stopReason = .deferred, .deferred = handle, .timestamp = @as(f64, @floatFromInt(std.Io.Clock.real.now(self.io).toMilliseconds())) };
    }
    fn execute(self: *@This(), request: *Request) !void {
        const a = request.handle.arena.allocator();
        if (request.options.callbacks.onResponse) |call| try call(request.options.callbacks.context, .{ .status = 200, .headers = &.{} }, request.model);
        if (request.deferred) |handle| {
            const entry = blk: {
                try self.mutex.lock(self.io);
                defer self.mutex.unlock(self.io);
                break :blk self.deferredResponses.get(handle.id) orelse {
                    request.errorMessage = try std.fmt.allocPrint(a, "Unknown faux deferred response: {s}", .{handle.id});
                    return error.UnknownFauxDeferredResponse;
                };
            };
            if (!std.mem.eql(u8, entry.handle.provider, handle.provider) or !std.mem.eql(u8, entry.handle.modelId, handle.modelId) or !std.mem.eql(u8, entry.handle.api, handle.api)) {
                request.errorMessage = try std.fmt.allocPrint(a, "Unknown faux deferred response: {s}", .{handle.id});
                return error.UnknownFauxDeferredResponse;
            }
            try entry.mutex.lock(self.io);
            var entry_locked = true;
            defer if (entry_locked) entry.mutex.unlock(self.io);
            const cancelled = entry.cancelled.load(.acquire);
            if (cancelled) {
                request.errorMessage = try std.fmt.allocPrint(a, "Faux deferred response was cancelled: {s}", .{handle.id});
                return error.FauxDeferredResponseCancelled;
            }
            const pending = entry.pending;
            if (pending > 0) entry.pending -= 1;
            const cached = entry.final;
            entry_locked = false;
            entry.mutex.unlock(self.io);
            if (pending > 0) return self.emit(request.handle, self.deferredMessage(request.model, entry.handle));
            const final = cached orelse blk: {
                // The source resolves with submission options minus its signal;
                // cancelling a fetch must not cache an aborted resolution.
                const protection = self.io.swapCancelProtection(.blocked);
                defer _ = self.io.swapCancelProtection(protection);
                var options = entry.options;
                options.deferred = null;
                options.callbacks.onResponse = null;
                const value = self.resolve(a, entry.step, entry.context, options, entry.model) catch |err| failure: {
                    if (err == error.OutOfMemory or err == error.Canceled) return err;
                    break :failure self.failure(entry.model, err);
                };
                try entry.mutex.lock(self.io);
                defer entry.mutex.unlock(self.io);
                try self.mutex.lock(self.io);
                defer self.mutex.unlock(self.io);
                const retained = try t.cloneInto(t.AssistantMessage, self.arena.allocator(), value);
                entry.final = retained;
                break :blk retained;
            };
            // The finalized message is immutable and owned by this retained handle.
            // Network-free streaming must not keep cancellation behind its delay.
            return self.emit(request.handle, final);
        }
        const step = request.step orelse {
            var message = self.failure(request.model, error.NoMoreFauxResponsesQueued);
            message.errorMessage = "No more faux responses queued";
            try self.usage(a, &message, request.context, request.options);
            try request.handle.push(.{ .@"error" = .{ .reason = .@"error", .@"error" = message } });
            return;
        };
        if (j.truthy(request.options.deferred orelse .null)) {
            const handle = blk: {
                try self.mutex.lock(self.io);
                defer self.mutex.unlock(self.io);
                const alloc = self.arena.allocator();
                const id = try randomId(alloc, self.io, "deferred");
                const handle = t.DeferredHandle{ .provider = request.model.provider, .modelId = request.model.id, .api = request.model.api, .id = id, .pollAfterMs = self.options.deferred.pollAfterMs };
                const pending = self.options.deferred.pendingFetches;
                const entry = try alloc.create(Deferred);
                entry.* = .{ .handle = try t.cloneInto(t.DeferredHandle, alloc, handle), .step = step, .context = try t.cloneInto(t.TranscriptContext, alloc, request.context), .options = try @import("../api/driver.zig").cloneOptions(alloc, request.options), .model = try t.cloneInto(t.Model, alloc, request.model), .pending = if (std.math.isNan(pending)) pending else @max(0, @floor(pending)) };
                try self.deferredResponses.put(alloc, id, entry);
                break :blk handle;
            };
            return self.emit(request.handle, self.deferredMessage(request.model, handle));
        }
        return self.emit(request.handle, try self.resolve(a, step, request.context, request.options, request.model));
    }
    fn start(self: *@This(), operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions, deferred: ?t.DeferredHandle) !*events.AssistantMessageEventStream {
        const handle = try events.AssistantMessageEventStream.create(operation.allocator, operation.io);
        errdefer handle.destroy();
        try handle.prepareModel(model);
        const a = handle.arena.allocator();
        const request = try a.create(Request);
        request.* = .{ .owner = self, .handle = handle, .model = try t.cloneInto(t.Model, a, model), .context = try t.cloneInto(t.TranscriptContext, a, context), .options = try @import("../api/driver.zig").cloneOptions(a, options), .deferred = if (deferred) |value| try t.cloneInto(t.DeferredHandle, a, value) else null };
        {
            try self.mutex.lock(self.io);
            defer self.mutex.unlock(self.io);
            if (deferred != null) self.state.deferredFetchCount += 1 else {
                self.state.callCount += 1;
                if (self.pending.items.len > 0) request.step = self.pending.orderedRemove(0);
            }
        }
        _ = self.references.fetchAdd(1, .monotonic);
        errdefer self.deinit();
        handle.producer = try operation.io.concurrent(producer, .{request});
        return handle;
    }
    fn stream(ctx: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !*events.AssistantMessageEventStream {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        return self.start(operation, model, context, options, null);
    }
    fn fetchDeferred(ctx: ?*anyopaque, operation: m.OperationContext, model: t.Model, handle: t.DeferredHandle, options: t.DeferredFetchOptions) !*events.AssistantMessageEventStream {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        return self.start(operation, model, .{ .messages = &.{} }, options.request, handle);
    }
    fn cancelDeferred(ctx: ?*anyopaque, operation: m.OperationContext, model: t.Model, handle: t.DeferredHandle, options: t.StreamOptions) !void {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        const entry = blk: {
            try self.mutex.lock(self.io);
            defer self.mutex.unlock(self.io);
            try self.state.cancelledDeferred.append(self.arena.allocator(), try t.cloneInto(t.DeferredHandle, self.arena.allocator(), handle));
            break :blk self.deferredResponses.get(handle.id);
        };
        if (entry) |value| value.cancelled.store(true, .release);
        if (options.callbacks.onResponse) |call| try call(options.callbacks.context, .{ .status = 200, .headers = &.{} }, model);
        try operation.io.checkCancel();
    }
};
fn keyless(_: ?*anyopaque, _: std.mem.Allocator, io: std.Io, _: auth.AuthContext, _: ?auth.ApiKeyCredential) !?auth.AuthResult {
    try io.checkCancel();
    return .{ .auth = .{}, .source = "Faux" };
}
pub fn fauxProvider(a: std.mem.Allocator, io: std.Io, options: RegisterFauxProviderOptions) !*FauxProviderHandle {
    const self = try a.create(FauxProviderHandle);
    errdefer a.destroy(self);
    var arena = std.heap.ArenaAllocator.init(a);
    errdefer arena.deinit();
    const alloc = arena.allocator();
    const api = try alloc.dupe(u8, options.api orelse try randomId(alloc, io, "faux"));
    const definitions = if (options.models.len > 0) options.models else &.{FauxModelDefinition{ .id = "faux-1", .name = "Faux Model" }};
    const model_list = try alloc.alloc(t.Model, definitions.len);
    for (definitions, model_list) |definition, *model| model.* = .{ .id = definition.id, .name = definition.name orelse definition.id, .api = api, .provider = options.provider, .baseUrl = "http://localhost:0", .reasoning = definition.reasoning, .input = definition.input, .inputLimits = definition.inputLimits, .cost = definition.cost, .contextWindow = definition.contextWindow, .maxTokens = definition.maxTokens };
    const owned_models = try t.cloneInto([]const t.Model, alloc, model_list);
    const owned_options = try t.cloneInto(RegisterFauxProviderOptions, alloc, options);
    const provider_id = try alloc.dupe(u8, options.provider);
    const provider = try m.createProvider(.{ .id = provider_id, .name = provider_id, .auth = @import("../auth/types.zig").ProviderAuth{ .apiKey = .{ .name = "Faux", .resolve = keyless } }, .models = owned_models, .context = self, .cloneContext = FauxProviderHandle.retain, .deinitContext = FauxProviderHandle.release, .apis = try alloc.dupe(m.ApiImplementation, &.{.{ .api = api, .context = self, .stream = FauxProviderHandle.stream, .streamSimple = FauxProviderHandle.stream, .fetchDeferred = FauxProviderHandle.fetchDeferred, .cancelDeferred = FauxProviderHandle.cancelDeferred }}) });
    self.* = .{ .allocator = a, .io = io, .arena = arena, .provider = provider, .api = api, .models = owned_models, .options = owned_options };
    return self;
}
/// Direct scripted streams used by the source's exported createFauxCore.
/// Own this value once; streams retain the underlying handle until they join.
pub const FauxCore = struct {
    handle: *FauxProviderHandle,
    api: []const u8,
    provider: []const u8,
    models: []const t.Model,
    state: *FauxProviderState,
    pub fn deinit(self: *@This()) void {
        self.handle.deinit();
        self.* = undefined;
    }
    pub fn getModel(self: @This(), id: ?[]const u8) ?t.Model {
        return self.handle.getModel(id);
    }
    pub fn setResponses(self: @This(), responses: []const FauxResponseStep) !void {
        return self.handle.setResponses(responses);
    }
    pub fn appendResponses(self: @This(), responses: []const FauxResponseStep) !void {
        return self.handle.appendResponses(responses);
    }
    pub fn getPendingResponseCount(self: @This()) !usize {
        return self.handle.getPendingResponseCount();
    }
    pub fn stream(self: @This(), operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !*events.AssistantMessageEventStream {
        return self.handle.start(operation, model, context, options, null);
    }
    pub const streamSimple = stream;
    pub fn fetchDeferred(self: @This(), operation: m.OperationContext, model: t.Model, deferred: t.DeferredHandle, options: t.DeferredFetchOptions) !*events.AssistantMessageEventStream {
        return self.handle.start(operation, model, .{ .messages = &.{} }, options.request, deferred);
    }
    pub fn cancelDeferred(self: @This(), operation: m.OperationContext, model: t.Model, deferred: t.DeferredHandle, options: t.DeferredCancelOptions) !void {
        return FauxProviderHandle.cancelDeferred(self.handle, operation, model, deferred, options);
    }
};
pub fn createFauxCore(a: std.mem.Allocator, io: std.Io, options: RegisterFauxProviderOptions) !FauxCore {
    const handle = try fauxProvider(a, io, options);
    return .{ .handle = handle, .api = handle.api, .provider = handle.provider.id, .models = handle.models, .state = &handle.state };
}
