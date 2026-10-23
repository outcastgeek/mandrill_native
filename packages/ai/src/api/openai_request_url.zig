//! Request URL construction shared by the pinned OpenAI/Anthropic clients.
const std = @import("std");
const j = @import("../json/value.zig");
pub fn endpoint(a: std.mem.Allocator, base: []const u8, path: []const u8) !std.Uri {
    const uri = @import("../transport/uri.zig");
    const selected = if (base.len > 0) base else "https://api.openai.com/v1";
    const target = try std.mem.concat(a, u8, &.{ selected, if (std.mem.endsWith(u8, selected, "/") and std.mem.startsWith(u8, path, "/")) path[1..] else path });
    var url = try uri.parseHttpUrl(a, target);
    if (url.query) |query| {
        // SDK buildURL first performs string concatenation, then converts
        // URLSearchParams to an ordinary object (last repeated value wins),
        // and serializes its string values with qs's RFC3986 encoding.
        var values = j.map(a);
        for (try uri.decodeFormPairs(a, query)) |pair| try j.put(a, &values, pair.name, j.str(pair.value));
        if (values.object.count() > 0) {
            var pairs: std.ArrayList(uri.Pair) = .empty;
            for (try j.objectEntries(a, values.object)) |entry| try pairs.append(a, .{ .name = entry.key, .value = entry.node.string });
            url.query = .{ .percent_encoded = try uri.encode(a, pairs.items) };
        }
    }
    return url;
}
