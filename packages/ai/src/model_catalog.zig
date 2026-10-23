const std = @import("std");
const t = @import("types.zig");
const j = @import("json/value.zig");
const Value = std.json.Value;
/// The result owns its map allocation and borrows model values and IDs from
/// groups. Replacement keeps the first key's position, as Object.fromEntries.
fn flattenModelCatalog(a: std.mem.Allocator, groups: Value, kind: t.ModelType) !Value {
    if (groups != .object) return error.InvalidCatalog;
    var result = j.map(a);
    errdefer result.object.deinit(a);
    const group_entries = try j.objectEntries(a, groups.object);
    defer a.free(group_entries);
    for (group_entries) |group| {
        if (group.node.* != .object) return error.InvalidCatalog;
        const model_entries = try j.objectEntries(a, group.node.object);
        defer a.free(model_entries);
        for (model_entries) |entry| {
            const value = entry.node.*;
            if (!std.mem.eql(u8, j.field(value, "type") orelse continue, @tagName(kind))) continue;
            try j.put(a, &result, j.field(value, "id") orelse return error.InvalidCatalog, value);
        }
    }
    return result;
}
pub fn flattenChatModelCatalog(a: std.mem.Allocator, _: []const u8, groups: Value) !Value {
    return flattenModelCatalog(a, groups, .chat);
}
pub fn flattenImageModelCatalog(a: std.mem.Allocator, _: []const u8, groups: Value) !Value {
    return flattenModelCatalog(a, groups, .image);
}
pub fn flattenClassifierModelCatalog(a: std.mem.Allocator, _: []const u8, groups: Value) !Value {
    return flattenModelCatalog(a, groups, .classifier);
}
pub const Catalog = struct {
    arena: std.heap.ArenaAllocator,
    models: []const t.Model,
    pub fn decode(a: std.mem.Allocator, bytes: []const u8) !@This() {
        var arena = std.heap.ArenaAllocator.init(a);
        errdefer arena.deinit();
        const alloc = arena.allocator();
        const groups = try j.parseFromSliceLeaky(Value, alloc, bytes, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always });
        if (groups != .object) return error.InvalidCatalog;
        var models: std.ArrayList(t.Model) = .empty;
        for ([_]t.ModelType{ .chat, .image, .classifier }) |kind| {
            const flattened = try flattenModelCatalog(alloc, groups, kind);
            // Object.values on the flattened record also puts canonical
            // integer model IDs ahead of its other insertion-ordered keys.
            for (try j.objectEntries(alloc, flattened.object)) |entry| {
                try models.append(alloc, try std.json.parseFromValueLeaky(t.Model, alloc, entry.node.*, .{ .ignore_unknown_fields = false }));
            }
        }
        const owned_models = try models.toOwnedSlice(alloc);
        return .{ .arena = arena, .models = owned_models };
    }
    pub fn deinit(self: *@This()) void {
        self.arena.deinit();
        self.* = undefined;
    }
    pub fn get(self: *const @This(), kind: t.ModelType, id: []const u8) ?t.Model {
        for (self.models) |model| if (model.modelType() == kind and std.mem.eql(u8, model.id, id)) return model;
        return null;
    }
};
