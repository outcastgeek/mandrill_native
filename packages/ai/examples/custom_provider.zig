const std = @import("std");
const ai = @import("ai");
pub fn main(init: std.process.Init) !void {
    var operation_arena = std.heap.ArenaAllocator.init(init.gpa);
    defer operation_arena.deinit();
    const a = operation_arena.allocator();
    const io = init.io;
    const faux = try ai.fauxProvider(init.gpa, io, .{ .provider = "example", .api = "example-scripted-api" });
    defer faux.deinit();
    const provider = try ai.createProvider(ai.CreateProviderOptions{
        .id = faux.provider.id,
        .auth = faux.provider.auth,
        .models = faux.models,
        .apis = faux.provider.apis,
        .context = faux.provider.context,
        .cloneContext = faux.provider.cloneContext,
        .deinitContext = faux.provider.deinitContext,
    });
    try faux.setResponses(&.{.{ .message = try ai.fauxAssistantMessage(a, io, .{ .string = "An app-owned provider with no network request." }, .{}) }});
    var environment = ai.EnvironmentContext{ .allocator = a, .environ = init.environ_map };
    const collection_options = ai.CreateModelsOptions{ .authContext = environment.context(), .providers = &.{provider} };
    var models = try ai.createModels(init.gpa, io, collection_options);
    defer models.deinit();
    const all_models = try models.getAllModels(a, null);
    _ = all_models;
    const request_options: ai.ApiStreamOptions("example-scripted-api") = .{};
    var result = try models.complete(a, faux.getModel(null).?, .{ .messages = &.{} }, request_options, .{});
    defer result.deinit();
    // A direct Provider call takes normalized input and explicit operation
    // state; the Models collection supplies auth resolution on its own path.
    try faux.setResponses(&.{.{ .message = try ai.fauxAssistantMessage(a, io, .{ .string = "A direct provider stream." }, .{}) }});
    const direct = try provider.stream(.{ .allocator = a, .io = io, .transport = models.transport }, faux.getModel(null).?, try ai.normalizeContext(a, .{ .messages = &.{} }), request_options);
    defer direct.destroy();
    const direct_result = try direct.result();
    var buffer: [4096]u8 = undefined;
    var output = std.Io.File.stdout().writerStreaming(io, &buffer);
    try std.json.Stringify.value(result.value(), .{ .whitespace = .indent_2 }, &output.interface);
    try output.interface.writeByte('\n');
    try std.json.Stringify.value(direct_result, .{ .whitespace = .indent_2 }, &output.interface);
    try output.interface.writeByte('\n');
    try output.interface.flush();
}
