//! Unicode hostname predicates.
const std = @import("std");
const re = @import("../regex/regexp.zig");
const unicode = @import("../utils/unicode.zig");
const nfc = @import("../utils/unicode_nfc.zig");

/// The pinned Punycode-label guard rejects these cases before a Unicode
/// decoder could make a difference: empty payload, last delimiter at payload
/// index zero, an ASCII-only payload ending with its delimiter, or ASCII
/// characters forbidden both as encoded digits and as retained label text.
pub const rejected_ace_pattern =
    \\^[Xx][Nn]--(?:|-[^-]*|[\x00-\x7f]*-|[\s\S]*[\x00-\x2a\x3b-\x40\x5b-\x60\x7b-\x7f][\s\S]*)(?![\s\S])
;
const Predicate = enum { european, arabic_number, nonspacing, hebrew, arabic_letter, letter, permitted, invalid, japanese, middle_dot, ace, rejected_ace };
const patterns = [_][]const u8{
    "[0-9\\u{06f0}-\\u{06f9}]",
    "[\\u{0660}-\\u{0669}]",
    "\\p{Mn}",
    "\\p{Script=Hebrew}",
    "[\\p{Script=Arabic}\\p{Script=Syriac}\\p{Script=Thaana}\\p{Script=Mandaic}]",
    "\\p{L}",
    \\^[\p{L}\p{Nd}\p{Mn}\p{Mc}\u{002d}\u{002b}\u{002e}\u{002c}\u{003a}\u{002f}\u{00b7}\u{0375}\u{05f3}\u{05f4}\u{200c}\u{200d}\u{30fb}\u{00df}\u{03c2}\u{06fd}\u{06fe}\u{0f0b}\u{3007}]+(?![\s\S])
    ,
    \\^-|-$|^..--|^[\p{Mn}\p{Mc}\p{Me}]|[\u{0640}\u{07fa}\u{302e}\u{302f}\u{3031}-\u{3035}\u{303b}]|(?<!l)\u{00b7}|\u{00b7}(?!l)|\u{0375}(?!\p{Script=Greek})|(?<!\p{Script=Hebrew})[\u{05f3}\u{05f4}]|(?:^|[\x00-\x7f])\u{200c}|(?<![\u{094d}\u{09cd}\u{0a4d}\u{0acd}\u{0b4d}\u{0bcd}\u{0c4d}\u{0ccd}\u{0d3b}\u{0d3c}\u{0d4d}\u{0dca}\u{1b44}\u{1baa}\u{1bab}\u{a9c0}\u{11046}\u{1107f}\u{110b9}\u{11133}\u{11134}\u{111c0}\u{11235}\u{1134d}\u{11442}\u{114c2}\u{115bf}\u{1163f}\u{116b6}\u{11c3f}\u{11d44}\u{11d45}])\u{200d}
    ,
    "[\\p{Script=Hiragana}\\p{Script=Katakana}\\p{Script=Han}]",
    "\\u{30fb}",
    "^[Xx][Nn]--",
    rejected_ace_pattern,
};

const Predicates = struct {
    expressions: [patterns.len]re.Regex,

    fn init(a: std.mem.Allocator) !Predicates {
        var result: Predicates = undefined;
        var count: usize = 0;
        errdefer for (result.expressions[0..count]) |*expression| expression.deinit();
        for (patterns, &result.expressions) |pattern, *expression| {
            expression.* = try re.Regex.init(a, null, pattern, .{ .unicode = true });
            count += 1;
        }
        return result;
    }

    fn deinit(self: *Predicates) void {
        for (&self.expressions) |*expression| expression.deinit();
    }

    fn @"test"(self: *Predicates, a: std.mem.Allocator, predicate: Predicate, text: []const u8) !bool {
        var match = (try self.expressions[@backingInt(predicate)].exec(a, text, 0)) orelse return false;
        defer match.deinit();
        return true;
    }
};

const Direction = enum { en, an, nsm, r, al, l, on };

fn classify(predicates: *Predicates, a: std.mem.Allocator, point: u21) !Direction {
    var buffer: [4]u8 = undefined;
    const length = std.unicode.wtf8Encode(point, &buffer) catch unreachable;
    const text = buffer[0..length];
    const ordered = [_]Predicate{ .european, .arabic_number, .nonspacing, .hebrew, .arabic_letter, .letter };
    const classes = [_]Direction{ .en, .an, .nsm, .r, .al, .l };
    for (ordered, classes) |predicate, class| if (try predicates.@"test"(a, predicate, text)) return class;
    return .on;
}

fn isRtl(classes: []const Direction) bool {
    for (classes) |class| switch (class) {
        .r, .al, .an => return true,
        else => {},
    };
    return false;
}

fn satisfiesBidi(classes: []const Direction) bool {
    if (classes.len == 0) return true;
    const rtl = switch (classes[0]) {
        .r, .al => true,
        .l => false,
        else => return false,
    };
    var saw_en = false;
    var saw_an = false;
    for (classes) |class| switch (class) {
        .en => saw_en = true,
        .an => {
            if (!rtl) return false;
            saw_an = true;
        },
        .r, .al => if (!rtl) return false,
        .l => if (rtl) return false,
        .nsm, .on => {},
    };
    // The pinned implementation deliberately leaves terminal-class rules out.
    return !(rtl and saw_en and saw_an);
}

