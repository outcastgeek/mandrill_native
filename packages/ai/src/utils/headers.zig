const std = @import("std");
const t = @import("../types.zig");
pub fn get(source: t.ProviderHeaders, name: []const u8) ?[]const u8 {
    for (source) |header| if (std.ascii.eqlIgnoreCase(header.name, name)) return header.value;
    return null;
}
pub const ByteStringFailure = struct { index: usize, unit: u16 };
/// Standard Unicode supplies the first UTF16 unit outside ByteString range.
pub fn byteStringFailure(a: std.mem.Allocator, text: []const u8) !?ByteStringFailure {
    const units = try std.unicode.wtf8ToWtf16LeAlloc(a, text);
    defer a.free(units);
    for (units, 0..) |unit, index| if (std.mem.littleToNative(u16, unit) > 255) {
        return .{ .index = index, .unit = std.mem.littleToNative(u16, unit) };
    };
    return null;
}
/// The conversion phase can precede header-list grammar/append validation.
pub fn validateByteString(a: std.mem.Allocator, text: []const u8, error_message: ?*?[]const u8) !void {
    if (try byteStringFailure(a, text)) |failure| {
        if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Header string contains a character outside ByteString at index {d}", .{failure.index});
        return error.InvalidHeaderByteString;
    }
}
/// Fetch append semantics over native strings. Standard Unicode determines
/// ByteString range; physical non-ASCII header encoding remains transport-owned.
pub fn appendFetch(a: std.mem.Allocator, list: *std.ArrayList(t.ProviderHeader), name: []const u8, raw: []const u8, error_message: ?*?[]const u8) !void {
    try validateByteString(a, name, error_message);
    try validateByteString(a, raw, error_message);
    const value = std.mem.trim(u8, raw, "\t\r\n ");
    const valid = try @import("../regex/regexp.zig").matches(a, null, "^[!#$%&'*+.^_`|~0-9A-Za-z-]+(?![\\s\\S])", name, .{});
    if (!valid or std.mem.findAny(u8, value, "\x00\r\n") != null) {
        if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Headers.append: \"{s}\" is an invalid header {s}.", .{ if (valid) value else name, if (valid) "value" else "name" });
        return error.InvalidHeader;
    }
    if (!std.ascii.eqlIgnoreCase(name, "set-cookie")) for (list.items) |*header| if (std.ascii.eqlIgnoreCase(name, header.name)) {
        header.value = try std.fmt.allocPrint(a, "{s}, {s}", .{ header.value.?, value });
        return;
    };
    try list.append(a, .{ .name = try std.ascii.allocLowerString(a, name), .value = try a.dupe(u8, value) });
}
pub fn clone(a: std.mem.Allocator, source: t.ProviderHeaders) !t.ProviderHeaders {
    const result = try a.alloc(t.ProviderHeader, source.len);
    var count: usize = 0;
    errdefer {
        for (result[0..count]) |header| {
            a.free(header.name);
            if (header.value) |value| a.free(value);
        }
        a.free(result);
    }
    for (result, source) |*output, input| {
        const name = try a.dupe(u8, input.name);
        errdefer a.free(name);
        output.* = .{ .name = name, .value = if (input.value) |value| try a.dupe(u8, value) else null };
        count += 1;
    }
    return result;
}
/// Later sources replace earlier names case-insensitively; null is a tombstone.
pub fn merge(a: std.mem.Allocator, sources: []const t.ProviderHeaders) !t.ProviderHeaders {
    return combine(a, sources, false);
}
pub fn overlay(a: std.mem.Allocator, sources: []const t.ProviderHeaders) !t.ProviderHeaders {
    return combine(a, sources, true);
}
/// Object spread/assignment retains the first insertion position of an exact
/// key. Keep this stage separate from later case-insensitive header merging.
pub fn spread(a: std.mem.Allocator, sources: []const t.ProviderHeaders) !t.ProviderHeaders {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const j = @import("../json/value.zig");
    var object = j.map(temporary.allocator());
    for (sources) |source| for (source) |header| try j.put(temporary.allocator(), &object, header.name, if (header.value) |value| j.str(value) else .null);
    return fromValue(a, object);
}
fn combine(a: std.mem.Allocator, sources: []const t.ProviderHeaders, keep_tombstones: bool) !t.ProviderHeaders {
    var headers: std.ArrayList(t.ProviderHeader) = .empty;
    errdefer headers.deinit(a);
    for (sources) |source| for (source) |header| {
        var i: usize = 0;
        while (i < headers.items.len) {
            if (std.ascii.eqlIgnoreCase(headers.items[i].name, header.name)) {
                _ = headers.orderedRemove(i);
            } else i += 1;
        }
        if (header.value != null or keep_tombstones) try headers.append(a, header);
    };
    return headers.toOwnedSlice(a);
}
pub fn fromValue(a: std.mem.Allocator, value: ?std.json.Value) !t.ProviderHeaders {
    var result: std.ArrayList(t.ProviderHeader) = .empty;
    errdefer result.deinit(a);
    if (value) |v| if (v == .object) {
        const entries = try @import("../json/value.zig").objectEntries(a, v.object);
        defer a.free(entries);
        for (entries) |entry| try result.append(a, .{ .name = entry.key, .value = if (entry.node.* == .string) entry.node.string else null });
    };
    return result.toOwnedSlice(a);
}
pub fn providerHeadersToRecord(a: std.mem.Allocator, sources: []const t.ProviderHeaders) !?std.json.Value {
    const merged = try merge(a, sources);
    defer a.free(merged);
    if (merged.len == 0) return null;
    const j = @import("../json/value.zig");
    var out = j.map(a);
    for (merged) |header| try j.put(a, &out, try a.dupe(u8, header.name), j.str(try a.dupe(u8, header.value.?)));
    return out;
}
/// Equivalent to iterating Fetch Headers: ordinary duplicate values are joined,
/// names are lowercase, and the resulting record follows name order.
pub fn headersToRecord(a: std.mem.Allocator, source: t.ProviderHeaders) !std.json.Value {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    const j = @import("../json/value.zig");
    var values = j.map(scratch);
    for (source) |header| if (header.value) |value| {
        const name = try std.ascii.allocLowerString(scratch, header.name);
        // The source assigns string values into {}, whose inherited setter
        // ignores __proto__. providerHeadersToRecord uses Object.fromEntries
        // instead and deliberately preserves that own key.
        if (std.mem.eql(u8, name, "__proto__")) continue;
        const joined = if (!std.mem.eql(u8, name, "set-cookie")) (if (j.field(values, name)) |previous| try std.fmt.allocPrint(scratch, "{s}, {s}", .{ previous, value }) else value) else value;
        try j.put(scratch, &values, name, j.str(joined));
    };
    const keys = try scratch.dupe([]const u8, values.object.keys());
    std.mem.sort([]const u8, keys, {}, struct {
        fn less(_: void, left: []const u8, right: []const u8) bool {
            return std.mem.order(u8, left, right) == .lt;
        }
    }.less);
    var out = j.map(a);
    for (keys) |name| try j.put(a, &out, try a.dupe(u8, name), j.str(try a.dupe(u8, j.field(values, name).?)));
    return out;
}
