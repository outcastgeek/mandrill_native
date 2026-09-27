//! Pass Native Image's compiler calls to Zig. Its macOS ABI query uses the
//! Clang driver spelling -ObjC; Zig cc accepts the equivalent -x spelling.
const std = @import("std");
const options = @import("build_options");

pub fn main(init: std.process.Init) !void {
    const allocator = init.arena.allocator();
    const incoming = try init.minimal.args.toSlice(allocator);
    var args: std.ArrayList([]const u8) = .empty;
    try args.appendSlice(allocator, &.{ options.zig_exe, "cc" });
    var export_list_next = false;
    for (incoming[1..]) |argument| {
        if (export_list_next and @embedFile("native_exports").len != 0) {
            const original_path = if (std.mem.startsWith(u8, argument, "-Wl,")) argument[4..] else argument;
            const cwd = std.Io.Dir.cwd();
            const original = try cwd.readFileAlloc(init.io, original_path, allocator, .limited(1024 * 1024));
            const exports = @embedFile("native_exports");
            var merged: std.Io.Writer.Allocating = .init(allocator);
            try merged.writer.writeAll(original);
            try merged.writer.writeByte('\n');
            var lines = std.mem.tokenizeScalar(u8, exports, '\n');
            while (lines.next()) |symbol| {
                try merged.writer.print("_{s}\n", .{symbol});
            }
            const path = try std.fmt.allocPrint(allocator, "{s}.native", .{original_path});
            try cwd.writeFile(init.io, .{ .sub_path = path, .data = merged.written() });
            try args.append(allocator, try std.fmt.allocPrint(allocator, "-Wl,{s}", .{path}));
            lines = std.mem.tokenizeScalar(u8, exports, '\n');
            // Keep the exported-symbols option adjacent to its path before adding retention roots.
            while (lines.next()) |symbol|
                try args.append(allocator, try std.fmt.allocPrint(allocator, "-Wl,-u,_{s}", .{symbol}));
            export_list_next = false;
            continue;
        }
        export_list_next = std.mem.eql(u8, argument, "-Wl,-exported_symbols_list");
        // Graal's Apple-ld compatibility switches have no Zig linker spelling.
        // Keep strict symbol resolution: the Darwin symbol must actually resolve.
        // Zig owns unwind emission; do not pass Apple's compact-unwind switch.
        if (std.mem.eql(u8, argument, "-Wl,-U,___darwin_check_fd_set_overflow") or
            std.mem.eql(u8, argument, "-Wl,-no_compact_unwind")) continue;
        if (std.mem.eql(u8, argument, "-ObjC")) {
            try args.appendSlice(allocator, &.{ "-x", "objective-c" });
        } else try args.append(allocator, argument);
    }
    return std.process.replace(init.io, .{ .argv = args.items });
}
