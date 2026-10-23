const std = @import("std");
const ai = @import("ai");
pub fn main(init: std.process.Init) !void {
    var operation_arena = std.heap.ArenaAllocator.init(init.gpa);
    defer operation_arena.deinit();
    const a = operation_arena.allocator();
    const io = init.io;
    const args = try init.minimal.args.toSlice(a);
    var buffer: [4096]u8 = undefined;
    var output = std.Io.File.stdout().writerStreaming(io, &buffer);
    if (args.len != 4) {
        try output.interface.writeAll("Usage: ai-streaming <provider> <model> <prompt>\n");
        try output.interface.flush();
        return;
    }
    var environment = ai.EnvironmentContext{ .allocator = a, .environ = init.environ_map };
    var models = try ai.providers.builtinModels(init.gpa, io, .{ .authContext = environment.context() });
    defer models.deinit();
    const model = (try models.getModel(args[1], args[2])) orelse return error.UnknownModel;
    const handle = try models.streamSimple(init.gpa, model, .{ .messages = &.{.{ .user = .{ .content = .{ .string = args[3] }, .timestamp = @as(f64, @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds())) } }} }, .{}, .{});
    defer handle.destroy();
    while (try handle.next()) |event| switch (event) {
        .text_delta => |delta| {
            try output.interface.writeAll(delta.delta);
            try output.interface.flush();
        },
        else => {},
    };
    const result = try handle.result();
    if (result.errorMessage) |message| try output.interface.print("\n{s}\n", .{message});
    try output.interface.writeByte('\n');
    try output.interface.flush();
}
