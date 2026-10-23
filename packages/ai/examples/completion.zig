const std = @import("std");
const ai = @import("ai");
pub fn main(init: std.process.Init) !void {
    // Include optional four-provider entrypoints in this compilation graph.
    // Taking their addresses performs no provider operation.
    inline for (.{
        &ai.providers.personalizeOpenRouterCatalog,
        &ai.Models.classify,
        &ai.Models.generateImages,
        &ai.api.google_generative_ai.stream,
        &ai.api.openai_responses.stream,
        &ai.api.openai_completions.stream,
        &ai.api.typesafe_system_one.classify,
        &ai.api.openai_decisions.classify,
    }) |entrypoint| std.mem.doNotOptimizeAway(entrypoint);
    var operation_arena = std.heap.ArenaAllocator.init(init.gpa);
    defer operation_arena.deinit();
    const a = operation_arena.allocator();
    const io = init.io;
    const args = try init.minimal.args.toSlice(a);
    var buffer: [4096]u8 = undefined;
    var output = std.Io.File.stdout().writerStreaming(io, &buffer);
    if (args.len != 4) {
        try output.interface.writeAll("Usage: ai-completion <provider> <model> <prompt>\n");
        try output.interface.flush();
        return;
    }
    var environment = ai.EnvironmentContext{ .allocator = a, .environ = init.environ_map };
    var models = try ai.providers.builtinModels(init.gpa, io, .{ .authContext = environment.context() });
    defer models.deinit();
    const model = (try models.getModel(args[1], args[2])) orelse return error.UnknownModel;
    var result = try models.complete(a, model, .{ .messages = &.{.{ .user = .{ .content = .{ .string = args[3] }, .timestamp = @as(f64, @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds())) } }} }, .{}, .{});
    defer result.deinit();
    try std.json.Stringify.value(result.value(), .{ .whitespace = .indent_2 }, &output.interface);
    try output.interface.writeByte('\n');
    try output.interface.flush();
}
