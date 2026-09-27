const std = @import("std");

pub fn main(init: std.process.Init) !void {
    const allocator = init.arena.allocator();
    const args = try init.minimal.args.toSlice(allocator);
    if (args.len != 5) return error.InvalidArguments;
    try verify(allocator, init.io, args[1], null);
    try verify(allocator, init.io, args[2], args[4]);
    try verify(allocator, init.io, args[3], null);
}

fn verify(allocator: std.mem.Allocator, io: std.Io, executable: []const u8, version: ?[]const u8) !void {
    const result = try std.process.run(allocator, io, .{
        .argv = &.{ executable, "--version" },
        .stdout_limit = .limited(1024 * 1024),
        .stderr_limit = .limited(1024 * 1024),
    });
    defer allocator.free(result.stdout);
    defer allocator.free(result.stderr);
    if (result.term != .exited or result.term.exited != 0) return error.ToolCheckFailed;
    if (version) |required| {
        if (std.mem.indexOf(u8, result.stdout, required) == null and std.mem.indexOf(u8, result.stderr, required) == null)
            return error.LeanVersionMismatch;
    }
}
