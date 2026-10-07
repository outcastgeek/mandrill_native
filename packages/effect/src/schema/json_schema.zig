//! Bounded JSON Schema export for declared encoded representations. This is
//! metadata translation, not a decoder or a reflection-based codec fallback.
//! Exact definitions reject policies the dialect cannot express; conservative
//! definitions may omit only restrictions that would otherwise overconstrain.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const ast = @import("schema_ast.zig");
const Issue = @import("schema_issue.zig").Issue;

/// Caller-selected maximum schema depth and emitted JSON Schema byte count.
pub const Limits = struct {
    max_depth: usize,
    max_output_bytes: usize,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Limits`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` when both maximum depth and output-byte bound are positive.
    pub fn valid(self: Limits) bool {
        return self.max_depth > 0 and self.max_output_bytes > 0;
    }
};

/// `Dialect` cases: `draft_2020_12`.
pub const Dialect = enum { draft_2020_12 };
const draft_2020_12_uri = "https://json-schema.org/draft/2020-12/schema";
const supported_pattern_dialect = "ecma262";

const EmitError = error{ OutOfMemory, TooLarge, Unsupported, Invalid, Depth };
const Shape = enum { integer, number, string, array, object, boolean, null_value, unknown };

const Sink = @import("internal/json_sink.zig").Sink(Emitter, EmitError, Emitter.write);

