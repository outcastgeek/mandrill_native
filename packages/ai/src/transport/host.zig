//! Approved WHATWG special-host adapter. URI boundaries remain in std.Uri.
const std = @import("std");
const idna = @import("host_idna.zig");
pub const Host = union(enum) {
    domain: []const u8,
    ipv4: [4]u8,
    ipv6: [8]u16,
    pub fn component(self: @This(), a: std.mem.Allocator) !std.Uri.Component {
        var out = std.Io.Writer.Allocating.init(a);
        defer out.deinit();
        switch (self) {
            .domain => |domain| return .{ .percent_encoded = try a.dupe(u8, domain) },
            .ipv4 => |bytes| try out.writer.print("{d}.{d}.{d}.{d}", .{ bytes[0], bytes[1], bytes[2], bytes[3] }),
            .ipv6 => |pieces| {
                var best: usize = 8;
                var length: usize = 0;
                var cursor: usize = 0;
                while (cursor < 8) {
                    if (pieces[cursor] != 0) {
                        cursor += 1;
                        continue;
                    }
                    const first = cursor;
                    while (cursor < 8 and pieces[cursor] == 0) cursor += 1;
                    if (cursor - first > length and cursor - first > 1) {
                        best = first;
                        length = cursor - first;
                    }
                }
                try out.writer.writeByte('[');
                cursor = 0;
                while (cursor < 8) {
                    if (cursor == best) {
                        try out.writer.writeAll("::");
                        cursor += length;
                        if (cursor == 8) break;
                    } else if (cursor > 0) try out.writer.writeByte(':');
                    try out.writer.print("{x}", .{pieces[cursor]});
                    cursor += 1;
                }
                try out.writer.writeByte(']');
            },
        }
        return .{ .percent_encoded = try out.toOwnedSlice() };
    }
};
fn ipv4Number(text: []const u8) !u64 {
    if (text.len == 0) return error.InvalidFormat;
    var digits = text;
    var base: u8 = 10;
    if (text.len >= 2 and text[0] == '0') {
        if (text[1] == 'x' or text[1] == 'X') {
            digits = text[2..];
            base = 16;
        } else {
            digits = text[1..];
            base = 8;
        }
    }
    if (digits.len == 0) return 0;
    for (digits) |byte| _ = std.fmt.charToDigit(byte, base) catch return error.InvalidFormat;
    return std.fmt.parseInt(u64, digits, base) catch return error.InvalidFormat;
}
fn endsInNumber(text: []const u8) bool {
    const domain = if (std.mem.endsWith(u8, text, ".")) text[0 .. text.len - 1] else text;
    const offset = if (std.mem.lastIndexOfScalar(u8, domain, '.')) |dot| dot + 1 else 0;
    const last = domain[offset..];
    if (last.len == 0) return false;
    const digits = for (last) |byte| {
        if (!std.ascii.isDigit(byte)) break false;
    } else true;
    if (digits) return true;
    if (last.len >= 2 and last[0] == '0' and (last[1] == 'x' or last[1] == 'X')) {
        for (last[2..]) |byte| if (!std.ascii.isHex(byte)) return false;
        return true;
    }
    return false;
}
fn ipv4(text: []const u8) ![4]u8 {
    const domain = if (std.mem.endsWith(u8, text, ".")) text[0 .. text.len - 1] else text;
    var split = std.mem.splitScalar(u8, domain, '.');
    var numbers: [4]u64 = undefined;
    var count: usize = 0;
    while (split.next()) |part| {
        if (count == 4) return error.InvalidFormat;
        numbers[count] = try ipv4Number(part);
        count += 1;
    }
    if (count == 0) return error.InvalidFormat;
    for (numbers[0 .. count - 1]) |value| if (value > 255) return error.InvalidFormat;
    const bits: u6 = @intCast(8 * (5 - count));
    if (numbers[count - 1] >= (@as(u64, 1) << bits)) return error.InvalidFormat;
    var value = numbers[count - 1];
    for (numbers[0 .. count - 1], 0..) |part, index| value += part << @as(u6, @intCast(8 * (3 - index)));
    return .{ @truncate(value >> 24), @truncate(value >> 16), @truncate(value >> 8), @truncate(value) };
}
fn section(text: []const u8, pieces: *[8]u16, allow_ipv4: bool) !usize {
    if (text.len == 0) return 0;
    var split = std.mem.splitScalar(u8, text, ':');
    var count: usize = 0;
    while (split.next()) |part| {
        if (part.len == 0 or count == 8) return error.InvalidFormat;
        if (std.mem.findScalar(u8, part, '.') != null) {
            if (!allow_ipv4 or split.next() != null or count > 6) return error.InvalidFormat;
            const address = std.Io.net.Ip4Address.parse(part, 0) catch return error.InvalidFormat;
            pieces[count] = (@as(u16, address.bytes[0]) << 8) | address.bytes[1];
            pieces[count + 1] = (@as(u16, address.bytes[2]) << 8) | address.bytes[3];
            return count + 2;
        }
        if (part.len > 4) return error.InvalidFormat;
        for (part) |byte| if (!std.ascii.isHex(byte)) return error.InvalidFormat;
        pieces[count] = std.fmt.parseInt(u16, part, 16) catch return error.InvalidFormat;
        count += 1;
    }
    return count;
}
fn ipv6(text: []const u8) ![8]u16 {
    var address: [8]u16 = @splat(0);
    if (std.mem.find(u8, text, "::")) |compression| {
        const tail = text[compression + 2 ..];
        if (std.mem.find(u8, tail, "::") != null) return error.InvalidFormat;
        var left: [8]u16 = undefined;
        var right: [8]u16 = undefined;
        const l = try section(text[0..compression], &left, false);
        const r = try section(tail, &right, true);
        if (l + r >= 8) return error.InvalidFormat;
        @memcpy(address[0..l], left[0..l]);
        @memcpy(address[8 - r ..], right[0..r]);
    } else if (try section(text, &address, true) != 8) return error.InvalidFormat;
    return address;
}
/// The returned domain owns a. Addresses are typed values. Percent decoding
/// and UTF8 validation use std; there is no network or locale dependency.
pub fn parse(a: std.mem.Allocator, component: std.Uri.Component) !Host {
    const literal = switch (component) {
        .raw, .percent_encoded => |text| text,
    };
    if (std.mem.startsWith(u8, literal, "[")) {
        if (literal.len < 2 or literal[literal.len - 1] != ']') return error.InvalidFormat;
        return .{ .ipv6 = try ipv6(literal[1 .. literal.len - 1]) };
    }
    var arena = std.heap.ArenaAllocator.init(a);
    defer arena.deinit();
    const scratch = arena.allocator();
    const decoded = try component.toRawMaybeAlloc(scratch);
    if (!std.unicode.utf8ValidateSlice(decoded)) return error.InvalidFormat;
    const domain = try idna.toAscii(scratch, decoded);
    for (domain) |byte| if (byte <= 0x20 or byte == 0x7f or std.mem.findScalar(u8, "#%/:<>?@[\\]^|", byte) != null) return error.InvalidFormat;
    if (endsInNumber(domain)) return .{ .ipv4 = try ipv4(domain) };
    return .{ .domain = try a.dupe(u8, domain) };
}

