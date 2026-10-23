const std = @import("std");
const m = @import("../models.zig");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const driver = @import("../api/driver.zig");
const config = @import("../api/azure_openai_responses.zig");
const lazy = @import("../api/lazy.zig");
const events = @import("../utils/event_stream.zig");
const Call = struct {
    allocator: std.mem.Allocator,
    model: t.Model,
    context: t.TranscriptContext,
    options: t.StreamOptions,
    simple: bool,
    callbacks: t.Callbacks,
    deploymentName: ?[]const u8 = null,
    errorMessage: ?[]const u8 = null,
    errorStorage: t.ErrorMessageStorage,
    fn payload(ctx: ?*anyopaque, a: std.mem.Allocator, value: *std.json.Value, model: t.Model) !?std.json.Value {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        var params = try j.objectSpread(a, value.*);
        try j.put(a, &params, "model", j.str(self.deploymentName.?));
        if (self.callbacks.onPayload) |callback| if (try callback(self.callbacks.context, a, &params, model)) |replacement| {
            if (replacement != .null) return replacement;
        };
        return params;
    }
    fn response(ctx: ?*anyopaque, value: t.ProviderResponse, model: t.Model) !void {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        if (self.callbacks.onResponse) |callback| try callback(self.callbacks.context, value, model);
    }
    fn event(ctx: ?*anyopaque, value: std.json.Value, model: t.Model) !void {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        if (self.callbacks.onProviderStreamEvent) |callback| try callback(self.callbacks.context, value, model);
    }
    fn setup(ctx: ?*anyopaque, operation: m.OperationContext) !lazy.Source {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        var temporary = std.heap.ArenaAllocator.init(operation.allocator);
        defer temporary.deinit();
        const a = temporary.allocator();
        var options = try driver.cloneOptions(a, self.options);
        var detail: ?[]const u8 = null;
        options.errorMessage = &detail;
        options.env = try driver.hydrateEndpointEnv(a, operation.io, operation.authContext, options.env, self.model);
        var model = self.model;
        model.baseUrl = config.resolveBaseUrl(a, model, options) catch |err| {
            if (detail) |message| self.errorMessage = try self.errorStorage.copy(message) else if (err == error.AzureBaseUrlRequired) self.errorMessage = "Azure OpenAI base URL is required. Set AZURE_OPENAI_BASE_URL or AZURE_OPENAI_RESOURCE_NAME, or pass azureBaseUrl, azureResourceName, or model.baseUrl.";
            return err;
        };
        const deployment = try config.resolveDeploymentName(a, model, options);
        if (!std.mem.eql(u8, deployment, model.id)) {
            self.deploymentName = try self.allocator.dupe(u8, deployment);
            options.callbacks = .{ .context = self, .onPayload = payload, .onResponse = if (self.callbacks.onResponse != null) response else null, .onProviderStreamEvent = if (self.callbacks.onProviderStreamEvent != null) event else null };
        }
        options.errorMessage = &self.errorMessage;
        const inner = if (self.simple) try driver.streamSimple(null, operation, model, self.context, options) else try driver.stream(null, operation, model, self.context, options);
        return lazy.Source.fromStream(inner);
    }
};
fn start(operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions, simple: bool) !*events.AssistantMessageEventStream {
    const target = try events.AssistantMessageEventStream.create(operation.allocator, operation.io);
    errdefer target.destroy();
    const a = target.arena.allocator();
    const call = try a.create(Call);
    call.* = .{ .allocator = a, .model = try t.cloneInto(t.Model, a, model), .context = try t.cloneInto(t.TranscriptContext, a, context), .options = try driver.cloneOptions(a, options), .callbacks = options.callbacks, .simple = simple, .errorStorage = .{ .allocator = a } };
    var forwarding = operation;
    forwarding.errorMessage = &call.errorMessage;
    forwarding.errorStorage = &call.errorStorage;
    try lazy.launch(target, forwarding, model, .{ .context = call, .call = Call.setup });
    return target;
}
fn stream(_: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !*events.AssistantMessageEventStream {
    return start(operation, model, context, options, false);
}
fn streamSimple(_: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: t.TranscriptContext, options: t.StreamOptions) !*events.AssistantMessageEventStream {
    return start(operation, model, context, options, true);
}
pub fn azureProvider(a: std.mem.Allocator) !m.Provider {
    const provider = try @import("base.zig").create(a, .{ .id = "azure", .name = "Azure", .baseUrl = null, .authName = "Azure OpenAI API key", .env = &.{"AZURE_OPENAI_API_KEY"}, .apis = &.{ "azure-openai-responses", "openai-completions" } }, @embedFile("../../data/azure.json"));
    // base.create allocated this unpublished slice with `a`.
    for (@constCast(provider.apis)) |*api| if (std.mem.eql(u8, api.api, "openai-completions")) {
        api.* = .{ .api = api.api, .stream = stream, .streamSimple = streamSimple };
    };
    return provider;
}
