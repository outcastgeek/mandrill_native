const std = @import("std");
const j = @import("../json/value.zig");
const Value = std.json.Value;
pub const Kind = enum { unsafe, string, number, integer, boolean, null, array, object, record, tuple, union_, literal };
pub const TSchema = struct {
    value: Value,
    kind: Kind = .unsafe,
    properties: []const Property = &.{},
    items: ?*const TSchema = null,
    variants: []const TSchema = &.{},
    pub fn jsonToValue(self: @This(), _: std.mem.Allocator, _: std.json.Stringify.Options) !Value {
        return self.value;
    }
    pub fn jsonStringify(self: @This(), writer: *std.json.Stringify) !void {
        try j.writeCompatible(writer, self.value);
    }
    pub fn jsonParse(a: std.mem.Allocator, source: anytype, options: std.json.ParseOptions) !@This() {
        var configured = options;
        configured.duplicate_field_behavior = .use_last;
        configured.parse_numbers = false;
        var value = try Value.jsonParse(a, source, configured);
        try j.normalizeParsed(&value);
        return .{ .value = value };
    }
    pub fn jsonParseFromValue(_: std.mem.Allocator, value: Value, _: std.json.ParseOptions) !@This() {
        return .{ .value = value };
    }
};
pub const Property = struct { name: []const u8, schema: TSchema };
pub const Options = struct {
    description: ?[]const u8 = null,
    default: ?Value = null,
    extra: ?Value = null,
};
fn finish(a: std.mem.Allocator, schema: TSchema, options: Options) !TSchema {
    var result = schema;
    if (options.description) |value| try j.put(a, &result.value, "description", j.str(try a.dupe(u8, value)));
    if (options.default) |value| try j.put(a, &result.value, "default", try j.clone(a, value));
    if (options.extra) |extra| {
        if (extra != .object) return error.InvalidSchemaOptions;
        const entries = try j.objectEntries(a, extra.object);
        defer a.free(entries);
        for (entries) |entry| {
            // Memory.Create installs the constructor's hidden kind after
            // options, so an option cannot replace it or expose it on wire.
            if (std.mem.eql(u8, entry.key, "~kind")) continue;
            const value = try j.clone(a, entry.node.*);
            try j.put(a, &result.value, try a.dupe(u8, entry.key), value);
            // Structural options replace the actual children, not merely
            // their serialized trees. JSON options carry no native kind.
            // Retaining the constructor's former children would coerce using
            // schemas that Memory.Create has already replaced.
            if (std.mem.eql(u8, entry.key, "items")) switch (result.kind) {
                .array => {
                    const child = try a.create(TSchema);
                    child.* = .{ .value = value };
                    result.items = child;
                },
                .tuple => result.variants = try plainVariants(a, value),
                else => {},
            };
            if (result.kind == .object and std.mem.eql(u8, entry.key, "properties")) {
                result.properties = &.{};
                if (value == .object) {
                    const properties = try j.objectEntries(a, value.object);
                    defer a.free(properties);
                    const children = try a.alloc(Property, properties.len);
                    for (properties, children) |property, *child| child.* = .{ .name = property.key, .schema = .{ .value = property.node.* } };
                    result.properties = children;
                }
            }
            if (result.kind == .union_ and std.mem.eql(u8, entry.key, "anyOf")) result.variants = try plainVariants(a, value);
        }
    }
    return result;
}
fn plainVariants(a: std.mem.Allocator, value: Value) ![]const TSchema {
    if (value != .array) return &.{};
    const children = try a.alloc(TSchema, value.array.items.len);
    for (value.array.items, children) |item, *child| child.* = .{ .value = item };
    return children;
}
fn cloneSchema(a: std.mem.Allocator, schema: TSchema) !TSchema {
    return @import("../types.zig").cloneInto(TSchema, a, schema);
}
/// Bind a runtime schema to an explicit native value type. Wire serialization
/// remains the schema tree; Static(@TypeOf(binding)) returns ValueType.
pub fn TypedSchema(comptime ValueType: type) type {
    return struct {
        schema: TSchema,
        pub const Static = ValueType;
        pub fn jsonToValue(self: @This(), _: std.mem.Allocator, _: std.json.Stringify.Options) !Value {
            return self.schema.value;
        }
        pub fn jsonStringify(self: @This(), writer: *std.json.Stringify) !void {
            try j.writeCompatible(writer, self.schema.value);
        }
    };
}
pub fn Static(comptime schema: anytype) type {
    if (@TypeOf(schema) == type) {
        if (@hasDecl(schema, "Static")) return schema.Static;
        @compileError("Static expects a TypedSchema type or a comptime TSchema value");
    }
    if (@TypeOf(schema) != TSchema) @compileError("Static expects a TypedSchema type or a comptime TSchema value");
    return switch (schema.kind) {
        .string => []const u8,
        .number, .integer => f64,
        .boolean => bool,
        .null => @TypeOf(null),
        .array => []const Static((schema.items orelse @compileError("array schema requires native item metadata")).*),
        .tuple => blk: {
            var fields: [schema.variants.len]type = undefined;
            for (schema.variants, 0..) |item, index| fields[index] = Static(item);
            break :blk @Tuple(&fields);
        },
        .object => blk: {
            var names: [schema.properties.len][:0]const u8 = undefined;
            var fields: [schema.properties.len]type = undefined;
            var attrs: [schema.properties.len]std.builtin.Type.Struct.FieldAttributes = @splat(.{});
            for (schema.properties, 0..) |property, index| {
                names[index] = std.fmt.comptimePrint("{s}", .{property.name});
                const Child = Static(property.schema);
                var required = false;
                if (j.get(schema.value, "required")) |list| if (list == .array) {
                    for (list.array.items) |name| {
                        if (name == .string and std.mem.eql(u8, name.string, property.name)) required = true;
                    }
                };
                fields[index] = if (required or Child == @TypeOf(null)) Child else ?Child;
                if (!required) {
                    const default: fields[index] = null;
                    attrs[index].default_value_ptr = &default;
                }
            }
            break :blk @Struct(.auto, null, &names, &fields, &attrs);
        },
        .union_ => blk: {
            if (schema.variants.len == 0) break :blk noreturn;
            var names: [schema.variants.len][:0]const u8 = undefined;
            var fields: [schema.variants.len]type = undefined;
            var tags: [schema.variants.len]comptime_int = undefined;
            for (schema.variants, 0..) |item, index| {
                names[index] = std.fmt.comptimePrint("variant_{d}", .{index});
                fields[index] = Static(item);
                tags[index] = index;
            }
            const Tag = @Enum(u32, .exhaustive, &names, &tags);
            break :blk @Union(.auto, Tag, &names, &fields, &@splat(.{}));
        },
        .literal => switch (j.get(schema.value, "const") orelse .null) {
            .string => []const u8,
            .bool => bool,
            .integer, .float, .number_string => f64,
            else => Value,
        },
        .unsafe, .record => Value,
    };
}
fn primitive(a: std.mem.Allocator, kind: Kind, wire_type: []const u8, options: Options) !TSchema {
    return finish(a, .{ .value = try j.from(a, .{ .type = wire_type }), .kind = kind }, options);
}
pub const Type = struct {
    /// The binding borrows this schema's arena; it adds no wire fields.
    pub fn Typed(comptime ValueType: type, schema: TSchema) TypedSchema(ValueType) {
        return .{ .schema = schema };
    }
    pub fn String(a: std.mem.Allocator, options: Options) !TSchema {
        return primitive(a, .string, "string", options);
    }
    pub fn Number(a: std.mem.Allocator, options: Options) !TSchema {
        return primitive(a, .number, "number", options);
    }
    pub fn Integer(a: std.mem.Allocator, options: Options) !TSchema {
        return primitive(a, .integer, "integer", options);
    }
    pub fn Boolean(a: std.mem.Allocator, options: Options) !TSchema {
        return primitive(a, .boolean, "boolean", options);
    }
    pub fn Null(a: std.mem.Allocator) !TSchema {
        return primitive(a, .null, "null", .{});
    }
    pub fn Array(a: std.mem.Allocator, items: TSchema, options: Options) !TSchema {
        var result = try primitive(a, .array, "array", .{});
        const child = try a.create(TSchema);
        child.* = try cloneSchema(a, items);
        result.items = child;
        try j.put(a, &result.value, "items", child.value);
        return finish(a, result, options);
    }
    /// A struct of named TSchema fields or a Property slice retains each
    /// child's native kind. A std.json.Value property map remains plain schema.
    pub fn Object(a: std.mem.Allocator, properties: anytype, required: []const []const u8, options: Options) !TSchema {
        var result = try primitive(a, .object, "object", .{});
        var children: std.ArrayList(Property) = .empty;
        var wire = j.map(a);
        const P = @TypeOf(properties);
        if (P == Value) {
            if (properties != .object) return error.InvalidSchema;
            wire = try j.clone(a, properties);
        } else switch (@typeInfo(P)) {
            .@"struct" => |info| {
                inline for (info.field_names, info.field_types) |name, Field| {
                    if (Field != TSchema) @compileError("schema properties must contain TSchema values");
                    const child = try cloneSchema(a, @field(properties, name));
                    try j.put(a, &wire, name, child.value);
                    try children.append(a, .{ .name = name, .schema = child });
                }
            },
            .pointer, .array => {
                for (properties) |property| {
                    const child = try cloneSchema(a, property.schema);
                    const name = try a.dupe(u8, property.name);
                    try j.put(a, &wire, name, child.value);
                    try children.append(a, .{ .name = name, .schema = child });
                }
            },
            else => @compileError("use a schema property record, Property slice, or JSON object"),
        }
        result.properties = try children.toOwnedSlice(a);
        if (required.len > 0) try j.put(a, &result.value, "required", try j.clone(a, try j.from(a, required)));
        try j.put(a, &result.value, "properties", wire);
        return finish(a, result, options);
    }
    pub fn Union(a: std.mem.Allocator, variants: []const TSchema) !TSchema {
        const owned = try @import("../types.zig").cloneInto([]const TSchema, a, variants);
        return .{ .value = try j.from(a, .{ .anyOf = owned }), .kind = .union_, .variants = owned };
    }
    pub fn Tuple(a: std.mem.Allocator, items: []const TSchema, options: Options) !TSchema {
        var result = try primitive(a, .tuple, "array", .{});
        result.variants = try @import("../types.zig").cloneInto([]const TSchema, a, items);
        try j.put(a, &result.value, "additionalItems", .{ .bool = false });
        try j.put(a, &result.value, "items", try j.from(a, result.variants));
        try j.put(a, &result.value, "minItems", j.num(@floatFromInt(items.len)));
        return finish(a, result, options);
    }
    pub fn Literal(a: std.mem.Allocator, value: Value) !TSchema {
        const kind: []const u8 = switch (value) {
            .string => "string",
            .bool => "boolean",
            .integer, .float => "number",
            else => return error.InvalidLiteralValue,
        };
        return .{ .value = try j.from(a, .{ .type = kind, .@"const" = try j.clone(a, value) }), .kind = .literal };
    }
    pub fn UnionWithOptions(a: std.mem.Allocator, variants: []const TSchema, options: Options) !TSchema {
        return finish(a, try Union(a, variants), options);
    }
    pub fn LiteralWithOptions(a: std.mem.Allocator, value: Value, options: Options) !TSchema {
        return finish(a, try Literal(a, value), options);
    }
    pub fn NullWithOptions(a: std.mem.Allocator, options: Options) !TSchema {
        return primitive(a, .null, "null", options);
    }
    pub fn Unsafe(a: std.mem.Allocator, value: Value) !TSchema {
        return .{ .value = try j.clone(a, value) };
    }
};
pub fn StringEnum(a: std.mem.Allocator, values: []const []const u8, options: struct { description: ?[]const u8 = null, default: ?[]const u8 = null }) !TSchema {
    var result = try j.clone(a, try j.from(a, .{ .type = "string", .@"enum" = values }));
    if (options.description) |description| if (description.len > 0) try j.put(a, &result, "description", j.str(try a.dupe(u8, description)));
    if (options.default) |value| if (value.len > 0) try j.put(a, &result, "default", j.str(try a.dupe(u8, value)));
    return .{ .value = result };
}
