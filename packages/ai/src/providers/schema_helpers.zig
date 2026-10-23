//! Allocator-owned schema construction shared by provider schema entrypoints.
const std = @import("std");
const j = @import("../json/value.zig");
const s = @import("../schema/type.zig");
pub const Schema = s.TSchema;
pub const Field = struct { name: []const u8, schema: Schema, required: bool = false };
pub fn object(a: std.mem.Allocator, fields: []const Field, options: s.Options) !Schema {
    const children = try a.alloc(s.Property, fields.len);
    defer a.free(children);
    var required: std.ArrayList([]const u8) = .empty;
    defer required.deinit(a);
    for (fields, children) |field, *child| {
        child.* = .{ .name = field.name, .schema = field.schema };
        if (field.required) try required.append(a, field.name);
    }
    return s.Type.Object(a, children, required.items, options);
}
pub fn enumeration(a: std.mem.Allocator, values: []const []const u8) !Schema {
    var result = try literals(a, values, .{});
    result.value = try j.from(a, .{ .@"enum" = values });
    return result;
}
pub fn literals(a: std.mem.Allocator, values: []const []const u8, options: s.Options) !Schema {
    const variants = try a.alloc(Schema, values.len);
    defer a.free(variants);
    for (values, variants) |value, *variant| variant.* = try s.Type.Literal(a, j.str(value));
    return s.Type.UnionWithOptions(a, variants, options);
}
pub fn stringRecord(a: std.mem.Allocator, item: Schema, description: []const u8) !Schema {
    const children = try a.alloc(s.Property, 1);
    children[0] = .{ .name = "^.*$", .schema = item };
    return .{ .kind = .record, .properties = children, .value = try j.from(a, .{ .type = "object", .patternProperties = .{ .@"^.*$" = item }, .description = description }) };
}
