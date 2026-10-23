//! Canonical Unicode normalization over standard WTF-8 decoded codepoints.
const std = @import("std");
const data = @import("unicode_nfc_data.zig");

const s_base: u21 = 0xAC00;
const l_base: u21 = 0x1100;
const v_base: u21 = 0x1161;
const t_base: u21 = 0x11A7;
const l_count: u21 = 19;
const v_count: u21 = 21;
const t_count: u21 = 28;
const n_count = v_count * t_count;
const s_count = l_count * n_count;

fn findPoint(comptime T: type, items: []const T, point: u21) ?T {
    var low: usize = 0;
    var high = items.len;
    while (low < high) {
        const middle = low + (high - low) / 2;
        const entry = items[middle];
        if (entry.point < point) low = middle + 1 else if (entry.point > point) high = middle else return entry;
    }
    return null;
}

pub fn combiningClass(point: u21) u8 {
    return if (findPoint(data.Class, &data.classes, point)) |entry| entry.value else 0;
}

fn decompose(a: std.mem.Allocator, output: *std.ArrayList(u21), point: u21) error{OutOfMemory}!void {
    if (point >= s_base and point < s_base + s_count) {
        const index = point - s_base;
        try output.append(a, l_base + index / n_count);
        try output.append(a, v_base + (index % n_count) / t_count);
        const trailing = index % t_count;
        if (trailing != 0) try output.append(a, t_base + trailing);
    } else if (findPoint(data.Mapping, &data.mappings, point)) |entry| {
        const start: usize = entry.start;
        for (data.decomposition[start..][0..entry.len]) |part| try decompose(a, output, part);
    } else {
        try output.append(a, point);
    }
}

fn lessClass(_: void, left: u21, right: u21) bool {
    return combiningClass(left) < combiningClass(right);
}

fn compose(first: u21, second: u21) ?u21 {
    if (first >= l_base and first < l_base + l_count and second >= v_base and second < v_base + v_count)
        return s_base + ((first - l_base) * v_count + second - v_base) * t_count;
    if (first >= s_base and first < s_base + s_count and (first - s_base) % t_count == 0 and second > t_base and second < t_base + t_count)
        return first + second - t_base;
    const key = (@as(u64, first) << 21) | @as(u64, second);
    var low: usize = 0;
    var high: usize = data.compositions.len;
    while (low < high) {
        const middle = low + (high - low) / 2;
        const entry = data.compositions[middle];
        if (entry.key < key) low = middle + 1 else if (entry.key > key) high = middle else return entry.point;
    }
    return null;
}

/// Returns an allocator-owned NFC string. Lone JavaScript surrogate units are
/// retained through the standard WTF-8 codec, as in String.prototype.normalize.
pub fn normalize(a: std.mem.Allocator, input: []const u8) ![]const u8 {
    var iterator = (try std.unicode.Wtf8View.init(input)).iterator();
    var points: std.ArrayList(u21) = .empty;
    defer points.deinit(a);
    while (iterator.nextCodepoint()) |point| try decompose(a, &points, point);

    // Each starter bounds a run. Stability preserves the order of equal-class
    // marks, including cases where a preceding mark blocks composition.
    var run_start: usize = 0;
    for (points.items, 0..) |point, index| {
        if (combiningClass(point) == 0) {
            std.sort.block(u21, points.items[run_start..index], {}, lessClass);
            run_start = index + 1;
        }
    }
    std.sort.block(u21, points.items[run_start..], {}, lessClass);

    var count: usize = 0;
    var starter: ?usize = null;
    var last_class: u8 = 0;
    for (points.items) |point| {
        const current_class = combiningClass(point);
        if (starter) |index| {
            if (last_class == 0 or last_class < current_class) {
                if (compose(points.items[index], point)) |composite| {
                    points.items[index] = composite;
                    continue;
                }
            }
        }
        points.items[count] = point;
        if (current_class == 0) starter = count;
        count += 1;
        last_class = current_class;
    }

    var output = std.Io.Writer.Allocating.init(a);
    defer output.deinit();
    for (points.items[0..count]) |point| {
        var buffer: [4]u8 = undefined;
        const length = std.unicode.wtf8Encode(point, &buffer) catch unreachable;
        output.writer.writeAll(buffer[0..length]) catch return error.OutOfMemory;
    }
    return output.toOwnedSlice();
}