/// Keep the standard URI parser for all components while bypassing its DNS
/// byte-length limit and accepting URL's empty decimal port. The expression
/// only locates the special host; std.Uri still splits userinfo/path/query.
pub fn parseUri(a: std.mem.Allocator, text: []const u8) !std.Uri {
    const re = @import("../regex/regexp.zig");
    var regex = try re.Regex.init(a, null,
        \\^((?:[A-Za-z][A-Za-z0-9+.-]*:)?//(?:[^/?#]*@)?)(\[[^\]]*\]|[^:/?#@]*)(?::([0-9]*))?([/?#][\s\S]*)?(?![\s\S])
    , .{});
    defer regex.deinit();
    var match = (try regex.exec(a, text, 0)) orelse return error.InvalidFormat;
    defer match.deinit();
    const prefix = (try match.text(a, 1)).?;
    defer a.free(prefix);
    const host = (try match.text(a, 2)).?;
    errdefer a.free(host);
    const port = try match.text(a, 3);
    defer if (port) |bytes| a.free(bytes);
    const tail = try match.text(a, 4);
    defer if (tail) |bytes| a.free(bytes);
    if (std.ascii.startsWithIgnoreCase(prefix, "file:") and port != null) return error.InvalidFormat;
    var out = std.Io.Writer.Allocating.init(a);
    defer out.deinit();
    try out.writer.print("{s}host.invalid", .{prefix});
    if (port) |bytes| if (bytes.len > 0) try out.writer.print(":{s}", .{bytes});
    try out.writer.writeAll(tail orelse "");
    const prepared = try out.toOwnedSlice();
    errdefer a.free(prepared);
    var uri = std.Uri.parse(prepared) catch |err| (std.Uri.parseAfterScheme("", prepared) catch return err);
    uri.host = .{ .percent_encoded = host };
    return uri;
}
