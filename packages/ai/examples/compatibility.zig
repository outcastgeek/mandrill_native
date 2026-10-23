//! The older facade uses an explicit registry owned by the application.
const std = @import("std");
const ai = @import("ai");
pub fn main(init: std.process.Init) !void {
    var arena = std.heap.ArenaAllocator.init(init.gpa);
    defer arena.deinit();
    const a = arena.allocator();
    const io = init.io;
    var environment = ai.EnvironmentContext{ .allocator = a, .environ = init.environ_map };
    var registry = try ai.compat.Registry.init(init.gpa, io, .{ .authContext = environment.context() });
    defer registry.deinit();
    var registration = try ai.compat.registerFauxProvider(&registry, .{ .api = "example-compatibility" });
    defer registration.deinit();
    try registration.setResponses(&.{.{ .message = try ai.compat.fauxAssistantMessage(a, io, .{ .string = "An application-owned compatibility registry." }, .{}) }});
    const model = registration.getModel(null) orelse return error.UnknownModel;
    var result = try ai.compat.complete(&registry, a, model, .{ .messages = &.{} }, .{});
    defer result.deinit();
    var buffer: [4096]u8 = undefined;
    var output = std.Io.File.stdout().writerStreaming(io, &buffer);
    try std.json.Stringify.value(result.value(), .{}, &output.interface);
    try output.interface.writeByte('\n');
    try output.interface.flush();
}
