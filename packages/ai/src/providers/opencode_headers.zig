const std = @import("std");
const t = @import("../types.zig");
const m = @import("../models.zig");
const driver = @import("../api/driver.zig");
const events = @import("../utils/event_stream.zig");
const session_header = "x-opencode-session";

fn withSessionHeader(a: std.mem.Allocator, original: t.StreamOptions) !t.StreamOptions {
    const session = original.sessionId orelse return original;
    if (session.len == 0) return original;
    for (original.headers) |header| if (std.ascii.eqlIgnoreCase(header.name, session_header)) return original;
    var options = original;
    const headers = try a.alloc(t.ProviderHeader, original.headers.len + 1);
    @memcpy(headers[0..original.headers.len], original.headers);
    headers[original.headers.len] = .{ .name = session_header, .value = session };
    options.headers = headers;
    return options;
}
const Wrapped = struct {
    original: m.ProviderStreams,
    fn owner(ctx: ?*anyopaque) *@This() {
        return @ptrCast(@alignCast(ctx.?));
    }
    fn Chat(comptime name: []const u8, comptime Result: type) type {
        return struct {
            fn call(ctx: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !Result {
                const original = owner(ctx).original;
                var temporary = std.heap.ArenaAllocator.init(operation.allocator);
                defer temporary.deinit();
                return @field(original, name).?(original.context, operation, model, context, try withSessionHeader(temporary.allocator(), options));
            }
        };
    }
    fn Forward(comptime name: []const u8, comptime Context: type, comptime Options: type, comptime Result: type) type {
        return struct {
            fn call(ctx: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: Context, options: Options) !Result {
                const original = owner(ctx).original;
                return @field(original, name).?(original.context, operation, model, context, options);
            }
        };
    }
};
/// Storage belongs to a. Keep it and the original opaque context alive through
/// all calls/streams; callbacks preserve the original implementation context.
pub fn withOpenCodeSessionHeader(a: std.mem.Allocator, original: m.ProviderStreams) !m.ProviderStreams {
    const state = try a.create(Wrapped);
    errdefer a.destroy(state);
    state.* = .{ .original = original };
    return .{ .api = try a.dupe(u8, original.api), .context = state, .stream = if (original.stream != null) Wrapped.Chat("stream", *events.AssistantMessageEventStream).call else null, .streamSimple = if (original.streamSimple != null) Wrapped.Chat("streamSimple", *events.AssistantMessageEventStream).call else null, .complete = if (original.complete != null) Wrapped.Chat("complete", t.Owned(t.AssistantMessage)).call else null, .completeSimple = if (original.completeSimple != null) Wrapped.Chat("completeSimple", t.Owned(t.AssistantMessage)).call else null, .generateImages = if (original.generateImages != null) Wrapped.Forward("generateImages", t.ImagesContext, t.ImagesOptions, t.AssistantImages).call else null, .classify = if (original.classify != null) Wrapped.Forward("classify", t.ClassifierContext, t.ClassifierOptions, t.ClassifierResult).call else null, .fetchDeferred = if (original.fetchDeferred != null) Wrapped.Forward("fetchDeferred", t.DeferredHandle, t.DeferredFetchOptions, *events.AssistantMessageEventStream).call else null, .cancelDeferred = if (original.cancelDeferred != null) Wrapped.Forward("cancelDeferred", t.DeferredHandle, t.DeferredCancelOptions, void).call else null };
}
fn stream(_: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !*events.AssistantMessageEventStream {
    var arena = std.heap.ArenaAllocator.init(operation.allocator);
    defer arena.deinit();
    return driver.stream(null, operation, model, context, try withSessionHeader(arena.allocator(), options));
}
fn streamSimple(_: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !*events.AssistantMessageEventStream {
    var arena = std.heap.ArenaAllocator.init(operation.allocator);
    defer arena.deinit();
    return driver.streamSimple(null, operation, model, context, try withSessionHeader(arena.allocator(), options));
}
fn complete(_: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !t.Owned(t.AssistantMessage) {
    var arena = std.heap.ArenaAllocator.init(operation.allocator);
    defer arena.deinit();
    return driver.complete(operation, model, context, try withSessionHeader(arena.allocator(), options));
}
fn completeSimple(_: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !t.Owned(t.AssistantMessage) {
    var arena = std.heap.ArenaAllocator.init(operation.allocator);
    defer arena.deinit();
    return driver.directCompleteSimple(null, operation, model, context, try withSessionHeader(arena.allocator(), options));
}
/// OpenCode's built-in chat APIs share the driver; the provider adapter adds
/// routing headers before either normal or simple dispatch clones its options.
pub fn implementation(api: []const u8) m.ApiImplementation {
    return .{ .api = api, .stream = stream, .streamSimple = streamSimple, .complete = complete, .completeSimple = completeSimple };
}
