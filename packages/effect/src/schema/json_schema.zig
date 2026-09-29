//! Bounded JSON Schema export for declared encoded representations. This is
//! metadata translation, not a decoder or a reflection-based codec fallback.
//! Exact definitions reject policies the dialect cannot express; conservative
//! definitions may omit only restrictions that would otherwise overconstrain.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const ast = @import("schema_ast.zig");
const Issue = @import("schema_issue.zig").Issue;

pub const Limits = struct {
    max_depth: usize,
    max_output_bytes: usize,

    pub fn valid(self: Limits) bool {
        return self.max_depth > 0 and self.max_output_bytes > 0;
    }
};

pub const Dialect = enum { draft_2020_12 };
const draft_2020_12_uri = "https://json-schema.org/draft/2020-12/schema";
const supported_pattern_dialect = "ecma262";

const EmitError = error{ OutOfMemory, TooLarge, Unsupported, Invalid, Depth };
const Shape = enum { integer, number, string, array, object, boolean, null_value, unknown };

const Emitter = struct {
    allocator: std.mem.Allocator,
    output: std.array_list.Managed(u8),
    limits: Limits,
    definitions: []const ast.Definition,

    fn init(allocator: std.mem.Allocator, limits: Limits, definitions: []const ast.Definition) Emitter {
        return .{
            .allocator = allocator,
            .output = std.array_list.Managed(u8).init(allocator),
            .limits = limits,
            .definitions = definitions,
        };
    }

    fn write(self: *Emitter, value: []const u8) EmitError!void {
        const next = std.math.add(usize, self.output.items.len, value.len) catch return error.TooLarge;
        if (next > self.limits.max_output_bytes) return error.TooLarge;
        try self.output.appendSlice(value);
    }

    fn byte(self: *Emitter, value: u8) EmitError!void {
        if (self.output.items.len >= self.limits.max_output_bytes) return error.TooLarge;
        try self.output.append(value);
    }

    fn hexNibble(value: u8) u8 {
        return if (value < 10) '0' + value else 'a' + (value - 10);
    }

    fn quoted(self: *Emitter, value: []const u8) EmitError!void {
        if (!std.unicode.utf8ValidateSlice(value)) return error.Invalid;
        try self.byte('"');
        for (value) |character| {
            switch (character) {
                '"' => try self.write("\\\""),
                '\\' => try self.write("\\\\"),
                0...0x1f => {
                    try self.write("\\u00");
                    try self.byte(hexNibble(character >> 4));
                    try self.byte(hexNibble(character & 0x0f));
                },
                else => try self.byte(character),
            }
        }
        try self.byte('"');
    }

    fn unsigned(self: *Emitter, value: usize) EmitError!void {
        var scratch: [std.fmt.count("{d}", .{std.math.maxInt(usize)})]u8 = undefined;
        const printed = std.fmt.bufPrint(&scratch, "{d}", .{value}) catch unreachable;
        try self.write(printed);
    }

    fn signed(self: *Emitter, value: i128) EmitError!void {
        var scratch: [std.fmt.count("{d}", .{std.math.minInt(i128)})]u8 = undefined;
        const printed = std.fmt.bufPrint(&scratch, "{d}", .{value}) catch unreachable;
        try self.write(printed);
    }

    fn pointerName(self: *Emitter, name: []const u8) EmitError!void {
        for (name) |character| switch (character) {
            '~' => try self.write("~0"),
            '/' => try self.write("~1"),
            else => try self.byte(character),
        };
    }

    fn reference(self: *Emitter, name: []const u8) EmitError!void {
        if (self.find(name) == null) return error.Invalid;
        try self.write("{\"$ref\":\"#/$defs/");
        try self.pointerName(name);
        try self.write("\"}");
    }

    fn find(self: *const Emitter, name: []const u8) ?ast.Definition {
        for (self.definitions) |definition| {
            if (std.mem.eql(u8, definition.name, name)) return definition;
        }
        return null;
    }

    fn level(self: *const Emitter, depth: usize) EmitError!usize {
        if (depth >= self.limits.max_depth) return error.Depth;
        return depth + 1;
    }

    fn primitive(self: *Emitter, value: ast.Primitive, accuracy: ast.Accuracy) EmitError!void {
        switch (value) {
            .boolean => try self.write("{\"type\":\"boolean\"}"),
            .signed_integer => {
                try self.write("{\"type\":\"integer\",\"minimum\":");
                try self.signed(std.math.minInt(i64));
                try self.write(",\"maximum\":");
                try self.signed(std.math.maxInt(i64));
                try self.byte('}');
            },
            .unsigned_integer => {
                try self.write("{\"type\":\"integer\",\"minimum\":0,\"maximum\":");
                try self.signed(std.math.maxInt(u64));
                try self.byte('}');
            },
            .finite_number => {
                // JSON Schema cannot express exactly which decimal JSON numbers
                // round to finite f64 values under this codec.
                if (accuracy == .exact) return error.Unsupported;
                try self.write("{\"type\":\"number\"}");
            },
            .string => try self.write("{\"type\":\"string\"}"),
            .null_value => try self.write("{\"type\":\"null\"}"),
            // The byte/decimal/identity/time wire formats require matching
            // codecs before an honest representation can be exported.
            .decimal, .bytes, .identifier, .timestamp => return error.Unsupported,
        }
    }

    fn shape(self: *const Emitter, source: *const ast.Node, depth: usize) Shape {
        if (depth >= self.limits.max_depth) return .unknown;
        return switch (source.*) {
            .primitive => |value| switch (value) {
                .boolean => .boolean,
                .signed_integer, .unsigned_integer => .integer,
                .finite_number => .number,
                .string, .decimal, .bytes, .identifier, .timestamp => .string,
                .null_value => .null_value,
            },
            .array => .array,
            .record, .tagged_union => .object,
            .nullable => .unknown,
            .constrained => |value| self.shape(value.base, depth + 1),
            .reference => |name| if (self.find(name)) |definition| self.shape(definition.root, depth + 1) else .unknown,
        };
    }

    fn node(self: *Emitter, source: *const ast.Node, accuracy: ast.Accuracy, depth: usize) EmitError!void {
        const next_depth = try self.level(depth);
        if (ast.validateLocal(source) == .err) return error.Invalid;
        switch (source.*) {
            .primitive => |value| try self.primitive(value, accuracy),
            .nullable => |inner| {
                try self.write("{\"anyOf\":[");
                try self.node(inner, accuracy, next_depth);
                try self.write(",{\"type\":\"null\"}]}");
            },
            .array => |inner| {
                try self.write("{\"type\":\"array\",\"items\":");
                try self.node(inner, accuracy, next_depth);
                try self.byte('}');
            },
            .record => |record| {
                // A rejecting raw-input duplicate policy cannot be enforced
                // by JSON Schema over already-parsed object members.
                if (record.duplicate_fields == .reject and accuracy == .exact) return error.Unsupported;
                try self.write("{\"type\":\"object\",\"properties\":{");
                for (record.fields, 0..) |field, index| {
                    if (field.presence == .defaulted and accuracy == .exact) return error.Unsupported;
                    if (index != 0) try self.byte(',');
                    try self.quoted(field.name);
                    try self.byte(':');
                    try self.node(field.schema, accuracy, next_depth);
                }
                try self.write("},\"required\":[");
                var first = true;
                for (record.fields) |field| {
                    if (field.presence != .required) continue;
                    if (!first) try self.byte(',');
                    first = false;
                    try self.quoted(field.name);
                }
                try self.write("],\"additionalProperties\":");
                try self.write(if (record.unknown_fields == .reject) "false}" else "true}");
            },
            .tagged_union => |tagged| {
                if (tagged.cases.len == 0) return error.Invalid;
                try self.write("{\"oneOf\":[");
                for (tagged.cases, 0..) |case, index| {
                    if (index != 0) try self.byte(',');
                    try self.write("{\"type\":\"object\",\"properties\":{");
                    try self.quoted(tagged.discriminator);
                    try self.write(":{\"const\":");
                    try self.quoted(case.tag);
                    try self.byte('}');
                    if (case.payload) |payload| {
                        try self.byte(',');
                        try self.quoted(tagged.payload_name);
                        try self.byte(':');
                        try self.node(payload, accuracy, next_depth);
                    }
                    try self.write("},\"required\":[");
                    try self.quoted(tagged.discriminator);
                    if (case.payload != null) {
                        try self.byte(',');
                        try self.quoted(tagged.payload_name);
                    }
                    try self.write("],\"additionalProperties\":false}");
                }
                try self.write("]}");
            },
            .reference => |name| try self.reference(name),
            .constrained => |constrained| {
                // allOf lets constraints remain separate from a recursive or
                // compound base; unsupported constraints are only omitted in
                // the caller's explicit conservative mode.
                try self.write("{\"allOf\":[");
                try self.node(constrained.base, accuracy, next_depth);
                const base_shape = self.shape(constrained.base, next_depth);
                for (constrained.rules) |rule| {
                    switch (rule) {
                        .integer_bounds => |bounds| {
                            if (base_shape != .integer) {
                                if (accuracy == .exact) return error.Unsupported;
                                continue;
                            }
                            try self.write(",{\"type\":\"integer\"");
                            if (bounds.minimum) |minimum| {
                                try self.write(",\"minimum\":");
                                try self.signed(minimum);
                            }
                            if (bounds.maximum) |maximum| {
                                try self.write(",\"maximum\":");
                                try self.signed(maximum);
                            }
                            try self.byte('}');
                        },
                        .length => |length| {
                            const is_string = length.unit == .unicode_scalars and base_shape == .string;
                            const is_array = length.unit == .elements and base_shape == .array;
                            if (!is_string and !is_array) {
                                if (accuracy == .exact) return error.Unsupported;
                                continue;
                            }
                            try self.write(if (is_string) ",{\"type\":\"string\"" else ",{\"type\":\"array\"");
                            if (length.minimum) |minimum| {
                                try self.write(if (is_string) ",\"minLength\":" else ",\"minItems\":");
                                try self.unsigned(minimum);
                            }
                            if (length.maximum) |maximum| {
                                try self.write(if (is_string) ",\"maxLength\":" else ",\"maxItems\":");
                                try self.unsigned(maximum);
                            }
                            try self.byte('}');
                        },
                        .pattern => |pattern| {
                            if (base_shape != .string or !std.mem.eql(u8, pattern.dialect, supported_pattern_dialect)) {
                                if (accuracy == .exact) return error.Unsupported;
                                continue;
                            }
                            try self.write(",{\"type\":\"string\",\"pattern\":");
                            try self.quoted(pattern.text);
                            try self.byte('}');
                        },
                    }
                }
                try self.write("]}");
            },
        }
    }
};

