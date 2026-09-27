//! Host proxy used where Lake requires one executable for LEAN_CC or LEAN_AR.

const std = @import("std");
const options = @import("build_options");

pub fn main(init: std.process.Init) !void {
    const allocator = init.arena.allocator();
    const incoming = try init.minimal.args.toSlice(allocator);
    const tail = if (incoming.len == 0) incoming else incoming[1..];
    var kept: usize = 0;
    for (tail) |argument| if (!excludedHostLibrary(argument)) {
        kept += 1;
    };
    const target_count: usize = if (options.target_triple.len == 0) 0 else 2;
    const cpu_count: usize = if (options.target_cpu.len == 0) 0 else 1;
    const argv = try allocator.alloc([]const u8, 2 + target_count + cpu_count + kept);
    argv[0] = options.zig_exe;
    argv[1] = options.subcommand;
    if (target_count != 0) {
        argv[2] = "-target";
        argv[3] = options.target_triple;
    }
    if (cpu_count != 0) argv[2 + target_count] = try std.fmt.allocPrint(allocator, "-mcpu={s}", .{options.target_cpu});
    var destination = 2 + target_count + cpu_count;
    for (tail) |argument| {
        if (excludedHostLibrary(argument)) continue;
        argv[destination] = argument;
        destination += 1;
    }
    return std.process.replace(init.io, .{ .argv = argv });
}

fn excludedHostLibrary(argument: []const u8) bool {
    return std.mem.eql(u8, argument, "-luv") or
        std.mem.eql(u8, argument, "-lssl") or
        std.mem.eql(u8, argument, "-lcrypto");
}

test "host-only optional libraries are filtered" {
    try std.testing.expect(excludedHostLibrary("-luv"));
    try std.testing.expect(!excludedHostLibrary("-lleanrt"));
}
