const std = @import("std");
const m = @import("../models.zig");
const t = @import("../types.zig");
const events = @import("../utils/event_stream.zig");
pub const Source = struct {
    context: *anyopaque,
    next: *const fn (*anyopaque) anyerror!?t.AssistantMessageEvent,
    result: ?*const fn (*anyopaque) anyerror!t.AssistantMessage = null,
    release: ?*const fn (*anyopaque) void = null,
    /// Transfer ownership of an inner stream to its forwarding outer stream.
    pub fn fromStream(stream: *events.AssistantMessageEventStream) @This() {
        return .{ .context = stream, .next = struct {
            fn call(ctx: *anyopaque) !?t.AssistantMessageEvent {
                const value: *events.AssistantMessageEventStream = @ptrCast(@alignCast(ctx));
                return value.next();
            }
        }.call, .result = struct {
            fn call(ctx: *anyopaque) !t.AssistantMessage {
                const value: *events.AssistantMessageEventStream = @ptrCast(@alignCast(ctx));
                return value.result();
            }
        }.call, .release = struct {
            fn call(ctx: *anyopaque) void {
                const value: *events.AssistantMessageEventStream = @ptrCast(@alignCast(ctx));
                value.destroy();
            }
        }.call };
    }
};
/// Callback context is borrowed until the returned outer stream has joined.
pub const Setup = struct { context: ?*anyopaque = null, call: *const fn (?*anyopaque, m.OperationContext) anyerror!Source };
const Forward = struct {
    operation: m.OperationContext,
    model: t.Model,
    target: *events.AssistantMessageEventStream,
    setup: Setup,
    fn execute(self: *@This()) !void {
        const source = try self.setup.call(self.setup.context, self.operation);
        defer if (source.release) |release| release(source.context);
        while (try source.next(source.context)) |event| try self.target.push(event);
        try self.target.end(if (source.result) |result| try result(source.context) else null);
    }
    fn run(self: *@This()) void {
        self.execute() catch |err| {
            const io = self.operation.io;
            const protection = io.swapCancelProtection(.blocked);
            defer _ = io.swapCancelProtection(protection);
            const base = t.AssistantMessage{ .api = self.model.api, .provider = self.model.provider, .model = self.model.id, .timestamp = @floatFromInt(self.target.started_wall) };
            var message = if (err == error.Canceled or err == error.OutOfMemory) self.target.latestPartial() orelse base else base;
            message.stopReason = if (err == error.Canceled) .aborted else .@"error";
            const fallback: []const u8 = if (err == error.DeferredFetchUnsupported) "API does not support deferred responses" else @errorName(err);
            message.errorMessage = if (err == error.Canceled) "Request was aborted" else if (self.operation.errorMessage) |slot| slot.* orelse fallback else fallback;
            self.target.push(.{ .@"error" = .{ .reason = message.stopReason, .@"error" = message } }) catch self.target.fail(err);
            if (err == error.Canceled) io.recancel();
        };
    }
};
pub fn launch(target: *events.AssistantMessageEventStream, operation: m.OperationContext, model: t.Model, setup: Setup) !void {
    try target.prepareModel(model);
    const a = target.arena.allocator();
    const forward = try a.create(Forward);
    // Inner producers use the caller's backing allocator, rather than the
    // outer publication arena that the forwarding task mutates concurrently.
    forward.* = .{ .operation = operation, .model = try t.cloneInto(t.Model, a, model), .target = target, .setup = setup };
    target.producer = try operation.io.concurrent(Forward.run, .{forward});
}
pub fn lazyStream(operation: m.OperationContext, model: t.Model, setup: Setup) !*events.AssistantMessageEventStream {
    const target = try events.AssistantMessageEventStream.create(operation.allocator, operation.io);
    errdefer target.destroy();
    try launch(target, operation, model, setup);
    return target;
}
pub const LazyApiCapabilities = struct { fetchDeferred: bool = false, cancelDeferred: bool = false };
pub const Loader = struct { context: ?*anyopaque = null, call: *const fn (?*anyopaque, m.OperationContext) anyerror!m.ApiImplementation };
/// Keep this value, its API ID, and the loader context alive until all calls
/// have joined. Zig's module cache supplies static imports; the loader can also
/// select an application-owned implementation on demand.
pub const LazyApi = struct {
    api: []const u8,
    loader: Loader,
    capabilities: LazyApiCapabilities,
    pub fn implementation(self: *@This()) m.ApiImplementation {
        return .{ .api = self.api, .context = self, .stream = stream, .streamSimple = streamSimple, .fetchDeferred = if (self.capabilities.fetchDeferred) fetchDeferred else null, .cancelDeferred = if (self.capabilities.cancelDeferred) cancelDeferred else null };
    }
    const Call = struct {
        owner: *LazyApi,
        model: t.Model,
        context: t.TranscriptContext,
        options: t.StreamOptions,
        kind: enum { stream, simple, deferred },
        deferred: ?t.DeferredHandle = null,
        fetchOptions: t.DeferredFetchOptions = .{},
        fn setup(ctx: ?*anyopaque, operation: m.OperationContext) !Source {
            const self: *@This() = @ptrCast(@alignCast(ctx.?));
            const api = try self.owner.loader.call(self.owner.loader.context, operation);
            const inner = switch (self.kind) {
                .stream => try (api.stream orelse return error.UnsupportedApi)(api.context, operation, self.model, self.context, self.options),
                .simple => try (api.streamSimple orelse return error.UnsupportedApi)(api.context, operation, self.model, self.context, self.options),
                .deferred => try (api.fetchDeferred orelse return error.DeferredFetchUnsupported)(api.context, operation, self.model, self.deferred.?, self.fetchOptions),
            };
            return Source.fromStream(inner);
        }
    };
    fn start(self: *@This(), operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions, kind: @FieldType(Call, "kind"), handle: ?t.DeferredHandle, fetch_options: t.DeferredFetchOptions) !*events.AssistantMessageEventStream {
        const target = try events.AssistantMessageEventStream.create(operation.allocator, operation.io);
        errdefer target.destroy();
        const a = target.arena.allocator();
        const call = try a.create(Call);
        call.* = .{ .owner = self, .model = try t.cloneInto(t.Model, a, model), .context = try t.cloneInto(t.TranscriptContext, a, context), .options = try @import("driver.zig").cloneOptions(a, options), .kind = kind, .deferred = if (handle) |value| try t.cloneInto(t.DeferredHandle, a, value) else null, .fetchOptions = .{ .request = try @import("driver.zig").cloneOptions(a, fetch_options.request), .wait = fetch_options.wait } };
        try launch(target, operation, model, .{ .context = call, .call = Call.setup });
        return target;
    }
    fn stream(ctx: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !*events.AssistantMessageEventStream {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        return self.start(operation, model, context, options, .stream, null, .{});
    }
    fn streamSimple(ctx: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !*events.AssistantMessageEventStream {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        return self.start(operation, model, context, options, .simple, null, .{});
    }
    fn fetchDeferred(ctx: ?*anyopaque, operation: m.OperationContext, model: t.Model, handle: t.DeferredHandle, options: t.DeferredFetchOptions) !*events.AssistantMessageEventStream {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        return self.start(operation, model, .{ .messages = &.{} }, options.request, .deferred, handle, options);
    }
    fn cancelDeferred(ctx: ?*anyopaque, operation: m.OperationContext, model: t.Model, handle: t.DeferredHandle, options: t.DeferredCancelOptions) !void {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        const api = try self.loader.call(self.loader.context, operation);
        const call = api.cancelDeferred orelse {
            if (operation.errorMessage) |slot| slot.* = try operation.allocator.dupe(u8, "API cannot cancel deferred responses");
            return error.DeferredCancelUnsupported;
        };
        return call(api.context, operation, model, handle, options);
    }
};
pub fn lazyApi(api: []const u8, loader: Loader, capabilities: LazyApiCapabilities) LazyApi {
    return .{ .api = api, .loader = loader, .capabilities = capabilities };
}
