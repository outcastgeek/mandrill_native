//! Independent application: only the declared ai module is imported.
const std = @import("std");
const ai = @import("ai");
pub fn main(init: std.process.Init) !void {
    var arena = std.heap.ArenaAllocator.init(init.gpa);
    defer arena.deinit();
    const a = arena.allocator();
    const io = init.io;
    // This application owns UUID ordering state instead of a module global.
    var ids = ai.UuidGenerator{};
    const provider_id = try ids.uuidv7(io, null);
    const provider = try ai.fauxProvider(init.gpa, io, .{ .provider = &provider_id, .api = "independent-example-api" });
    defer provider.deinit();
    try provider.setResponses(&.{.{ .message = try ai.fauxAssistantMessage(a, io, .{ .string = "Independent application-owned provider" }, .{}) }});
    var context = ai.EnvironmentContext{ .allocator = init.gpa, .environ = init.environ_map };
    var models = try ai.createModels(init.gpa, io, .{ .authContext = context.context(), .providers = &.{provider.provider} });
    defer models.deinit();
    var result = try models.complete(a, provider.getModel(null).?, .{ .messages = &.{} }, .{}, .{});
    defer result.deinit();
    var buffer: [1024]u8 = undefined;
    var output = std.Io.File.stdout().writerStreaming(io, &buffer);
    try std.json.Stringify.value(result.value(), .{}, &output.interface);
    try output.interface.flush();
}
