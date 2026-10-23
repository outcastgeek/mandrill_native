const std = @import("std");
pub fn shortHash(a: std.mem.Allocator, bytes: []const u8) ![]const u8 {
    const units = try std.unicode.wtf8ToWtf16LeAlloc(a, bytes);
    defer a.free(units);
    var h1: u32 = 0xdeadbeef;
    var h2: u32 = 0x41c6ce57;
    for (units) |ch| {
        h1 = (h1 ^ ch) *% 2654435761;
        h2 = (h2 ^ ch) *% 1597334677;
    }
    h1 = ((h1 ^ (h1 >> 16)) *% 2246822507) ^ ((h2 ^ (h2 >> 13)) *% 3266489909);
    h2 = ((h2 ^ (h2 >> 16)) *% 2246822507) ^ ((h1 ^ (h1 >> 13)) *% 3266489909);
    var writer = std.Io.Writer.Allocating.init(a);
    defer writer.deinit();
    try writer.writer.printInt(h2, 36, .lower, .{});
    try writer.writer.printInt(h1, 36, .lower, .{});
    return writer.toOwnedSlice();
}
