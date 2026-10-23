const std = @import("std");
const j = @import("../json/value.zig");
const t = @import("../types.zig");
const Value = std.json.Value;
pub const UnsupportedStrictSchemaKeywordCheck = struct {
    context: ?*anyopaque = null,
    call: *const fn (?*anyopaque, []const u8, Value) anyerror!bool,
};
const unsupported = [_][]const u8{ "$ref", "$defs", "definitions", "allOf", "oneOf", "patternProperties", "dependentSchemas", "dependencies", "unevaluatedProperties", "propertyNames", "contains", "prefixItems", "not", "if", "then", "else" };
const StrictFailure = union(enum) {
    root_object,
    boolean_schema,
    keyword: []const u8,
    provider_keyword: struct { key: []const u8, value: Value },
    empty_any_of,
    structured_union,
    tuple,
    properties_without_object,
    additional_properties,
    properties_map,
    required_strings,
    unknown_required,
    fn text(self: @This(), a: std.mem.Allocator) ![]const u8 {
        return switch (self) {
            .root_object => "root schema must have type object",
            .boolean_schema => "boolean schemas are unsupported",
            .keyword => |key| try std.fmt.allocPrint(a, "{s} schemas are unsupported", .{key}),
            .provider_keyword => |entry| try std.fmt.allocPrint(a, "{s}: {s} is unsupported", .{ entry.key, try j.stringify(a, entry.value, .{}) }),
            .empty_any_of => "anyOf must contain at least one schema",
            .structured_union => "object and array unions are unsupported",
            .tuple => "tuple schemas are unsupported",
            .properties_without_object => "properties require type object",
            .additional_properties => "schema-valued or true additionalProperties is unsupported",
            .properties_map => "object properties must be a schema map",
            .required_strings => "object required must be a string array",
            .unknown_required => "required contains an unknown property",
        };
    }
};
fn unsupportedSchema(failure: *?StrictFailure, reason: StrictFailure) error{UnsupportedStrictJsonSchema} {
    failure.* = reason;
    return error.UnsupportedStrictJsonSchema;
}
fn structured(schema: Value) bool {
    if (schema != .object) return false;
    if (j.field(schema, "type")) |kind| if (std.mem.eql(u8, kind, "object") or std.mem.eql(u8, kind, "array")) return true;
    if (j.get(schema, "type")) |kinds| if (kinds == .array) {
        for (kinds.array.items) |kind| if (kind == .string and (std.mem.eql(u8, kind.string, "object") or std.mem.eql(u8, kind.string, "array"))) return true;
    };
    return j.get(schema, "properties") != null or j.get(schema, "items") != null;
}
fn allowsNull(schema: Value) bool {
    if (schema != .object) return false;
    if (j.field(schema, "type")) |kind| if (std.mem.eql(u8, kind, "null")) return true;
    if (j.get(schema, "type")) |kinds| if (kinds == .array) {
        for (kinds.array.items) |kind| if (kind == .string and std.mem.eql(u8, kind.string, "null")) return true;
    };
    if (j.get(schema, "const")) |value| if (value == .null) return true;
    if (j.get(schema, "enum")) |values| if (values == .array) {
        for (values.array.items) |value| if (value == .null) return true;
    };
    if (j.get(schema, "anyOf")) |variants| if (variants == .array) {
        for (variants.array.items) |variant| if (allowsNull(variant)) return true;
    };
    return false;
}
fn strictNode(a: std.mem.Allocator, schema: *Value, check: ?UnsupportedStrictSchemaKeywordCheck, failure: *?StrictFailure) anyerror!void {
    if (schema.* != .object) return unsupportedSchema(failure, .boolean_schema);
    for (unsupported) |key| if (j.get(schema.*, key) != null) return unsupportedSchema(failure, .{ .keyword = key });
    if (check) |predicate| {
        const entries = try j.objectEntries(a, schema.object);
        defer a.free(entries);
        for (entries) |entry| if (try predicate.call(predicate.context, entry.key, entry.node.*)) return unsupportedSchema(failure, .{ .provider_keyword = .{ .key = entry.key, .value = entry.node.* } });
    }
    if (schema.object.getPtr("anyOf")) |variants| {
        if (variants.* != .array or variants.array.items.len == 0) return unsupportedSchema(failure, .empty_any_of);
        for (variants.array.items) |*variant| {
            if (structured(variant.*)) return unsupportedSchema(failure, .structured_union);
            try strictNode(a, variant, check, failure);
        }
    }
    if (schema.object.getPtr("items")) |items| {
        if (items.* == .array) return unsupportedSchema(failure, .tuple);
        try strictNode(a, items, check, failure);
    }
    const object = if (j.field(schema.*, "type")) |kind| std.mem.eql(u8, kind, "object") else false;
    if (j.get(schema.*, "properties") != null and !object) return unsupportedSchema(failure, .properties_without_object);
    if (!object) return;
    if (j.get(schema.*, "additionalProperties")) |additional| if (additional != .bool or additional.bool) return unsupportedSchema(failure, .additional_properties);
    var properties = j.get(schema.*, "properties") orelse j.map(a);
    if (properties != .object) return unsupportedSchema(failure, .properties_map);
    const required = j.get(schema.*, "required");
    if (required) |keys| {
        if (keys != .array) return unsupportedSchema(failure, .required_strings);
        for (keys.array.items) |key| if (key != .string) return unsupportedSchema(failure, .required_strings);
        for (keys.array.items) |key| if (!properties.object.contains(key.string)) return unsupportedSchema(failure, .unknown_required);
    }
    var all_required = j.array(a);
    const entries = try j.objectEntries(a, properties.object);
    defer a.free(entries);
    for (entries) |entry| {
        const property = properties.object.getPtr(entry.key).?;
        try strictNode(a, property, check, failure);
        const is_required = if (required) |keys| for (keys.array.items) |key| {
            if (std.mem.eql(u8, key.string, entry.key)) break true;
        } else false else false;
        if (!is_required and !allowsNull(property.*)) {
            var alternatives = j.array(a);
            try alternatives.array.append(property.*);
            try alternatives.array.append(try j.from(a, .{ .type = "null" }));
            var nullable = j.map(a);
            try j.put(a, &nullable, "anyOf", alternatives);
            property.* = nullable;
        }
        try all_required.array.append(j.str(entry.key));
    }
    try j.put(a, schema, "required", all_required);
    try j.put(a, schema, "additionalProperties", .{ .bool = false });
}
pub fn makeStrictJsonSchema(a: std.mem.Allocator, input: Value) !Value {
    return makeStrictJsonSchemaWithCheck(a, input, null);
}
pub fn makeStrictJsonSchemaWithCheck(a: std.mem.Allocator, input: Value, check: ?UnsupportedStrictSchemaKeywordCheck) !Value {
    return makeStrictJsonSchemaWithError(a, input, check, null);
}
fn makeStrict(a: std.mem.Allocator, input: Value, check: ?UnsupportedStrictSchemaKeywordCheck, failure: *?StrictFailure) !Value {
    var schema = try j.clone(a, input);
    if (schema != .object) return unsupportedSchema(failure, .root_object);
    try strictNode(a, &schema, check, failure);
    if (!std.mem.eql(u8, j.field(schema, "type") orelse "", "object")) return unsupportedSchema(failure, .root_object);
    return schema;
}
pub fn makeStrictJsonSchemaWithError(a: std.mem.Allocator, input: Value, check: ?UnsupportedStrictSchemaKeywordCheck, error_message: ?*?[]const u8) !Value {
    var failure: ?StrictFailure = null;
    return makeStrict(a, input, check, &failure) catch |err| {
        if (err == error.UnsupportedStrictJsonSchema) if (failure) |internal| if (error_message) |slot| {
            slot.* = try internal.text(a);
        };
        return err;
    };
}
pub fn getJsonSchemaToolParameters(a: std.mem.Allocator, tool: t.Tool, strict: ?bool) !Value {
    return if (strict orelse false) makeStrictJsonSchema(a, tool.parameters.value) else tool.parameters.value;
}
pub fn getJsonSchemaToolParametersWithError(a: std.mem.Allocator, tool: t.Tool, strict: ?bool, error_message: ?*?[]const u8) !Value {
    return if (strict orelse false) makeStrictJsonSchemaWithError(a, tool.parameters.value, null, error_message) else tool.parameters.value;
}
pub fn resolveJsonSchemaStrictSampling(a: std.mem.Allocator, tool: t.Tool, supports: bool) !?bool {
    return resolveJsonSchemaStrictSamplingWithCheck(a, tool, supports, null);
}
pub fn resolveJsonSchemaStrictSamplingWithCheck(a: std.mem.Allocator, tool: t.Tool, supports: bool, check: ?UnsupportedStrictSchemaKeywordCheck) !?bool {
    return resolveJsonSchemaStrictSamplingWithError(a, tool, supports, check, null);
}
pub fn resolveJsonSchemaStrictSamplingWithError(a: std.mem.Allocator, tool: t.Tool, supports: bool, check: ?UnsupportedStrictSchemaKeywordCheck, error_message: ?*?[]const u8) !?bool {
    const config = tool.constrainedSampling orelse return null;
    if (!std.mem.eql(u8, j.field(config, "type") orelse "", "json_schema")) return null;
    var failure: ?StrictFailure = null;
    if (supports) {
        _ = makeStrict(a, tool.parameters.value, check, &failure) catch |err| {
            // A public callback can return the same Zig error tag. Only our
            // rejection helper supplies provenance for prefer/require handling.
            if (err != error.UnsupportedStrictJsonSchema or failure == null) return err;
            if (!std.mem.eql(u8, j.field(config, "strict") orelse "", "require")) return null;
            if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Tool \"{s}\" requires JSON-schema constrained sampling, but {s}.", .{ tool.name, try failure.?.text(a) });
            return error.RequiredConstrainedSamplingUnsupported;
        };
        return true;
    }
    if (std.mem.eql(u8, j.field(config, "strict") orelse "", "require")) {
        if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Tool \"{s}\" requires JSON-schema constrained sampling, but strict tools are unsupported.", .{tool.name});
        return error.RequiredConstrainedSamplingUnsupported;
    }
    return null;
}
pub const GrammarConstrainedSampling = struct { format: enum { lark, regex }, definition: []const u8, inputProperty: []const u8 };
pub const GrammarToolInputJsonBuffer = struct {
    input: []const u8 = "",
    started: bool = false,
    closed: bool = false,
};
pub fn appendGrammarToolInputJsonDelta(a: std.mem.Allocator, buffer: *GrammarToolInputJsonBuffer, input_property: []const u8, next_input: []const u8, close: bool) !?[]const u8 {
    return appendGrammarToolInputJsonDeltaWithError(a, buffer, input_property, next_input, close, null);
}
pub fn appendGrammarToolInputJsonDeltaWithError(a: std.mem.Allocator, buffer: *GrammarToolInputJsonBuffer, input_property: []const u8, next_input: []const u8, close: bool, error_message: ?*?[]const u8) !?[]const u8 {
    var arena = std.heap.ArenaAllocator.init(a);
    defer arena.deinit();
    const scratch = arena.allocator();
    const previous_units = try std.unicode.wtf8ToWtf16LeAlloc(scratch, buffer.input);
    const next_units = try std.unicode.wtf8ToWtf16LeAlloc(scratch, next_input);
    if (buffer.closed) {
        if (close and std.mem.eql(u16, next_units, previous_units)) return null;
        if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "grammar tool input for property \"{s}\" changed after it was closed", .{input_property});
        return error.GrammarToolInputChangedAfterClose;
    }
    if (!std.mem.startsWith(u16, next_units, previous_units)) {
        if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "grammar tool input for property \"{s}\" changed non-monotonically", .{input_property});
        return error.GrammarToolInputChangedNonMonotonically;
    }
    if (!close and next_units.len == previous_units.len) return null;
    var output = std.Io.Writer.Allocating.init(a);
    defer output.deinit();
    // Recreate the standard serializer's object context for every call. For
    // an already-started buffer its prefix is discarded, so the public buffer
    // remains the same three-field state as Pi and retains no writer pointer.
    var discard_buffer: [64]u8 = undefined;
    var discarded = std.Io.Writer.Discarding.init(&discard_buffer);
    var serializer = std.json.Stringify{ .writer = if (buffer.started) &discarded.writer else &output.writer };
    serializer.beginObject() catch return error.OutOfMemory;
    j.writeField(&serializer, input_property) catch return error.OutOfMemory;
    serializer.beginWriteRaw() catch return error.OutOfMemory;
    serializer.writer = &output.writer;
    if (!buffer.started) {
        // Only these string delimiters use the approved compatibility path.
        output.writer.writeByte('"') catch return error.OutOfMemory;
    }
    const input_delta = try std.unicode.wtf16LeToWtf8Alloc(scratch, next_units[previous_units.len..]);
    j.writeStringChars(input_delta, .{}, &output.writer) catch return error.OutOfMemory;
    if (close) {
        output.writer.writeByte('"') catch return error.OutOfMemory;
        serializer.endWriteRaw();
        serializer.endObject() catch return error.OutOfMemory;
    }
    const input = try a.dupe(u8, next_input);
    errdefer a.free(input);
    const delta = try output.toOwnedSlice();
    buffer.input = input;
    buffer.started = true;
    buffer.closed = close;
    return delta;
}
const GrammarFailure = union(enum) {
    no_variant,
    missing_variants,
    null_variants,
    null_schema,
    object_schema,
    required_string,
    missing_property: []const u8,
    property_type: []const u8,
    fn text(self: @This(), a: std.mem.Allocator) ![]const u8 {
        return switch (self) {
            .no_variant => "no supported grammar variant was provided",
            .missing_variants => "Cannot read properties of undefined (reading 'openai_lark')",
            .null_variants => "Cannot read properties of null (reading 'openai_lark')",
            .null_schema => "Cannot read properties of null (reading 'type')",
            .object_schema => "grammar constrained sampling requires an object parameter schema",
            .required_string => "grammar constrained sampling requires exactly one required string property",
            .missing_property => |key| try std.fmt.allocPrint(a, "grammar constrained sampling requires a properties entry for {s}", .{key}),
            .property_type => |key| try std.fmt.allocPrint(a, "grammar constrained sampling property {s} must have type string", .{key}),
        };
    }
};
pub fn resolveGrammarConstrainedSampling(tool: t.Tool, supports: bool) !?GrammarConstrainedSampling {
    var failure: GrammarFailure = undefined;
    return resolveGrammar(tool, supports, &failure);
}
pub fn resolveGrammarConstrainedSamplingWithError(a: std.mem.Allocator, tool: t.Tool, supports: bool, error_message: ?*?[]const u8) !?GrammarConstrainedSampling {
    var failure: GrammarFailure = undefined;
    return resolveGrammar(tool, supports, &failure) catch |err| {
        if (err == error.MissingGrammarVariant or err == error.InvalidGrammarToolSchema) if (error_message) |slot| {
            slot.* = switch (failure) {
                .missing_variants, .null_variants => try failure.text(a),
                else => try std.fmt.allocPrint(a, "Tool \"{s}\" cannot use grammar constrained sampling: {s}.", .{ tool.name, try failure.text(a) }),
            };
        };
        return err;
    };
}
fn invalidGrammar(failure: *GrammarFailure, reason: GrammarFailure) error{InvalidGrammarToolSchema} {
    failure.* = reason;
    return error.InvalidGrammarToolSchema;
}
fn grammarProperty(properties: Value, key: []const u8) !?Value {
    if (properties == .null) return null;
    if (j.get(properties, key)) |value| return value;
    if (properties == .array) {
        if (std.mem.eql(u8, key, "length")) return j.num(@floatFromInt(properties.array.items.len));
        if (j.arrayIndex(key)) |index| if (index < properties.array.items.len) return properties.array.items[index];
    }
    if (properties == .string) {
        const length = try @import("../utils/unicode.zig").utf16Length(properties.string);
        if (std.mem.eql(u8, key, "length")) return j.num(@floatFromInt(length));
        if (j.arrayIndex(key)) |index| if (index < length) return .{ .bool = true };
    }
    // Inherited native members are truthy but have no schema type. A marker
    // preserves that diagnostic distinction without constructing JS functions.
    const object_members = [_][]const u8{ "constructor", "__defineGetter__", "__defineSetter__", "hasOwnProperty", "__lookupGetter__", "__lookupSetter__", "isPrototypeOf", "propertyIsEnumerable", "toLocaleString", "toString", "valueOf", "__proto__" };
    for (object_members) |name| if (std.mem.eql(u8, key, name)) return .{ .bool = true };
    const array_members = [_][]const u8{ "at", "concat", "copyWithin", "fill", "find", "findIndex", "findLast", "findLastIndex", "lastIndexOf", "pop", "push", "reverse", "shift", "unshift", "slice", "sort", "splice", "includes", "indexOf", "join", "keys", "entries", "values", "forEach", "filter", "flat", "flatMap", "map", "every", "some", "reduce", "reduceRight", "toReversed", "toSorted", "toSpliced", "with" };
    if (properties == .array) for (array_members) |name| if (std.mem.eql(u8, key, name)) {
        return .{ .bool = true };
    };
    const string_members = [_][]const u8{ "anchor", "at", "big", "blink", "bold", "charAt", "charCodeAt", "codePointAt", "concat", "endsWith", "fixed", "fontcolor", "fontsize", "includes", "indexOf", "isWellFormed", "italics", "lastIndexOf", "link", "localeCompare", "match", "matchAll", "normalize", "padEnd", "padStart", "repeat", "replace", "replaceAll", "search", "slice", "small", "split", "startsWith", "strike", "sub", "substr", "substring", "sup", "toLocaleLowerCase", "toLocaleUpperCase", "toLowerCase", "toUpperCase", "toWellFormed", "trim", "trimEnd", "trimStart", "trimLeft", "trimRight" };
    if (properties == .string) for (string_members) |name| if (std.mem.eql(u8, key, name)) {
        return .{ .bool = true };
    };
    if (j.number(properties) != null) for ([_][]const u8{ "toExponential", "toFixed", "toPrecision" }) |name| if (std.mem.eql(u8, key, name)) {
        return .{ .bool = true };
    };
    return null;
}
fn resolveGrammar(tool: t.Tool, supports: bool, failure: *GrammarFailure) !?GrammarConstrainedSampling {
    const config = tool.constrainedSampling orelse return null;
    if (!supports or !std.mem.eql(u8, j.field(config, "type") orelse "", "grammar")) return null;
    const variants = j.get(config, "variants") orelse {
        failure.* = .missing_variants;
        return error.MissingGrammarVariant;
    };
    if (variants == .null) {
        failure.* = .null_variants;
        return error.MissingGrammarVariant;
    }
    const lark = j.field(variants, "openai_lark");
    const regex = j.field(variants, "openai_regex");
    const use_lark = if (lark) |v| !try @import("../utils/unicode.zig").isBlank(v) else false;
    if (!use_lark and (regex == null or try @import("../utils/unicode.zig").isBlank(regex.?))) {
        failure.* = .no_variant;
        return error.MissingGrammarVariant;
    }
    const schema = tool.parameters.value;
    if (schema == .null) return invalidGrammar(failure, .null_schema);
    if (!std.mem.eql(u8, j.field(schema, "type") orelse "", "object")) return invalidGrammar(failure, .object_schema);
    const required = j.get(schema, "required") orelse return invalidGrammar(failure, .required_string);
    if (required != .array or required.array.items.len != 1 or required.array.items[0] != .string) return invalidGrammar(failure, .required_string);
    const key = required.array.items[0].string;
    const properties = j.get(schema, "properties") orelse return invalidGrammar(failure, .{ .missing_property = key });
    const property = (try grammarProperty(properties, key)) orelse return invalidGrammar(failure, .{ .missing_property = key });
    if (!j.truthy(property)) return invalidGrammar(failure, .{ .missing_property = key });
    if (!std.mem.eql(u8, j.field(property, "type") orelse "", "string")) return invalidGrammar(failure, .{ .property_type = key });
    return .{ .format = if (use_lark) .lark else .regex, .definition = if (use_lark) lark.? else regex.?, .inputProperty = key };
}
pub fn getGrammarToolInput(arguments: Value, property: []const u8) ![]const u8 {
    return j.field(arguments, property) orelse error.InvalidGrammarToolInput;
}
pub fn getGrammarToolInputWithError(a: std.mem.Allocator, tool_name: []const u8, arguments: Value, property: []const u8, error_message: ?*?[]const u8) ![]const u8 {
    if (arguments == .null) {
        if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Cannot read properties of null (reading '{s}')", .{property});
        return error.InvalidGrammarToolInput;
    }
    return getGrammarToolInput(arguments, property) catch |err| {
        if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Grammar tool call \"{s}\" requires argument \"{s}\" to be a string.", .{ tool_name, property });
        return err;
    };
}
pub fn createGrammarToolInputProperties(a: std.mem.Allocator, tools: []const t.Tool, supports: bool) !std.StringHashMapUnmanaged([]const u8) {
    return createGrammarToolInputPropertiesWithError(a, tools, supports, null);
}
pub fn createGrammarToolInputPropertiesWithError(a: std.mem.Allocator, tools: []const t.Tool, supports: bool, error_message: ?*?[]const u8) !std.StringHashMapUnmanaged([]const u8) {
    var result: std.StringHashMapUnmanaged([]const u8) = .{};
    errdefer result.deinit(a);
    for (tools) |tool| if (try resolveGrammarConstrainedSamplingWithError(a, tool, supports, error_message)) |grammar| try result.put(a, tool.name, grammar.inputProperty);
    return result;
}