const Emitter = struct {
    output: std.array_list.Managed(u8),
    limits: Limits,
    definitions: []const ast.Definition,

    fn init(allocator: std.mem.Allocator, limits: Limits, definitions: []const ast.Definition) Emitter {
        return .{
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

    fn pointerValue(self: *Emitter, name: []const u8) EmitError!void {
        if (!std.unicode.utf8ValidateSlice(name)) return error.Invalid;
        var sink: Sink = .{ .target = self };
        // JSON Pointer escaping is schema-reference semantics. JSON string
        // escaping is delegated to std, including quotes and control bytes.
        try self.write("\"#/$defs/");
        for (name) |character| switch (character) {
            '~' => try self.write("~0"),
            '/' => try self.write("~1"),
            else => std.json.Stringify.encodeJsonStringChars(&.{character}, .{}, &sink.writer) catch return sink.failure.?,
        };
        try self.write("\"");
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
            .array, .prefix_array => .array,
            .exact_number => |number| if (number.integer) .integer else .number,
            .alternatives => .unknown,
            .enumeration => .string,
            .object => .object,
            .record, .tagged_union => .object,
            .nullable => .unknown,
            .constrained => |value| self.shape(value.base, depth + 1),
            .reference => |name| if (self.find(name)) |definition| self.shape(definition.root, depth + 1) else .unknown,
        };
    }

    fn child(self: *Emitter, json: *std.json.Stringify, sink: *Sink, source: *const ast.Node, accuracy: ast.Accuracy, depth: usize) EmitError!void {
        try sink.check(json.beginWriteRaw());
        try self.node(source, accuracy, depth);
        json.endWriteRaw();
    }

    fn node(self: *Emitter, source: *const ast.Node, accuracy: ast.Accuracy, depth: usize) EmitError!void {
        const next_depth = try self.level(depth);
        if (ast.validateLocal(source) == .err) return error.Invalid;
        var sink: Sink = .{ .target = self };
        // One std grammar state per AST node: recursion is bounded by level(),
        // while std sees only the local schema clause and its shallow wrappers.
        var json: std.json.Stringify = .{ .writer = &sink.writer };
        try sink.check(json.beginObject());
        switch (source.*) {
            .primitive => |value| {
                try sink.check(json.objectField("type"));
                try sink.check(json.write(switch (value) {
                    .boolean => "boolean",
                    .signed_integer, .unsigned_integer => "integer",
                    .finite_number => if (accuracy == .exact) return error.Unsupported else "number",
                    .string => "string",
                    .null_value => "null",
                    .decimal, .bytes, .identifier, .timestamp => return error.Unsupported,
                }));
                if (value == .signed_integer or value == .unsigned_integer) {
                    try sink.check(json.objectField("minimum"));
                    try sink.check(json.write(if (value == .signed_integer) @as(i128, std.math.minInt(i64)) else 0));
                    try sink.check(json.objectField("maximum"));
                    try sink.check(json.write(if (value == .signed_integer) @as(i128, std.math.maxInt(i64)) else std.math.maxInt(u64)));
                }
            },
            .enumeration => |value| {
                try sink.check(json.objectField("type"));
                try sink.check(json.write("string"));
                if (!value.preserve_unknown) {
                    try sink.check(json.objectField("enum"));
                    try sink.check(json.write(value.values));
                }
            },
            .exact_number => |number| {
                try sink.check(json.objectField("type"));
                try sink.check(json.write(if (number.integer) "integer" else "number"));
                inline for (.{ "minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum", "multipleOf" }, .{ number.minimum, number.maximum, number.exclusive_minimum, number.exclusive_maximum, number.multiple_of }) |name, operand| if (operand) |text| {
                    try sink.check(json.objectField(name));
                    try sink.check(json.beginWriteRaw());
                    // validateLocal checked the exact token; preserve its digits.
                    try self.write(text);
                    json.endWriteRaw();
                };
            },
            .alternatives => |value| {
                try sink.check(json.objectField(switch (value.mode) {
                    .one => "oneOf",
                    .any => "anyOf",
                    .all => "allOf",
                }));
                try sink.check(json.beginArray());
                for (value.branches) |branch| try self.child(&json, &sink, branch, accuracy, next_depth);
                try sink.check(json.endArray());
            },
            .prefix_array => |value| {
                try sink.check(json.objectField("type"));
                try sink.check(json.write("array"));
                try sink.check(json.objectField("prefixItems"));
                try sink.check(json.beginArray());
                var required: usize = 0;
                for (value.elements) |element| {
                    if (element.required) required += 1;
                    try self.child(&json, &sink, element.schema, accuracy, next_depth);
                }
                try sink.check(json.endArray());
                try sink.check(json.objectField("items"));
                if (value.rest) |rest| try self.child(&json, &sink, rest, accuracy, next_depth) else try sink.check(json.write(false));
                try sink.check(json.objectField("minItems"));
                try sink.check(json.write(@max(value.minimum, required)));
                try sink.check(json.objectField("maxItems"));
                try sink.check(json.write(if (value.rest == null) @min(value.maximum, value.elements.len) else value.maximum));
                if (value.unique) {
                    try sink.check(json.objectField("uniqueItems"));
                    try sink.check(json.write(true));
                }
            },
            .object => |value| {
                try sink.check(json.objectField("allOf"));
                try sink.check(json.beginArray());
                try self.child(&json, &sink, value.base, accuracy, next_depth);
                try sink.check(json.beginObject());
                try sink.check(json.objectField("type"));
                try sink.check(json.write("object"));
                try sink.check(json.objectField("minProperties"));
                try sink.check(json.write(value.minimum));
                try sink.check(json.objectField("maxProperties"));
                try sink.check(json.write(value.maximum));
                if (value.property_names) |names| {
                    try sink.check(json.objectField("propertyNames"));
                    try self.child(&json, &sink, names, accuracy, next_depth);
                }
                try sink.check(json.objectField("properties"));
                try sink.check(json.beginObject());
                if (value.base.* != .record and accuracy == .exact) return error.Unsupported;
                if (value.base.* == .record) for (value.base.record.fields) |field| {
                    try sink.check(json.objectField(field.name));
                    try sink.check(json.beginObject());
                    try sink.check(json.endObject());
                };
                try sink.check(json.endObject());
                try sink.check(json.objectField("patternProperties"));
                try sink.check(json.beginObject());
                for (value.patterns) |pattern| {
                    if (!std.mem.eql(u8, pattern.dialect, supported_pattern_dialect)) {
                        if (accuracy == .exact) return error.Unsupported;
                        continue;
                    }
                    try sink.check(json.objectField(pattern.source));
                    try self.child(&json, &sink, pattern.schema, accuracy, next_depth);
                }
                try sink.check(json.endObject());
                try sink.check(json.objectField("additionalProperties"));
                if (value.additional) |additional| try self.child(&json, &sink, additional, accuracy, next_depth) else {
                    var unknown_pattern = value.base.* != .record;
                    for (value.patterns) |pattern| if (!std.mem.eql(u8, pattern.dialect, supported_pattern_dialect)) {
                        unknown_pattern = true;
                    };
                    try sink.check(json.write(unknown_pattern and accuracy == .conservative));
                }
                try sink.check(json.endObject());
                try sink.check(json.endArray());
            },
            .nullable => |inner| {
                try sink.check(json.objectField("anyOf"));
                try sink.check(json.beginArray());
                try self.child(&json, &sink, inner, accuracy, next_depth);
                try sink.check(json.write(.{ .type = "null" }));
                try sink.check(json.endArray());
            },
            .array => |inner| {
                try sink.check(json.objectField("type"));
                try sink.check(json.write("array"));
                try sink.check(json.objectField("items"));
                try self.child(&json, &sink, inner, accuracy, next_depth);
            },
            .record => |record| {
                if (record.duplicate_fields == .reject and accuracy == .exact) return error.Unsupported;
                try sink.check(json.objectField("type"));
                try sink.check(json.write("object"));
                try sink.check(json.objectField("properties"));
                try sink.check(json.beginObject());
                for (record.fields) |field| {
                    if (field.presence == .defaulted and accuracy == .exact) return error.Unsupported;
                    try sink.check(json.objectField(field.name));
                    try self.child(&json, &sink, field.schema, accuracy, next_depth);
                }
                try sink.check(json.endObject());
                try sink.check(json.objectField("required"));
                try sink.check(json.beginArray());
                for (record.fields) |field| if (field.presence == .required) {
                    try sink.check(json.write(field.name));
                };
                try sink.check(json.endArray());
                try sink.check(json.objectField("additionalProperties"));
                try sink.check(json.write(record.unknown_fields != .reject));
            },
            .tagged_union => |tagged| {
                try sink.check(json.objectField("oneOf"));
                try sink.check(json.beginArray());
                for (tagged.cases) |case| {
                    try sink.check(json.beginObject());
                    try sink.check(json.objectField("type"));
                    try sink.check(json.write("object"));
                    try sink.check(json.objectField("properties"));
                    try sink.check(json.beginObject());
                    try sink.check(json.objectField(tagged.discriminator));
                    try sink.check(json.write(.{ .@"const" = case.tag }));
                    if (case.payload) |payload| {
                        try sink.check(json.objectField(tagged.payload_name));
                        try self.child(&json, &sink, payload, accuracy, next_depth);
                    }
                    try sink.check(json.endObject());
                    try sink.check(json.objectField("required"));
                    try sink.check(json.beginArray());
                    try sink.check(json.write(tagged.discriminator));
                    if (case.payload != null) try sink.check(json.write(tagged.payload_name));
                    try sink.check(json.endArray());
                    try sink.check(json.objectField("additionalProperties"));
                    try sink.check(json.write(false));
                    try sink.check(json.endObject());
                }
                try sink.check(json.endArray());
            },
            .reference => |name| {
                if (self.find(name) == null) return error.Invalid;
                try sink.check(json.objectField("$ref"));
                try sink.check(json.beginWriteRaw());
                try self.pointerValue(name);
                json.endWriteRaw();
            },
            .constrained => |constrained| {
                try sink.check(json.objectField("allOf"));
                try sink.check(json.beginArray());
                try self.child(&json, &sink, constrained.base, accuracy, next_depth);
                const base_shape = self.shape(constrained.base, next_depth);
                for (constrained.rules) |rule| switch (rule) {
                    .integer_bounds => |bounds| {
                        if (base_shape != .integer) {
                            if (accuracy == .exact) return error.Unsupported;
                            continue;
                        }
                        try sink.check(json.beginObject());
                        try sink.check(json.objectField("type"));
                        try sink.check(json.write("integer"));
                        if (bounds.minimum) |minimum| {
                            try sink.check(json.objectField("minimum"));
                            try sink.check(json.write(minimum));
                        }
                        if (bounds.maximum) |maximum| {
                            try sink.check(json.objectField("maximum"));
                            try sink.check(json.write(maximum));
                        }
                        try sink.check(json.endObject());
                    },
                    .length => |length| {
                        const is_string = length.unit == .unicode_scalars and base_shape == .string;
                        const is_array = length.unit == .elements and base_shape == .array;
                        if (!is_string and !is_array) {
                            if (accuracy == .exact) return error.Unsupported;
                            continue;
                        }
                        try sink.check(json.beginObject());
                        try sink.check(json.objectField("type"));
                        try sink.check(json.write(if (is_string) "string" else "array"));
                        if (length.minimum) |minimum| {
                            try sink.check(json.objectField(if (is_string) "minLength" else "minItems"));
                            try sink.check(json.write(minimum));
                        }
                        if (length.maximum) |maximum| {
                            try sink.check(json.objectField(if (is_string) "maxLength" else "maxItems"));
                            try sink.check(json.write(maximum));
                        }
                        try sink.check(json.endObject());
                    },
                    .check => {
                        if (accuracy == .exact) return error.Unsupported;
                    },
                    .format => |format| {
                        if (base_shape != .string) {
                            if (accuracy == .exact) return error.Unsupported;
                            continue;
                        }
                        if (accuracy == .exact) return error.Unsupported;
                        try sink.check(json.write(.{ .format = if (format == .uuid_v4) "uuid" else @tagName(format) }));
                    },
                    .pattern => |pattern| {
                        if (base_shape != .string or !std.mem.eql(u8, pattern.dialect, supported_pattern_dialect)) {
                            if (accuracy == .exact) return error.Unsupported;
                            continue;
                        }
                        try sink.check(json.write(.{ .type = "string", .pattern = pattern.text }));
                    },
                };
                try sink.check(json.endArray());
            },
        }
        try sink.check(json.endObject());
    }

    fn document(self: *Emitter, root_name: []const u8) EmitError!void {
        var sink: Sink = .{ .target = self };
        var json: std.json.Stringify = .{ .writer = &sink.writer };
        try sink.check(json.beginObject());
        try sink.check(json.objectField("$schema"));
        try sink.check(json.write(draft_2020_12_uri));
        try sink.check(json.objectField("$ref"));
        try sink.check(json.beginWriteRaw());
        try self.pointerValue(root_name);
        json.endWriteRaw();
        try sink.check(json.objectField("$defs"));
        try sink.check(json.beginObject());
        for (self.definitions) |definition| {
            try sink.check(json.objectField(definition.name));
            try self.child(&json, &sink, definition.root, definition.accuracy, 0);
        }
        try sink.check(json.endObject());
        try sink.check(json.endObject());
    }
};

/// Translate immutable encoded-shape definitions into draft-2020-12 JSON Schema.
/// R supplies the output allocator; success owns its bytes and failures own Issues.
/// Definitions/names are borrowed for this call. References remain references,
/// so recursive names do not recursively emit or acquire runtime services.
/// Caller max_depth bounds AST expansion and max_output_bytes bounds emitted UTF-8.
/// std.json handles JSON syntax; exact decimal operands are validated raw tokens.
/// Exact mode rejects unsupported semantics rather than silently weakening them;
/// conservative mode may over-approximate format/check/pattern behavior.
/// Allocation failure is .die; invalid/unsupported/bounded representations are .fail.
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
    const generated: ?EmitError = blk: {
        emitter.document(root_name) catch |problem| break :blk problem;
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
