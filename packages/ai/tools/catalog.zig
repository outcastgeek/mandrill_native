const std = @import("std");
const ai = @import("ai");
const validation = ai.ai.catalog_validation;
pub fn main(init: std.process.Init) !void {
    const a = init.arena.allocator();
    const io = init.io;
    const args = try init.minimal.args.toSlice(a);
    var buffer: [4096]u8 = undefined;
    var output = std.Io.File.stdout().writerStreaming(io, &buffer);
    if (args.len < 2) {
        try output.interface.writeAll("Usage: ai-catalog check <data-dir> | list <provider> | import <provider> <input.json> <output-dir> <generated-at>\n");
        try output.interface.flush();
        return;
    }
    if (std.mem.eql(u8, args[1], "check")) {
        if (args.len != 3) return error.InvalidArguments;
        try validation.checkManifest(a, io, args[2]);
        try output.interface.writeAll("Generated model data is valid.\n");
    } else if (std.mem.eql(u8, args[1], "list")) {
        if (args.len != 3) return error.InvalidArguments;
        const models = try ai.providers.getAllBuiltinModels(a, args[2]);
        for (models) |model| try output.interface.print("{s}\t{s}\t{s}\n", .{ @tagName(model.modelType()), model.api, model.id });
    } else if (std.mem.eql(u8, args[1], "import")) {
        if (args.len != 6) return error.InvalidArguments;
        if (ai.providers.catalogBytes(args[2]) == null) return error.UnknownBuiltinProvider;
        const bytes = try std.Io.Dir.cwd().readFileAlloc(io, args[3], a, .unlimited);
        _ = try validation.structure(a, args[2], bytes);
        _ = try ai.datetime.parseRfc3339(a, args[5]);
        const path = try std.fs.path.join(a, &.{ args[4], try std.fmt.allocPrint(a, "{s}.json", .{args[2]}) });
        try std.Io.Dir.cwd().writeFile(io, .{ .sub_path = path, .data = bytes });
        const manifest = try validation.createManifest(a, io, args[4], args[5]);
        const manifest_path = try std.fs.path.join(a, &.{ args[4], ".manifest.json" });
        try std.Io.Dir.cwd().writeFile(io, .{ .sub_path = manifest_path, .data = try ai.json_value.stringify(a, manifest, .{ .whitespace = .indent_2 }) });
    } else return error.UnknownCommand;
    try output.interface.flush();
}