/// The caller supplies the entire named definition table and resource limits.
/// References remain references, so recursive names do not recursively emit.
pub fn exportSchema(
    env: anytype,
    definitions: []const ast.Definition,
    root_name: []const u8,
    limits: Limits,
    comptime dialect: Dialect,
) outcomes.Exit(outcomes.OwnedSlice(u8), Issue) {
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
        @compileError("JSON Schema export requires the caller allocator in R");
    if (!limits.valid() or definitions.len == 0) return .{ .fail = Issue.init(env, .representation, .invalid_value) };
    if (dialect != .draft_2020_12) @compileError("unsupported JSON Schema dialect");
    for (definitions, 0..) |definition, index| {
        if (ast.validateDefinition(definition) == .err) return .{ .fail = Issue.init(env, .representation, .invalid_value) };
        for (definitions[0..index]) |previous| {
            if (std.mem.eql(u8, previous.name, definition.name))
                return .{ .fail = Issue.init(env, .representation, .invalid_value) };
        }
    }
    var emitter = Emitter.init(env.allocator, limits, definitions);
    defer emitter.output.deinit();
    if (emitter.find(root_name) == null) return .{ .fail = Issue.init(env, .representation, .invalid_value) };
    const generated = blk: {
        emitter.write("{\"$schema\":") catch |problem| break :blk problem;
        emitter.quoted(draft_2020_12_uri) catch |problem| break :blk problem;
        emitter.write(",\"$ref\":\"#/$defs/") catch |problem| break :blk problem;
        emitter.pointerName(root_name) catch |problem| break :blk problem;
        emitter.write("\",\"$defs\":{") catch |problem| break :blk problem;
        for (definitions, 0..) |definition, index| {
            if (index != 0) emitter.byte(',') catch |problem| break :blk problem;
            emitter.quoted(definition.name) catch |problem| break :blk problem;
            emitter.byte(':') catch |problem| break :blk problem;
            emitter.node(definition.root, definition.accuracy, 0) catch |problem| break :blk problem;
        }
        emitter.write("}}") catch |problem| break :blk problem;
        break :blk null;
    };
    if (generated) |problem| {
        return switch (problem) {
            error.OutOfMemory => .{ .die = outcomes.Defect.out_of_memory },
            error.TooLarge => .{ .fail = Issue.init(env, .representation, .size_limit) },
            error.Depth => .{ .fail = Issue.init(env, .representation, .depth_limit) },
            error.Unsupported => .{ .fail = Issue.init(env, .representation, .unsupported_representation) },
            error.Invalid => .{ .fail = Issue.init(env, .representation, .invalid_value) },
        };
    }
    const owned = env.allocator.dupe(u8, emitter.output.items) catch return .{ .die = outcomes.Defect.out_of_memory };
    return .{ .success = .{ .items = owned, .allocator = env.allocator } };
}
