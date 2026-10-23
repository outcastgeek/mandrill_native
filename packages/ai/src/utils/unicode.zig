const std = @import("std");
const whitespace = [_]u16{ 0x0009, 0x000a, 0x000b, 0x000c, 0x000d, 0x0020, 0x00a0, 0x1680, 0x2000, 0x2001, 0x2002, 0x2003, 0x2004, 0x2005, 0x2006, 0x2007, 0x2008, 0x2009, 0x200a, 0x2028, 0x2029, 0x202f, 0x205f, 0x3000, 0xfeff };
/// JavaScript string length counts UTF-16 code units.
pub fn utf16Length(bytes: []const u8) !usize {
    return std.unicode.calcWtf16LeLen(bytes);
}
/// ECMAScript String.trim on decoded text, using standard Unicode codecs.
pub fn trimWhitespace(a: std.mem.Allocator, bytes: []const u8) ![]const u8 {
    const units = try std.unicode.wtf8ToWtf16LeAlloc(a, bytes);
    defer a.free(units);
    return std.unicode.wtf16LeToWtf8Alloc(a, std.mem.trim(u16, units, &whitespace));
}
/// ECMAScript String.trimStart on decoded text, using standard Unicode codecs.
pub fn trimWhitespaceStart(a: std.mem.Allocator, bytes: []const u8) ![]const u8 {
    const units = try std.unicode.wtf8ToWtf16LeAlloc(a, bytes);
    defer a.free(units);
    return std.unicode.wtf16LeToWtf8Alloc(a, std.mem.trimStart(u16, units, &whitespace));
}
pub fn splitWhitespace(a: std.mem.Allocator, bytes: []const u8) ![]const []const u8 {
    const units = try std.unicode.wtf8ToWtf16LeAlloc(a, bytes);
    defer a.free(units);
    var parts: std.ArrayList([]const u8) = .empty;
    errdefer {
        for (parts.items) |part| a.free(part);
        parts.deinit(a);
    }
    var tokens = std.mem.tokenizeAny(u16, units, &whitespace);
    while (tokens.next()) |token| {
        const part = try std.unicode.wtf16LeToWtf8Alloc(a, token);
        errdefer a.free(part);
        try parts.append(a, part);
    }
    return parts.toOwnedSlice(a);
}
pub fn isBlank(bytes: []const u8) !bool {
    var iterator = (try std.unicode.Wtf8View.init(bytes)).iterator();
    while (iterator.nextCodepoint()) |point| {
        if (point > std.math.maxInt(u16) or std.mem.findScalar(u16, &whitespace, @intCast(point)) == null) return false;
    }
    return true;
}
pub fn sanitizeSurrogates(a: std.mem.Allocator, bytes: []const u8) ![]const u8 {
    if (std.unicode.utf8ValidateSlice(bytes)) return a.dupe(u8, bytes);
    // Native callers may represent JavaScript's unpaired UTF-16 units as WTF-8.
    // Standard Unicode conversion owns all decoding; this is not JSON repair.
    const units = try std.unicode.wtf8ToWtf16LeAlloc(a, bytes);
    defer a.free(units);
    var iterator = std.unicode.Utf16LeIterator.init(units);
    var output = std.Io.Writer.Allocating.init(a);
    defer output.deinit();
    while (true) {
        const codepoint = (iterator.nextCodepoint() catch continue) orelse break;
        var buffer: [4]u8 = undefined;
        const length = try std.unicode.utf8Encode(codepoint, &buffer);
        try output.writer.writeAll(buffer[0..length]);
    }
    return output.toOwnedSlice();
}
