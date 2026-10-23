const std = @import("std");
const ai = @import("ai");
fn resolve(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, _: ai.auth.AuthContext, _: ?ai.auth.ApiKeyCredential) !?ai.auth.AuthResult {
    try io.checkCancel();
    return .{ .auth = .{ .headers = try a.dupe(ai.types.ProviderHeader, &.{.{ .name = "x-example-auth", .value = "example-only" }}) }, .source = "app-owned header authentication" };
}
const AuthCheckOperation = ai.utils.abort.ObservedOperation(?ai.auth.AuthCheck);
/// The embedding UI creates this owner before login and releases it at UI
/// shutdown, after login calls have returned. Its retain/release callbacks own
/// the UI context independently of any individual login's stack or arena.
pub fn createPromptOwner(a: std.mem.Allocator, io: std.Io, callbacks: ai.auth.PromptOwner.Callbacks) !*ai.auth.PromptOwner {
    return ai.auth.PromptOwner.create(a, io, callbacks);
}
/// Browser callback success returns without joining the UI's prompt producer.
/// Completed manual input is copied into a before the waiting handle releases.
pub fn loginWithOwnedPrompts(models: *ai.Models, a: std.mem.Allocator, provider: []const u8, owner: *ai.auth.PromptOwner, options: ai.auth.LoginOptions) !ai.auth.Credential {
    return models.login(a, provider, .oauth, owner.interaction(null), options);
}
const Lookup = struct {
    models: *ai.Models,
    id: []const u8,
    canceled: ?*ai.utils.abort_signals.AbortSignal,
    fn check(context: ?*anyopaque, a: std.mem.Allocator, io: std.Io) !?ai.auth.AuthCheck {
        const self: *@This() = @ptrCast(@alignCast(context.?));
        try io.checkCancel();
        const signal = self.canceled orelse return self.perform(a, io);
        const Outcome = union(enum) { auth: anyerror!?ai.auth.AuthCheck, canceled: std.Io.Cancelable!void };
        var buffer: [2]Outcome = undefined;
        var select = std.Io.Select(Outcome).init(io, &buffer);
        defer select.cancelDiscard();
        try select.concurrent(.auth, perform, .{ self, a, io });
        try select.concurrent(.canceled, std.Io.Event.wait, .{ &signal.event, io });
        return switch (try select.await()) {
            .auth => |result| result,
            .canceled => |result| {
                try result;
                return error.Canceled;
            },
        };
    }
    fn perform(self: *@This(), a: std.mem.Allocator, io: std.Io) !?ai.auth.AuthCheck {
        try io.checkCancel();
        return self.models.checkAuth(a, self.id);
    }
};
pub fn main(init: std.process.Init) !void {
    std.mem.doNotOptimizeAway(&createPromptOwner);
    std.mem.doNotOptimizeAway(&loginWithOwnedPrompts);
    std.mem.doNotOptimizeAway(&ai.auth.PromptOwner.deinit);
    var operation_arena = std.heap.ArenaAllocator.init(init.gpa);
    defer operation_arena.deinit();
    const a = operation_arena.allocator();
    const io = init.io;
    const provider = try ai.createProvider(.{ .id = "custom-auth", .name = "Custom auth", .auth = ai.auth.ProviderAuth{ .apiKey = .{ .name = "Application auth", .resolve = resolve } }, .apis = @as([]const ai.ApiImplementation, &.{ai.ai.api.openai_responses.openAIResponsesApi()}) });
    var environment = ai.EnvironmentContext{ .allocator = a, .environ = init.environ_map };
    var models = try ai.createModels(init.gpa, io, .{ .authContext = environment.context(), .providers = &.{provider} });
    defer models.deinit();
    // A host can set either event, or cancel the task waiting on the result.
    var request_canceled = ai.utils.abort_signals.AbortSignal.init(init.gpa);
    defer request_canceled.deinit();
    var shutdown = ai.utils.abort_signals.AbortSignal.init(init.gpa);
    defer shutdown.deinit();
    var signals = try ai.utils.abort_signals.combineAbortSignals(init.gpa, io, &.{ &request_canceled, &shutdown });
    defer signals.deinit();
    var lookup = Lookup{ .models = &models, .id = provider.id, .canceled = signals.signal };
    var observed = try AuthCheckOperation.init(init.gpa, io, .{ .context = &lookup, .call = Lookup.check });
    defer observed.deinit();
    const configured = (try ai.utils.abort.raceWithAbortSignal(?ai.auth.AuthCheck, &observed, ai.utils.abort.operationSignal(io, io))) orelse return error.NotConfigured;
    var buffer: [4096]u8 = undefined;
    var output = std.Io.File.stdout().writerStreaming(io, &buffer);
    try output.interface.print("{s}: {s}\n", .{ @tagName(configured.type), configured.source orelse "configured" });
    try output.interface.flush();
}
