//! Node path.join composition through standard lexical path normalization.
//! This does not resolve the filesystem or read path contents.
const std = @import("std");
pub fn join(a: std.mem.Allocator, parts: []const []const u8) ![]const u8 {
    if (comptime @import("builtin").os.tag == .windows) {
        const regex = @import("../regex/regexp.zig");
        var components: std.ArrayList([]const u8) = .empty;
        defer components.deinit(a);
        for (parts) |part| if (part.len > 0) {
            try components.append(a, part);
        };
        if (components.items.len == 0) return a.dupe(u8, ".");
        const joined = try std.mem.join(a, "\\", components.items);
        defer a.free(joined);
        // Node join preserves a UNC prefix only when its first nonempty
        // argument supplies exactly two separators and then a nonseparator.
        const unc = try regex.matches(a, null,
            \\^[\\/]{2}[^\\/]
        , components.items[0], .{});
        const tail = std.mem.trimStart(u8, joined, "\\/");
        const prepared = if (!unc and joined.len - tail.len >= 2) try std.fmt.allocPrint(a, "\\{s}", .{tail}) else try a.dupe(u8, joined);
        defer a.free(prepared);
        const original = std.fs.path.parsePathWindows(u8, prepared);
        const drive = try regex.matches(a, null, "^[A-Za-z]:", prepared, .{});
        const ordinary = if (!drive and (original.kind == .drive_absolute or original.kind == .drive_relative)) try std.fmt.allocPrint(a, ".\\{s}", .{prepared}) else try a.dupe(u8, prepared);
        defer a.free(ordinary);
        const normalized = try std.fs.path.resolveAllocWindows(a, &.{ordinary});
        errdefer a.free(normalized);
        if (drive) normalized[0] = prepared[0];
        const relative = !drive and !try regex.matches(a, null,
            \\^[\\/]
        , prepared, .{});
        if (relative and std.mem.findScalar(u8, prepared, ':') != null and
            (try regex.matches(a, null, "^[A-Za-z]:", normalized, .{}) or try regex.matches(a, null,
                \\:(?:[\\/]|$)
            , prepared, .{})))
        {
            const result = try std.fmt.allocPrint(a, ".\\{s}", .{normalized});
            a.free(normalized);
            return result;
        }
        const parsed = std.fs.path.parsePathWindows(u8, normalized);
        if (parsed.kind == .unc_absolute and parsed.root.len == normalized.len and normalized[normalized.len - 1] != '\\') {
            const result = try std.fmt.allocPrint(a, "{s}\\", .{normalized});
            a.free(normalized);
            return result;
        }
        return normalized;
    }
    const joined = try std.fs.path.join(a, parts);
    defer a.free(joined);
    return std.fs.path.resolveAllocPosix(a, &.{joined});
}
