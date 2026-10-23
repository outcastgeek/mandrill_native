//! Approved UTS46 nontransitional host compatibility, Unicode 17.0.0.
//! CheckBidi/CheckJoiners=true; STD3/CheckHyphens/VerifyDnsLength=false.
const std = @import("std");
const data = @import("idna_data.zig");
const nfc = @import("../utils/unicode_nfc.zig");
const Failure = error{InvalidFormat};
fn range(comptime rows: anytype, cp: u21) ?@TypeOf(rows[0]) {
    var low: usize = 0;
    var high: usize = rows.len;
    while (low < high) {
        const mid = low + (high - low) / 2;
        const item = rows[mid];
        if (cp < item.first) high = mid else if (cp > item.last) low = mid + 1 else return item;
    }
    return null;
}
fn point(out: *std.Io.Writer, cp: u21) error{ OutOfMemory, InvalidFormat }!void {
    var bytes: [4]u8 = undefined;
    const n = std.unicode.utf8Encode(cp, &bytes) catch return error.InvalidFormat;
    out.writeAll(bytes[0..n]) catch return error.OutOfMemory;
}
fn mapped(a: std.mem.Allocator, input: []const u8) ![]const u8 {
    var out = std.Io.Writer.Allocating.init(a);
    defer out.deinit();
    var it = (std.unicode.Utf8View.init(input) catch return error.InvalidFormat).iterator();
    while (it.nextCodepoint()) |cp| {
        const row = range(data.mapping, cp) orelse return error.InvalidFormat;
        switch (row.status) {
            .ignored => {},
            .mapped => for (data.replacements[row.offset..][0..row.len]) |replacement| try point(&out.writer, replacement),
            .valid, .deviation, .disallowed => try point(&out.writer, cp),
        }
    }
    return nfc.normalize(a, out.written());
}
fn adapt(initial: u64, count: u64, first: bool) u64 {
    var delta = if (first) initial / 700 else initial / 2;
    delta += delta / count;
    var k: u64 = 0;
    while (delta > 455) {
        delta /= 35;
        k += 36;
    }
    return k + 36 * delta / (delta + 38);
}
fn digit(value: u64) u8 {
    return @intCast(if (value < 26) 'a' + value else '0' + value - 26);
}
fn number(byte: u8) ?u64 {
    return if (byte >= 'a' and byte <= 'z') byte - 'a' else if (byte >= 'A' and byte <= 'Z') byte - 'A' else if (byte >= '0' and byte <= '9') byte - '0' + 26 else null;
}
fn plus(left: u64, right: u64) Failure!u64 {
    return std.math.add(u64, left, right) catch error.InvalidFormat;
}
fn times(left: u64, right: u64) Failure!u64 {
    return std.math.mul(u64, left, right) catch error.InvalidFormat;
}
fn encode(a: std.mem.Allocator, points: []const u21) ![]const u8 {
    var out = std.Io.Writer.Allocating.init(a);
    defer out.deinit();
    var basic: u64 = 0;
    for (points) |cp| if (cp < 128) {
        try out.writer.writeByte(@intCast(cp));
        basic += 1;
    };
    var handled = basic;
    if (basic > 0) try out.writer.writeByte('-');
    var n: u64 = 128;
    var delta: u64 = 0;
    var bias: u64 = 72;
    while (handled < points.len) {
        var minimum: u64 = 0x110000;
        for (points) |cp| if (cp >= n) {
            minimum = @min(minimum, cp);
        };
        delta = try plus(delta, try times(minimum - n, handled + 1));
        n = minimum;
        for (points) |cp| {
            if (cp < n) delta = try plus(delta, 1);
            if (cp != n) continue;
            var q = delta;
            var k: u64 = 36;
            while (true) : (k += 36) {
                const t: u64 = if (k <= bias) 1 else if (k >= bias + 26) 26 else k - bias;
                if (q < t) break;
                try out.writer.writeByte(digit(t + (q - t) % (36 - t)));
                q = (q - t) / (36 - t);
            }
            try out.writer.writeByte(digit(q));
            bias = adapt(delta, handled + 1, handled == basic);
            delta = 0;
            handled += 1;
        }
        delta = try plus(delta, 1);
        n += 1;
    }
    return out.toOwnedSlice();
}
fn decode(a: std.mem.Allocator, input: []const u8) ![]const u21 {
    var out: std.ArrayList(u21) = .empty;
    errdefer out.deinit(a);
    var cursor: usize = 0;
    if (std.mem.lastIndexOfScalar(u8, input, '-')) |delimiter| {
        for (input[0..delimiter]) |byte| {
            if (byte >= 128) return error.InvalidFormat;
            try out.append(a, byte);
        }
        cursor = delimiter + 1;
    }
    var n: u64 = 128;
    var index: u64 = 0;
    var bias: u64 = 72;
    while (cursor < input.len) {
        const old = index;
        var weight: u64 = 1;
        var k: u64 = 36;
        while (true) : (k += 36) {
            if (cursor == input.len) return error.InvalidFormat;
            const d = number(input[cursor]) orelse return error.InvalidFormat;
            cursor += 1;
            index = try plus(index, try times(d, weight));
            const threshold: u64 = if (k <= bias) 1 else if (k >= bias + 26) 26 else k - bias;
            if (d < threshold) break;
            weight = try times(weight, 36 - threshold);
        }
        const count = out.items.len + 1;
        bias = adapt(index - old, count, old == 0);
        n = try plus(n, index / count);
        index %= count;
        if (n > 0x10ffff or (n >= 0xd800 and n <= 0xdfff)) return error.InvalidFormat;
        try out.insert(a, @intCast(index), @intCast(n));
        index += 1;
    }
    return out.toOwnedSlice(a);
}
fn bidi(cp: u21) data.Bidi {
    return (range(data.bidi, cp) orelse unreachable).value;
}
fn joining(cp: u21) data.Joining {
    return if (range(data.joining, cp)) |row| row.value else .U;
}
fn checkJoiners(points: []const u21) Failure!void {
    for (points, 0..) |cp, index| {
        if (cp != 0x200c and cp != 0x200d) continue;
        if (index > 0 and nfc.combiningClass(points[index - 1]) == 9) continue;
        if (cp == 0x200d) return error.InvalidFormat;
        var left = index;
        while (left > 0 and joining(points[left - 1]) == .T) left -= 1;
        if (left == 0 or (joining(points[left - 1]) != .L and joining(points[left - 1]) != .D)) return error.InvalidFormat;
        var right = index + 1;
        while (right < points.len and joining(points[right]) == .T) right += 1;
        if (right == points.len or (joining(points[right]) != .R and joining(points[right]) != .D)) return error.InvalidFormat;
    }
}
fn checkBidi(points: []const u21) Failure!void {
    if (points.len == 0) return;
    const rtl = switch (bidi(points[0])) {
        .R, .AL => true,
        .L => false,
        else => return error.InvalidFormat,
    };
    var en = false;
    var an = false;
    var last = bidi(points[0]);
    for (points) |cp| {
        const class = bidi(cp);
        if (class != .NSM) last = class;
        switch (class) {
            .R, .AL, .AN => if (!rtl) return error.InvalidFormat,
            .L => if (rtl) return error.InvalidFormat,
            .EN, .ES, .CS, .ET, .ON, .BN, .NSM => {},
            else => return error.InvalidFormat,
        }
        if (class == .EN) en = true;
        if (class == .AN) an = true;
    }
    if (rtl) {
        if (en and an) return error.InvalidFormat;
        switch (last) {
            .R, .AL, .EN, .AN => {},
            else => return error.InvalidFormat,
        }
    } else switch (last) {
        .L, .EN => {},
        else => return error.InvalidFormat,
    }
}
fn pointsFrom(a: std.mem.Allocator, text: []const u8) ![]const u21 {
    var out: std.ArrayList(u21) = .empty;
    errdefer out.deinit(a);
    var it = (std.unicode.Utf8View.init(text) catch return error.InvalidFormat).iterator();
    while (it.nextCodepoint()) |cp| try out.append(a, cp);
    return out.toOwnedSlice(a);
}
fn validate(a: std.mem.Allocator, points: []const u21) !void {
    if (points.len == 0) return;
    if (range(data.marks, points[0]) != null) return error.InvalidFormat;
    if (points.len >= 4 and points[0] == 'x' and points[1] == 'n' and points[2] == '-' and points[3] == '-') return error.InvalidFormat;
    var out = std.Io.Writer.Allocating.init(a);
    defer out.deinit();
    for (points) |cp| {
        if (cp == '.') return error.InvalidFormat;
        const status = (range(data.mapping, cp) orelse return error.InvalidFormat).status;
        if (status != .valid and status != .deviation) return error.InvalidFormat;
        try point(&out.writer, cp);
    }
    const normalized = try nfc.normalize(a, out.written());
    defer a.free(normalized);
    if (!std.mem.eql(u8, normalized, out.written())) return error.InvalidFormat;
    try checkJoiners(points);
}
/// Caller owns the ASCII result. No DNS lookup, locale or platform codec.
pub fn toAscii(a: std.mem.Allocator, input: []const u8) ![]const u8 {
    var arena = std.heap.ArenaAllocator.init(a);
    defer arena.deinit();
    const scratch = arena.allocator();
    const normalized = try mapped(scratch, input);
    var labels: std.ArrayList([]const u21) = .empty;
    var split = std.mem.splitScalar(u8, normalized, '.');
    var rtl = false;
    while (split.next()) |label| {
        var points: []const u21 = undefined;
        if (std.mem.startsWith(u8, label, "xn--")) {
            for (label) |byte| if (byte >= 128) return error.InvalidFormat;
            points = try decode(scratch, label[4..]);
            const nonascii = for (points) |cp| {
                if (cp >= 128) break true;
            } else false;
            if (!nonascii) return error.InvalidFormat;
        } else points = try pointsFrom(scratch, label);
        try validate(scratch, points);
        for (points) |cp| switch (bidi(cp)) {
            .R, .AL, .AN => {
                rtl = true;
            },
            else => {},
        };
        try labels.append(scratch, points);
    }
    if (rtl) for (labels.items) |points| try checkBidi(points);
    var out = std.Io.Writer.Allocating.init(a);
    errdefer out.deinit();
    for (labels.items, 0..) |points, index| {
        if (index > 0) try out.writer.writeByte('.');
        const nonascii = for (points) |cp| {
            if (cp >= 128) break true;
        } else false;
        if (nonascii) {
            try out.writer.writeAll("xn--");
            try out.writer.writeAll(try encode(scratch, points));
        } else for (points) |cp| try out.writer.writeByte(@intCast(cp));
    }
    if (out.written().len == 0) return error.InvalidFormat;
    return out.toOwnedSlice();
}