fn adapt(initial_delta: u64, points: u64, first: bool) u64 {
    var delta = if (first) initial_delta / 700 else initial_delta / 2;
    delta += delta / points;
    var k: u64 = 0;
    while (delta > 455) {
        delta /= 35;
        k += 36;
    }
    return k + 36 * delta / (delta + 38);
}

/// Measures the Bootstring representation over already-decoded scalar values;
/// this does not construct or parse an ACE label.
fn encodedLength(points: []const u21) usize {
    var basic: u64 = 0;
    for (points) |point| {
        if (point < 128) basic += 1;
    }
    var length: usize = @intCast(basic + @intFromBool(basic != 0));
    var handled = basic;
    var n: u64 = 128;
    var delta: u64 = 0;
    var bias: u64 = 72;
    while (handled < points.len) {
        var minimum: u64 = std.math.maxInt(u64);
        for (points) |point| {
            if (point >= n) minimum = @min(minimum, point);
        }
        delta += (minimum - n) * (handled + 1);
        n = minimum;
        for (points) |point| {
            if (point < n) delta += 1;
            if (point != n) continue;
            var q = delta;
            var k: u64 = 36;
            while (true) : (k += 36) {
                const threshold: u64 = if (k <= bias) 1 else if (k >= bias + 26) 26 else k - bias;
                if (q < threshold) break;
                length += 1;
                q = (q - threshold) / (36 - threshold);
            }
            length += 1;
            bias = adapt(delta, handled + 1, handled == basic);
            delta = 0;
            handled += 1;
        }
        delta += 1;
        n += 1;
    }
    return length;
}

fn writePoint(writer: *std.Io.Writer, point: u21) error{OutOfMemory}!void {
    var buffer: [4]u8 = undefined;
    const length = std.unicode.wtf8Encode(point, &buffer) catch unreachable;
    writer.writeAll(buffer[0..length]) catch return error.OutOfMemory;
}

fn normalize(a: std.mem.Allocator, input: []const u8) ![]const u8 {
    var mapped = std.Io.Writer.Allocating.init(a);
    defer mapped.deinit();
    var iterator = (try std.unicode.Wtf8View.init(input)).iterator();
    while (iterator.nextCodepoint()) |point| try writePoint(&mapped.writer, if (point >= 0xFF01 and point <= 0xFF5E) point - 0xFEE0 else point);
    const composed = try nfc.normalize(a, mapped.written());
    defer a.free(composed);
    var output = std.Io.Writer.Allocating.init(a);
    defer output.deinit();
    iterator = (try std.unicode.Wtf8View.init(composed)).iterator();
    while (iterator.nextCodepoint()) |point| {
        if (point == 0x00AD or point == 0x034F or (point >= 0x180B and point <= 0x180D) or point == 0x200B or
            (point >= 0xFE00 and point <= 0xFE0F) or (point >= 0xE0100 and point <= 0xE01EF)) continue;
        try writePoint(&output.writer, switch (point) {
            0x002E, 0x3002, 0xFF0E, 0xFF61 => '.',
            else => point,
        });
    }
    return output.toOwnedSlice();
}

/// ACE decoding remains an unresolved codec boundary. All ordinary Unicode
/// labels are evaluated with the pinned normalization and character predicates.
pub fn isValid(a: std.mem.Allocator, input: []const u8) !?bool {
    if (input.len == 0 or std.mem.findScalar(u8, input, ' ') != null) return false;
    const normalized = try normalize(a, input);
    defer a.free(normalized);
    if (try unicode.utf16Length(normalized) > 253) return false;
    if (!try re.matches(a, null, "^[^.]+(?:\\.[^.]+)*$", normalized, .{ .unicode = true })) return false;
    var predicates = try Predicates.init(a);
    defer predicates.deinit();
    var label_regex = try re.Regex.init(a, null, "[^.]+", .{ .unicode = true });
    defer label_regex.deinit();
    var start: usize = 0;
    var unknown_ace = false;
    var has_rtl = false;
    var all_satisfy_bidi = true;
    while (try label_regex.exec(a, normalized, start)) |found| {
        var match = found;
        defer match.deinit();
        start = match.captures[0].?.end;
        const label = (try match.text(a, 0)).?;
        defer a.free(label);
        if (try unicode.utf16Length(label) > 63) return false;
        if (try predicates.@"test"(a, .ace, label)) {
            if (try predicates.@"test"(a, .rejected_ace, label)) return false;
            unknown_ace = true;
            continue;
        }
        if (!try predicates.@"test"(a, .permitted, label) or try predicates.@"test"(a, .invalid, label)) return false;
        if (try predicates.@"test"(a, .middle_dot, label) and !try predicates.@"test"(a, .japanese, label)) return false;
        var points: [63]u21 = undefined;
        var classes: [63]Direction = undefined;
        var count: usize = 0;
        var non_ascii = false;
        var iterator = (try std.unicode.Wtf8View.init(label)).iterator();
        while (iterator.nextCodepoint()) |point| {
            points[count] = point;
            classes[count] = try classify(&predicates, a, point);
            count += 1;
            non_ascii = non_ascii or point >= 128;
        }
        if (non_ascii and encodedLength(points[0..count]) + 4 > 63) return false;
        const rtl = isRtl(classes[0..count]);
        const valid_bidi = satisfiesBidi(classes[0..count]);
        if (rtl and !valid_bidi) return false;
        has_rtl = has_rtl or rtl;
        all_satisfy_bidi = all_satisfy_bidi and valid_bidi;
    }
    if (has_rtl and !all_satisfy_bidi) return false;
    return if (unknown_ace) null else true;
}
