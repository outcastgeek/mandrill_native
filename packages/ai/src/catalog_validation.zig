const std = @import("std");
const j = @import("json/value.zig");
const all = @import("providers/all.zig");
pub const MODEL_DATA_SCHEMA_VERSION = 6;
fn modalityList(value: std.json.Value) bool {
    if (value != .array or value.array.items.len == 0) return false;
    for (value.array.items) |item| {
        const name = j.text(item) orelse return false;
        if (!std.mem.eql(u8, name, "text") and !std.mem.eql(u8, name, "image")) return false;
    }
    return true;
}
fn ordered(a: std.mem.Allocator, value: std.json.Value) !std.json.Value {
    if (value != .object) return error.InvalidCatalogStructure;
    var scratch = std.heap.ArenaAllocator.init(a);
    defer scratch.deinit();
    const temp = scratch.allocator();
    const Key = struct { text: []const u8, units: []u16 };
    const keys = try temp.alloc(Key, value.object.count());
    for (value.object.keys(), keys) |name, *key| {
        const units = try std.unicode.wtf8ToWtf16LeAlloc(temp, name);
        for (units) |*unit| unit.* = std.mem.littleToNative(u16, unit.*);
        key.* = .{ .text = name, .units = units };
    }
    // JavaScript's relational string comparison orders UTF16 code units,
    // including supplementary characters, rather than encoded UTF8 bytes.
    std.mem.sort(Key, keys, {}, struct {
        fn less(_: void, left: Key, right: Key) bool {
            return std.mem.order(u16, left.units, right.units) == .lt;
        }
    }.less);
    var result = j.map(a);
    for (keys) |key| try j.put(a, &result, key.text, value.object.get(key.text).?);
    return result;
}
pub fn structure(a: std.mem.Allocator, provider: []const u8, bytes: []const u8) !std.json.Value {
    const parsed = try @import("json/value.zig").parseFromSliceLeaky(std.json.Value, a, bytes, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always });
    if (parsed != .object) return error.InvalidCatalog;
    var result = j.map(a);
    var groups = parsed.object.iterator();
    while (groups.next()) |group| {
        if (group.value_ptr.* != .object) return error.InvalidCatalogGroup;
        var models = group.value_ptr.object.iterator();
        while (models.next()) |entry| {
            if (result.object.contains(entry.key_ptr.*)) return error.DuplicateCatalogIdentity;
            // Validate the original tree, as the pinned model-data tooling
            // does. Native Model defaults must not hide missing fields, and
            // this checker does not impose extra rules on open metadata.
            const value = entry.value_ptr.*;
            const id = j.field(value, "id") orelse return error.InvalidCatalogModel;
            const kind = j.field(value, "type") orelse return error.InvalidCatalogModel;
            if (!std.mem.eql(u8, provider, j.field(value, "provider") orelse "") or !std.mem.eql(u8, group.key_ptr.*, j.field(value, "api") orelse "") or !std.mem.eql(u8, entry.key_ptr.*, try std.fmt.allocPrint(a, "{s}:{s}", .{ kind, id }))) return error.CatalogIdentityMismatch;
            if ((j.field(value, "name") orelse @as([]const u8, "")).len == 0 or j.field(value, "baseUrl") == null or !modalityList(j.get(value, "input") orelse .null)) return error.InvalidCatalogModel;
            if (std.mem.eql(u8, kind, "image")) {
                const output = j.get(value, "output") orelse return error.InvalidCatalogModel;
                if (!modalityList(output)) return error.InvalidCatalogModel;
                const has_image = for (output.array.items) |modality| {
                    if (std.mem.eql(u8, modality.string, "image")) break true;
                } else false;
                if (!has_image) return error.InvalidCatalogModel;
            } else {
                if (j.get(value, "output") != null) return error.InvalidCatalogModel;
                if (!std.mem.eql(u8, kind, "chat") and !std.mem.eql(u8, kind, "classifier")) return error.InvalidCatalogModel;
                const window = j.number(j.get(value, "contextWindow") orelse .null) orelse return error.InvalidCatalogModel;
                if (!std.math.isFinite(window) or window <= 0) return error.InvalidCatalogModel;
                if (std.mem.eql(u8, kind, "chat")) {
                    if (j.boolean(j.get(value, "reasoning") orelse .null) == null) return error.InvalidCatalogModel;
                    const maximum = j.number(j.get(value, "maxTokens") orelse .null) orelse return error.InvalidCatalogModel;
                    if (!std.math.isFinite(maximum) or maximum <= 0) return error.InvalidCatalogModel;
                }
            }
            const cost = j.get(value, "cost") orelse return error.InvalidCatalogCost;
            if (cost != .object) return error.InvalidCatalogCost;
            for ([_][]const u8{ "input", "output", "cacheRead", "cacheWrite" }) |key| {
                const rate = j.number(j.get(cost, key) orelse .null) orelse return error.InvalidCatalogCost;
                if (!std.math.isFinite(rate)) return error.InvalidCatalogCost;
            }
            try j.put(a, &result, entry.key_ptr.*, j.str(group.key_ptr.*));
        }
    }
    if (result.object.count() == 0) return error.EmptyCatalog;
    return ordered(a, result);
}
fn digest(bytes: []const u8) [64]u8 {
    var hash: [32]u8 = undefined;
    std.crypto.hash.sha2.Sha256.hash(bytes, &hash, .{});
    return std.fmt.bytesToHex(hash, .lower);
}
pub fn createManifest(a: std.mem.Allocator, io: std.Io, directory: []const u8, generated_at: []const u8) !std.json.Value {
    var files = j.map(a);
    var providers = j.map(a);
    for (all.providerIds) |provider| {
        const filename = try std.fmt.allocPrint(a, "{s}.json", .{provider});
        const path = try std.fs.path.join(a, &.{ directory, filename });
        const bytes = try std.Io.Dir.cwd().readFileAlloc(io, path, a, .unlimited);
        try j.put(a, &providers, provider, try structure(a, provider, bytes));
        const hash = digest(bytes);
        try j.put(a, &files, filename, j.str(try a.dupe(u8, &hash)));
    }
    const normalized = try ordered(a, providers);
    const encoded = try @import("json/value.zig").stringify(a, normalized, .{});
    const hash = digest(encoded);
    return j.from(a, .{ .schemaVersion = MODEL_DATA_SCHEMA_VERSION, .generatedAt = generated_at, .structureHash = &hash, .files = try ordered(a, files) });
}
pub fn checkManifest(a: std.mem.Allocator, io: std.Io, directory: []const u8) !void {
    const dir = try std.Io.Dir.cwd().openDir(io, directory, .{ .iterate = true });
    defer dir.close(io);
    var entries = dir.iterate();
    var count: usize = 0;
    while (try entries.next(io)) |entry| {
        if (!std.mem.endsWith(u8, entry.name, ".json") or std.mem.eql(u8, entry.name, ".manifest.json")) continue;
        const provider = entry.name[0 .. entry.name.len - 5];
        const known = for (all.providerIds) |name| {
            if (std.mem.eql(u8, provider, name)) break true;
        } else false;
        if (!known) return error.CatalogFileSetMismatch;
        count += 1;
    }
    if (count != all.providerIds.len) return error.CatalogFileSetMismatch;
    const path = try std.fs.path.join(a, &.{ directory, ".manifest.json" });
    const bytes = try std.Io.Dir.cwd().readFileAlloc(io, path, a, .unlimited);
    const expected = try @import("json/value.zig").parseFromSliceLeaky(std.json.Value, a, bytes, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always });
    if (j.number(j.get(expected, "schemaVersion") orelse .null) != MODEL_DATA_SCHEMA_VERSION) return error.UnsupportedCatalogSchemaVersion;
    const generated = j.field(expected, "generatedAt") orelse return error.CatalogTimestampMissing;
    _ = try @import("utils/datetime.zig").parseRfc3339(a, generated);
    const actual = try createManifest(a, io, directory, generated);
    if (!std.mem.eql(u8, j.field(expected, "structureHash") orelse "", j.field(actual, "structureHash").?)) return error.CatalogStructureHashMismatch;
    const expected_files = j.get(expected, "files") orelse return error.CatalogFilesMissing;
    const actual_files = j.get(actual, "files").?;
    if (expected_files != .object or expected_files.object.count() != actual_files.object.count()) return error.CatalogFileSetMismatch;
    var iterator = actual_files.object.iterator();
    while (iterator.next()) |entry| if (!std.mem.eql(u8, j.field(expected_files, entry.key_ptr.*) orelse "", entry.value_ptr.string)) return error.CatalogFileHashMismatch;
}
