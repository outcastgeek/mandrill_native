const std = @import("std");
const uri = @import("../../transport/uri.zig");
pub const Input = struct { code: ?[]const u8 = null, state: ?[]const u8 = null };
pub fn parse(a: std.mem.Allocator, bytes: []const u8) !Input {
    return parseInput(a, bytes, true);
}
pub fn parseOpenRouter(a: std.mem.Allocator, bytes: []const u8) !Input {
    return parseInput(a, bytes, false);
}
fn parseInput(a: std.mem.Allocator, bytes: []const u8, hash_state: bool) !Input {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const alloc = temporary.allocator();
    const value = try @import("../../utils/unicode.zig").trimWhitespace(alloc, bytes);
    const parsed = try parseTrimmed(alloc, value, hash_state);
    const code = if (parsed.code) |text| try a.dupe(u8, text) else null;
    errdefer if (code) |text| a.free(text);
    return .{ .code = code, .state = if (parsed.state) |text| try a.dupe(u8, text) else null };
}
fn parseTrimmed(a: std.mem.Allocator, value: []const u8, hash_state: bool) !Input {
    if (value.len == 0) return .{};
    if (uri.absoluteUrlQuery(a, value)) |query| {
        const component = query orelse return .{};
        return .{ .code = try uri.queryValue(a, component, "code"), .state = try uri.queryValue(a, component, "state") };
    } else |err| {
        if (err == error.OutOfMemory) return err;
    }
    if (hash_state and std.mem.findScalar(u8, value, '#') != null) {
        var fields = std.Io.Reader.fixed(value);
        return .{ .code = (try fields.takeDelimiter('#')).?, .state = (try fields.takeDelimiter('#')) orelse "" };
    }
    if (std.mem.find(u8, value, "code=") != null) {
        // URLSearchParams(string) accepts one leading query marker. A parsed
        // Uri.query has already removed it and must not be stripped again.
        var params = std.Io.Reader.fixed(value);
        if (std.mem.startsWith(u8, value, "?")) params.toss(1);
        const query = std.Uri.Component{ .percent_encoded = params.buffered() };
        return .{ .code = try uri.queryValue(a, query, "code"), .state = try uri.queryValue(a, query, "state") };
    }
    return .{ .code = value };
}
