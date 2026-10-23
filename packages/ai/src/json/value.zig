//! JSON tree operations. External decoding and encoding always use std.json.
const std = @import("std");
pub const Value = std.json.Value;
pub const Scanner = @import("utf16_tokens.zig").Scanner;
pub const writeString = @import("utf16_strings.zig").write;
pub const writeField = @import("utf16_strings.zig").field;
pub const writeStringChars = @import("utf16_strings.zig").chars;
pub fn object() Value {
    return .{ .object = .empty };
}
pub fn map(a: std.mem.Allocator) Value {
    _ = a;
    return object();
}
pub fn array(a: std.mem.Allocator) Value {
    return .{ .array = std.array_list.Managed(Value).init(a) };
}
pub fn str(s: []const u8) Value {
    return .{ .string = s };
}
pub fn num(n: f64) Value {
    return .{ .float = n };
}
pub fn int(n: i64) Value {
    return .{ .integer = n };
}
pub fn put(a: std.mem.Allocator, v: *Value, key: []const u8, value: Value) !void {
    if (v.* != .object) return error.UnexpectedToken;
    try v.object.put(a, key, value);
}
pub fn get(v: Value, key: []const u8) ?Value {
    return if (v == .object) v.object.get(key) else null;
}
/// A nullish lookup for source expressions that use ?? rather than ||.
/// False, zero and empty strings remain present values.
pub fn getNonNull(v: Value, key: []const u8) ?Value {
    const value = get(v, key) orelse return null;
    return if (value == .null) null else value;
}
pub fn text(v: Value) ?[]const u8 {
    return if (v == .string) v.string else null;
}
pub fn field(v: Value, key: []const u8) ?[]const u8 {
    return text(get(v, key) orelse return null);
}
pub fn number(v: Value) ?f64 {
    return switch (v) {
        .integer => |n| @floatFromInt(n),
        .float => |n| n,
        .number_string => |s| std.fmt.parseFloat(f64, s) catch null,
        else => null,
    };
}
pub fn boolean(v: Value) ?bool {
    return if (v == .bool) v.bool else null;
}
pub fn truthy(v: Value) bool {
    return switch (v) {
        .null => false,
        .bool => |b| b,
        .string => |s| s.len > 0,
        .integer => |n| n != 0,
        .float => |n| n != 0 and !std.math.isNan(n),
        .number_string => |s| if (std.fmt.parseFloat(f64, s)) |n| n != 0 and !std.math.isNan(n) else |_| false,
        .array, .object => true,
    };
}
/// Object spread is a tree transformation. String properties use JavaScript's
/// UTF-16 indices; standard codecs own all Unicode conversion and JSON output.
pub fn objectSpread(a: std.mem.Allocator, input: Value) anyerror!Value {
    if (input == .object) return clone(a, input);
    var out = map(a);
    switch (input) {
        .array => |items| for (items.items, 0..) |item, index| {
            try put(a, &out, try std.fmt.allocPrint(a, "{d}", .{index}), try clone(a, item));
        },
        .string => |text_value| {
            const units = try std.unicode.wtf8ToWtf16LeAlloc(a, text_value);
            defer a.free(units);
            for (0..units.len) |index| try put(a, &out, try std.fmt.allocPrint(a, "{d}", .{index}), str(try std.unicode.wtf16LeToWtf8Alloc(a, units[index .. index + 1])));
        },
        else => {},
    }
    return out;
}
/// JSON.parse exposes binary64 numbers. Keep small native integers convenient,
/// but round larger integers and decimal tokens through the standard float
/// codec before exposing decoded dynamic values.
pub fn normalizeParsed(value: *Value) !void {
    switch (value.*) {
        .integer => |n| if (n > 9007199254740991 or n < -9007199254740991) {
            value.* = num(@floatFromInt(n));
        },
        .number_string => |number_text| {
            const number_value = try std.fmt.parseFloat(f64, number_text);
            // The integer decoder erases the sign of JSON's -0. Keep that
            // value as binary64 while retaining ordinary small integer tags.
            if (number_value != 0 or !std.math.signbit(number_value)) {
                if (std.fmt.parseInt(i64, number_text, 10)) |integer| {
                    if (integer >= -9007199254740991 and integer <= 9007199254740991) {
                        value.* = int(integer);
                        return;
                    }
                } else |_| {}
            }
            value.* = num(number_value);
        },
        .array => |*items| for (items.items) |*item| try normalizeParsed(item),
        .object => |*entries| {
            var it = entries.iterator();
            while (it.next()) |entry| try normalizeParsed(entry.value_ptr);
        },
        else => {},
    }
}
pub fn parseFromSlice(comptime T: type, a: std.mem.Allocator, bytes: []const u8, options: std.json.ParseOptions) !std.json.Parsed(T) {
    const parse_options = if (T == Value) native: {
        var configured = options;
        configured.parse_numbers = false;
        break :native configured;
    } else options;
    var scanner = Scanner.initCompleteInput(a, bytes);
    defer scanner.deinit();
    var configured = parse_options;
    if (configured.max_value_len == null) configured.max_value_len = bytes.len;
    var result = try std.json.parseFromTokenSource(T, a, &scanner, configured);
    errdefer result.deinit();
    if (T == Value) try normalizeParsed(&result.value);
    return result;
}
pub fn parseFromSliceLeaky(comptime T: type, a: std.mem.Allocator, bytes: []const u8, options: std.json.ParseOptions) !T {
    const parse_options = if (T == Value) native: {
        var configured = options;
        configured.parse_numbers = false;
        break :native configured;
    } else options;
    var scanner = Scanner.initCompleteInput(a, bytes);
    defer scanner.deinit();
    var configured = parse_options;
    if (configured.max_value_len == null) configured.max_value_len = bytes.len;
    var result = try std.json.parseFromTokenSourceLeaky(T, a, &scanner, configured);
    if (T == Value) try normalizeParsed(&result);
    return result;
}
/// Canonical ECMAScript own-property array index, using standard numeric codecs.
pub fn arrayIndex(key: []const u8) ?u32 {
    const index = std.fmt.parseInt(u32, key, 10) catch return null;
    if (index == std.math.maxInt(u32)) return null;
    var buffer: [10]u8 = undefined;
    const canonical = std.fmt.bufPrint(&buffer, "{d}", .{index}) catch unreachable;
    return if (std.mem.eql(u8, key, canonical)) index else null;
}
pub const ObjectEntry = struct { key: []const u8, node: *const Value, index: ?u32, order: usize };
/// Allocation-free enumeration for standard serializer hooks, which receive
/// no allocator. Ordinary properties retain insertion order.
const OrderedIterator = struct {
    object_value: std.json.ObjectMap,
    after: ?u32 = null,
    numeric: bool = true,
    position: usize = 0,
    fn next(self: *@This()) ?struct { key: []const u8, value: Value } {
        if (self.numeric) {
            var selected: ?usize = null;
            var smallest: u32 = std.math.maxInt(u32);
            for (self.object_value.keys(), 0..) |key, index| if (arrayIndex(key)) |number_value| {
                if ((self.after == null or number_value > self.after.?) and number_value < smallest) {
                    selected = index;
                    smallest = number_value;
                }
            };
            if (selected) |index| {
                self.after = smallest;
                return .{ .key = self.object_value.keys()[index], .value = self.object_value.values()[index] };
            }
            self.numeric = false;
        }
        while (self.position < self.object_value.count()) {
            const index = self.position;
            self.position += 1;
            const key = self.object_value.keys()[index];
            if (arrayIndex(key) == null) return .{ .key = key, .value = self.object_value.values()[index] };
        }
        return null;
    }
};
fn writeNumber(writer: *std.json.Stringify, value: f64) std.json.Stringify.Error!void {
    if (!std.math.isFinite(value)) return writer.write(null);
    // A binary64 decimal plus the standard scalar scanner fits this bounded
    // scratch region. The standard parser supplies the numeric token, and the
    // standard serializer emits it; no raw JSON text is emitted here.
    var storage: [4096]u8 = undefined;
    var scratch = std.heap.FixedBufferAllocator.init(&storage);
    const decimal = @import("../utils/numbers.zig").toString(scratch.allocator(), value) catch return error.WriteFailed;
    const token = std.json.parseFromSliceLeaky(Value, scratch.allocator(), decimal, .{ .parse_numbers = false, .allocate = .alloc_if_needed }) catch return error.WriteFailed;
    try writer.write(token);
}
/// Standard writer composition with JSON.stringify's binary64 and non-finite
/// semantics, including values nested in opaque metadata trees.
pub fn writeCompatible(writer: *std.json.Stringify, value: anytype) std.json.Stringify.Error!void {
    const T = @TypeOf(value);
    if (T == Value) return switch (value) {
        .float => |number_value| writeNumber(writer, number_value),
        .integer => |number_value| if (number_value > 9007199254740991 or number_value < -9007199254740991) writeNumber(writer, @floatFromInt(number_value)) else writer.write(number_value),
        .number_string => |text_value| writeNumber(writer, std.fmt.parseFloat(f64, text_value) catch return error.WriteFailed),
        .array => |items| blk: {
            try writer.beginArray();
            for (items.items) |item| try writeCompatible(writer, item);
            try writer.endArray();
            break :blk;
        },
        .object => |fields| blk: {
            try writer.beginObject();
            var entries = OrderedIterator{ .object_value = fields };
            while (entries.next()) |entry| {
                try writeField(writer, entry.key);
                try writeCompatible(writer, entry.value);
            }
            try writer.endObject();
            break :blk;
        },
        .string => |text_value| writeString(writer, text_value),
        else => writer.write(value),
    };
    switch (@typeInfo(T)) {
        .float, .comptime_float => try writeNumber(writer, @floatCast(value)),
        .int => |info| {
            if (writer.options.emit_nonportable_numbers_as_strings) return writer.write(value);
            if (info.bits <= 53) return writer.write(value);
            if (value > 9007199254740991 or (info.signedness == .signed and value < -9007199254740991)) try writeNumber(writer, @floatFromInt(value)) else try writer.write(value);
        },
        .comptime_int => if (value > 9007199254740991 or value < -9007199254740991) {
            try writeNumber(writer, @floatFromInt(value));
        } else {
            try writer.write(value);
        },
        .optional => if (value) |present| try writeCompatible(writer, present) else try writer.write(null),
        .pointer => |pointer| switch (pointer.size) {
            .one => try writeCompatible(writer, value.*),
            .slice => if (pointer.child == u8) {
                try writeString(writer, value);
            } else {
                try writer.beginArray();
                for (value) |item| try writeCompatible(writer, item);
                try writer.endArray();
            },
            else => try writer.write(value),
        },
        .array => |info| if (info.child == u8) {
            try writeString(writer, &value);
        } else {
            try writer.beginArray();
            for (value) |item| try writeCompatible(writer, item);
            try writer.endArray();
        },
        .@"struct" => |info| {
            if (@hasDecl(T, "jsonStringify")) return writer.write(value);
            if (info.is_tuple) try writer.beginArray() else try writer.beginObject();
            inline for (info.field_names, info.field_types) |name, Field| {
                if (Field == void) continue;
                if (@typeInfo(Field) == .optional) {
                    if (!writer.options.emit_null_optional_fields and @field(value, name) == null) continue;
                }
                if (!info.is_tuple) try writeField(writer, name);
                try writeCompatible(writer, @field(value, name));
            }
            if (info.is_tuple) try writer.endArray() else try writer.endObject();
        },
        else => try writer.write(value),
    }
}
/// Borrow the original values in ECMAScript own-property order. The caller
/// owns only the entry slice; keys and nodes retain their original lifetimes.
pub fn objectEntries(a: std.mem.Allocator, object_value: std.json.ObjectMap) ![]ObjectEntry {
    const result = try a.alloc(ObjectEntry, object_value.count());
    var it = object_value.iterator();
    var index: usize = 0;
    while (it.next()) |entry| : (index += 1) result[index] = .{ .key = entry.key_ptr.*, .node = entry.value_ptr, .index = arrayIndex(entry.key_ptr.*), .order = index };
    std.mem.sort(ObjectEntry, result, {}, struct {
        fn less(_: void, left: ObjectEntry, right: ObjectEntry) bool {
            if (left.index) |value| return if (right.index) |other| value < other else true;
            if (right.index != null) return false;
            return left.order < right.order;
        }
    }.less);
    return result;
}
/// A transient JSON tree supplies JSON.stringify's number and property-order
/// semantics. The standard serializer still emits every token and escape.
fn stringifyNumber(a: std.mem.Allocator, number_value: f64) !Value {
    if (!std.math.isFinite(number_value)) return .null;
    const decimal = try @import("../utils/numbers.zig").toString(a, number_value);
    // Decode the scalar spelling with std.json, retaining its numeric token
    // representation. std.json.Stringify then owns emission of that token.
    const token = try std.json.parseFromSliceLeaky(Value, a, decimal, .{ .parse_numbers = false });
    if (token != .number_string) return error.InvalidJsonNumber;
    return token;
}
/// JSON.stringify-compatible tree projection. Containers and numeric tokens
/// use a; unchanged strings/keys borrow input until copied by their next owner.
pub fn stringifyTree(a: std.mem.Allocator, input: Value) anyerror!Value {
    return switch (input) {
        .float => |n| try stringifyNumber(a, n),
        .integer => |n| if (n > 9007199254740991 or n < -9007199254740991) try stringifyNumber(a, @floatFromInt(n)) else input,
        .number_string => |number_text| blk: {
            const parsed = try std.json.parseFromSliceLeaky(Value, a, number_text, .{ .parse_numbers = false });
            if (parsed != .number_string) return error.InvalidJsonNumber;
            const n = try std.fmt.parseFloat(f64, parsed.number_string);
            break :blk try stringifyNumber(a, n);
        },
        .array => |items| blk: {
            var out = array(a);
            for (items.items) |item| try out.array.append(try stringifyTree(a, item));
            break :blk out;
        },
        .object => |entries| blk: {
            const ordered = try objectEntries(a, entries);
            defer a.free(ordered);
            var out = map(a);
            for (ordered) |entry| try put(a, &out, entry.key, try stringifyTree(a, entry.node.*));
            break :blk out;
        },
        else => input,
    };
}
fn standardTree(a: std.mem.Allocator, value: anytype, options: std.json.Stringify.Options) !Value {
    const bytes = try std.json.Stringify.valueAlloc(a, Compatible(@TypeOf(value)){ .value = value }, options);
    return parseFromSliceLeaky(Value, a, bytes, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always });
}
/// Convert native records to a typed tree before standard serialization. Native
/// tagged/transparent unions declare their representation; custom serializers
/// retain their own standard-json hooks.
pub fn toValue(a: std.mem.Allocator, value: anytype, options: std.json.Stringify.Options) anyerror!Value {
    const T = @TypeOf(value);
    if (T == Value) return value;
    return switch (@typeInfo(T)) {
        .optional => if (value) |present| toValue(a, present, options) else .null,
        .null => .null,
        .bool => .{ .bool = value },
        .void => map(a),
        .float, .comptime_float => num(@floatCast(value)),
        .int, .comptime_int => blk: {
            if (options.emit_nonportable_numbers_as_strings) break :blk try standardTree(a, value, options);
            break :blk if (std.math.cast(i64, value)) |integer| (if (integer >= -9007199254740991 and integer <= 9007199254740991) int(integer) else num(@floatFromInt(value))) else num(@floatFromInt(value));
        },
        .pointer => |pointer| switch (pointer.size) {
            .one => toValue(a, value.*, options),
            .slice => blk: {
                if (pointer.child == u8 and !options.emit_strings_as_arrays) break :blk str(value);
                var out = array(a);
                for (value) |item| try out.array.append(try toValue(a, item, options));
                break :blk out;
            },
            else => standardTree(a, value, options),
        },
        .array => |info| blk: {
            if (info.child == u8 and !options.emit_strings_as_arrays) break :blk str(try a.dupe(u8, &value));
            var out = array(a);
            for (value) |item| try out.array.append(try toValue(a, item, options));
            break :blk out;
        },
        .vector => |info| blk: {
            const items: [info.len]info.child = value;
            break :blk try toValue(a, items, options);
        },
        .@"struct" => |info| blk: {
            if (@hasDecl(T, "jsonToValue")) break :blk try value.jsonToValue(a, options);
            if (@hasDecl(T, "jsonStringify")) break :blk try standardTree(a, value, options);
            var out = if (info.is_tuple) array(a) else map(a);
            inline for (info.field_names, info.field_types) |name, field_type| {
                if (field_type == void) continue;
                const field_value = @field(value, name);
                const include = if (@typeInfo(field_type) == .optional) options.emit_null_optional_fields or field_value != null else true;
                if (include) {
                    const child = try toValue(a, field_value, options);
                    if (info.is_tuple) try out.array.append(child) else try put(a, &out, name, child);
                }
            }
            break :blk out;
        },
        .@"union" => blk: {
            if (@hasDecl(T, "jsonToValue")) break :blk try value.jsonToValue(a, options);
            if (@hasDecl(T, "jsonTransparent")) break :blk switch (value) {
                inline else => |payload| try toValue(a, payload, options),
            };
            if (@hasDecl(T, "jsonDiscriminator")) {
                break :blk taggedToValue(value, T.jsonDiscriminator, a, options);
            }
            break :blk try standardTree(a, value, options);
        },
        else => standardTree(a, value, options),
    };
}
fn Compatible(comptime T: type) type {
    return struct {
        value: T,
        pub fn jsonStringify(self: @This(), writer: *std.json.Stringify) !void {
            try writeCompatible(writer, self.value);
        }
    };
}
pub fn stringify(a: std.mem.Allocator, value: anytype, options: std.json.Stringify.Options) ![]u8 {
    var arena = std.heap.ArenaAllocator.init(a);
    defer arena.deinit();
    const temporary = arena.allocator();
    return std.json.Stringify.valueAlloc(a, Compatible(Value){ .value = try stringifyTree(temporary, try toValue(temporary, value, options)) }, options);
}
pub fn clone(a: std.mem.Allocator, v: Value) std.mem.Allocator.Error!Value {
    return switch (v) {
        .string => |s| str(try a.dupe(u8, s)),
        .number_string => |s| .{ .number_string = try a.dupe(u8, s) },
        .array => |items| blk: {
            var out = array(a);
            errdefer deinitCloned(a, &out);
            for (items.items) |item| {
                var child = try clone(a, item);
                errdefer deinitCloned(a, &child);
                try out.array.append(child);
            }
            break :blk out;
        },
        .object => |obj| blk: {
            var out = map(a);
            errdefer deinitCloned(a, &out);
            var it = obj.iterator();
            while (it.next()) |e| {
                const key = try a.dupe(u8, e.key_ptr.*);
                errdefer a.free(key);
                var child = try clone(a, e.value_ptr.*);
                errdefer deinitCloned(a, &child);
                try out.object.put(a, key, child);
            }
            break :blk out;
        },
        else => v,
    };
}
/// Only for a tree returned by clone(), whose strings and children are owned.
/// Trees assembled with borrowed strings use their caller's lifetime instead.
pub fn deinitCloned(a: std.mem.Allocator, value: *Value) void {
    switch (value.*) {
        .string => |bytes| a.free(bytes),
        .number_string => |bytes| a.free(bytes),
        .array => |*items| {
            for (items.items) |*item| deinitCloned(a, item);
            items.deinit();
        },
        .object => |*entries| {
            var it = entries.iterator();
            while (it.next()) |entry| {
                a.free(entry.key_ptr.*);
                deinitCloned(a, entry.value_ptr);
            }
            entries.deinit(a);
        },
        else => {},
    }
    value.* = undefined;
}
pub fn from(a: std.mem.Allocator, value: anytype) !Value {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    return clone(a, try toValue(temporary.allocator(), value, .{ .emit_null_optional_fields = false }));
}
/// Use only at a source JSON.parse(JSON.stringify(value)) boundary. Ordinary
/// internal construction keeps non-finite numbers visible to payload observers.
pub fn roundTrip(a: std.mem.Allocator, value: anytype) !Value {
    const encoded = try stringify(a, value, .{ .emit_null_optional_fields = false });
    defer a.free(encoded);
    const parsed = try parseFromSlice(Value, a, encoded, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always });
    defer parsed.deinit();
    return clone(a, parsed.value);
}
pub fn taggedStringify(value: anytype, comptime discriminator: []const u8, s: *std.json.Stringify) !void {
    try s.beginObject();
    switch (value) {
        inline else => |payload| {
            if (comptime @hasField(@TypeOf(payload), "jsonFields")) {
                try Record(@TypeOf(payload)).writeFields(payload, s, .{ .name = discriminator, .value = @tagName(value) });
            } else {
                try writeField(s, discriminator);
                try s.write(@tagName(value));
                const info = @typeInfo(@TypeOf(payload)).@"struct";
                inline for (info.field_names, info.field_types) |name, field_type| {
                    if (comptime std.mem.eql(u8, name, discriminator)) continue;
                    const x = @field(payload, name);
                    if (@typeInfo(field_type) == .optional) {
                        if (x) |present| {
                            try writeField(s, name);
                            try writeCompatible(s, present);
                        }
                    } else {
                        try writeField(s, name);
                        try writeCompatible(s, x);
                    }
                }
            }
        },
    }
    try s.endObject();
}
pub fn taggedToValue(value: anytype, comptime discriminator: []const u8, a: std.mem.Allocator, options: std.json.Stringify.Options) !Value {
    var fields = switch (value) {
        inline else => |payload| try toValue(a, payload, options),
    };
    if (fields != .object) return error.UnexpectedToken;
    if (fields.object.contains(discriminator)) {
        try put(a, &fields, discriminator, str(@tagName(value)));
        return fields;
    }
    var out = map(a);
    try put(a, &out, discriminator, str(@tagName(value)));
    var iterator = fields.object.iterator();
    while (iterator.next()) |entry| try put(a, &out, entry.key_ptr.*, entry.value_ptr.*);
    return out;
}
pub fn taggedParse(comptime T: type, discriminator: []const u8, a: std.mem.Allocator, source: anytype, options: std.json.ParseOptions) !T {
    var configured = options;
    configured.duplicate_field_behavior = .use_last;
    configured.parse_numbers = false;
    var value = try Value.jsonParse(a, source, configured);
    try normalizeParsed(&value);
    return taggedFromValue(T, discriminator, a, value, options);
}
pub fn taggedFromValue(comptime T: type, discriminator: []const u8, a: std.mem.Allocator, value: Value, options: std.json.ParseOptions) !T {
    const tag = field(value, discriminator) orelse return error.UnexpectedToken;
    const info = @typeInfo(T).@"union";
    inline for (info.field_names, info.field_types) |name, field_type| {
        if (std.mem.eql(u8, name, tag)) {
            var opts = options;
            opts.ignore_unknown_fields = true;
            const payload = try std.json.innerParseFromValue(field_type, a, value, opts);
            return @unionInit(T, name, payload);
        }
    }
    return error.UnknownField;
}
pub fn Tagged(comptime T: type, comptime discriminator: []const u8) type {
    return struct {
        pub fn jsonStringify(self: T, s: *std.json.Stringify) !void {
            try taggedStringify(self, discriminator, s);
        }
        pub fn jsonParse(a: std.mem.Allocator, source: anytype, options: std.json.ParseOptions) !T {
            return taggedParse(T, discriminator, a, source, options);
        }
        pub fn jsonParseFromValue(a: std.mem.Allocator, value: Value, options: std.json.ParseOptions) !T {
            return taggedFromValue(T, discriminator, a, value, options);
        }
    };
}

/// Typed public records retain the decoded object's property order, explicit
/// nulls and unknown members in jsonFields. Known native fields remain the
/// editable values. All token parsing/escaping stays in the standard library.
fn recordEqual(left: anytype, right: @TypeOf(left)) bool {
    switch (@typeInfo(@TypeOf(left))) {
        .pointer => |info| {
            if (info.size != .slice) return left == right;
            if (left.len != right.len) return false;
            for (left, right) |a, b| if (!recordEqual(a, b)) return false;
            return true;
        },
        .@"struct" => |info| {
            inline for (info.field_names) |name| if (!recordEqual(@field(left, name), @field(right, name))) return false;
            return true;
        },
        .@"union" => |info| {
            const Tag = info.tag_type orelse return std.meta.eql(left, right);
            if (@as(Tag, left) != @as(Tag, right)) return false;
            return switch (left) {
                inline else => |value, tag| recordEqual(value, @field(right, @tagName(tag))),
            };
        },
        .optional => {
            const a = left orelse return right == null;
            const b = right orelse return false;
            return recordEqual(a, b);
        },
        .array => {
            for (left, right) |a, b| if (!recordEqual(a, b)) return false;
            return true;
        },
        .float => {
            if (std.math.isNan(left) and std.math.isNan(right)) return true;
            if (left == 0 and right == 0) return std.math.signbit(left) == std.math.signbit(right);
            return left == right;
        },
        .void => return true,
        else => return std.meta.eql(left, right),
    }
}
pub fn Record(comptime T: type) type {
    return struct {
        const info = @typeInfo(T).@"struct";
        fn metadata(comptime name: []const u8) bool {
            if (@hasDecl(T, "jsonRecordMetadata")) {
                inline for (T.jsonRecordMetadata) |field_name| {
                    if (std.mem.eql(u8, name, field_name)) return true;
                }
            }
            return std.mem.eql(u8, name, "jsonFields") or std.mem.eql(u8, name, "jsonAssigned") or std.mem.eql(u8, name, "jsonOmitted") or std.mem.eql(u8, name, "jsonRetained") or std.mem.eql(u8, name, "jsonContentProperties") or (@hasDecl(T, "jsonExtra") and std.mem.eql(u8, name, "extra"));
        }
        fn known(name: []const u8) bool {
            inline for (info.field_names) |field_name| {
                if (comptime metadata(field_name)) continue;
                if (std.mem.eql(u8, name, field_name)) return true;
            }
            return false;
        }
        fn explicitNull(self: T, name: []const u8) bool {
            return if (get(self.jsonFields, name)) |previous| previous == .null else false;
        }
        fn include(self: T, comptime name: []const u8, options: std.json.Stringify.Options) bool {
            _ = options; // Source property presence also governs direct std.json hooks.
            const mask = fieldMask(T, name);
            if (self.jsonOmitted & mask != 0) return false;
            if (retainedValue(self, name)) return true;
            const NativeField = @FieldType(T, name);
            if (comptime @typeInfo(NativeField) == .@"struct" or @typeInfo(NativeField) == .@"union") {
                if (@hasDecl(NativeField, "jsonIsUndefined")) {
                    if (@field(self, name).jsonIsUndefined()) return false;
                }
            }
            if (self.jsonAssigned & mask != 0) return true;
            // A parsed record can have native defaults for absent wire fields.
            // Do not materialize those defaults merely by round-tripping it.
            if (self.jsonFields == .object and get(self.jsonFields, name) == null) {
                inline for (info.field_names, info.field_types, info.field_attrs) |field_name, Field, attrs| {
                    if (comptime std.mem.eql(u8, name, field_name)) {
                        if (comptime attrs.defaultValue(Field)) |default| {
                            if (recordEqual(@field(self, name), default)) return false;
                        }
                    }
                }
            }
            return if (@typeInfo(@FieldType(T, name)) == .optional)
                @field(self, name) != null or explicitNull(self, name)
            else
                true;
        }
        fn retainedValue(self: T, comptime name: []const u8) bool {
            if (self.jsonRetained & fieldMask(T, name) == 0 or self.jsonAssigned & fieldMask(T, name) != 0) return false;
            inline for (info.field_names, info.field_types, info.field_attrs) |field_name, Field, attrs| if (comptime std.mem.eql(u8, name, field_name)) {
                if (comptime attrs.defaultValue(Field)) |default| return recordEqual(@field(self, name), default);
            };
            // A required field has no declared default to compare against.
            // Its explicit retained value survives until markPresent clears it.
            return true;
        }
        pub fn jsonToValue(self: T, a: std.mem.Allocator, options: std.json.Stringify.Options) !Value {
            var out = map(a);
            const unknown = if (@hasDecl(T, "jsonExtra")) self.jsonExtra() else self.jsonFields;
            if (self.jsonFields == .object) {
                var entries = self.jsonFields.object.iterator();
                while (entries.next()) |entry| {
                    inline for (info.field_names) |name| {
                        if (comptime metadata(name)) continue;
                        if (std.mem.eql(u8, entry.key_ptr.*, name) and include(self, name, options))
                            try put(a, &out, name, if (retainedValue(self, name)) get(self.jsonFields, name) orelse .null else try toValue(a, @field(self, name), options));
                    }
                    if (!known(entry.key_ptr.*)) if (get(unknown, entry.key_ptr.*)) |value| try put(a, &out, entry.key_ptr.*, value);
                }
            }
            inline for (info.field_names) |name| {
                if (comptime metadata(name)) continue;
                if (get(self.jsonFields, name) == null and include(self, name, options))
                    try put(a, &out, name, if (retainedValue(self, name)) get(self.jsonFields, name) orelse .null else try toValue(a, @field(self, name), options));
            }
            if (comptime @hasDecl(T, "jsonExtra")) {
                if (unknown == .object) {
                    var entries = unknown.object.iterator();
                    while (entries.next()) |entry| {
                        if (!known(entry.key_ptr.*) and get(self.jsonFields, entry.key_ptr.*) == null) try put(a, &out, entry.key_ptr.*, entry.value_ptr.*);
                    }
                }
            }
            return out;
        }
        pub fn jsonStringify(self: T, writer: *std.json.Stringify) !void {
            try writer.beginObject();
            try writeFields(self, writer, null);
            try writer.endObject();
        }
        pub fn writeFields(self: T, writer: *std.json.Stringify, tag: ?struct { name: []const u8, value: []const u8 }) !void {
            const unknown = if (@hasDecl(T, "jsonExtra")) self.jsonExtra() else self.jsonFields;
            // Integer-named unknown fields enumerate before all ordinary
            // fields, even when newly added to a record's extra properties.
            if (unknown == .object) {
                var entries = OrderedIterator{ .object_value = unknown.object };
                while (entries.next()) |entry| {
                    if (arrayIndex(entry.key) == null) break;
                    if (!known(entry.key)) {
                        try writeField(writer, entry.key);
                        try writeCompatible(writer, entry.value);
                    }
                }
            }
            if (tag) |override| if (get(self.jsonFields, override.name) == null) {
                try writeField(writer, override.name);
                try writer.write(override.value);
            };
            if (self.jsonFields == .object) {
                var entries = self.jsonFields.object.iterator();
                while (entries.next()) |entry| {
                    if (arrayIndex(entry.key_ptr.*) != null) continue;
                    if (tag) |override| if (std.mem.eql(u8, entry.key_ptr.*, override.name)) {
                        try writeField(writer, override.name);
                        try writer.write(override.value);
                        continue;
                    };
                    inline for (info.field_names) |name| {
                        if (comptime metadata(name)) continue;
                        if (std.mem.eql(u8, entry.key_ptr.*, name) and include(self, name, writer.options)) {
                            try writeField(writer, name);
                            if (retainedValue(self, name)) try writeCompatible(writer, get(self.jsonFields, name) orelse .null) else try writeCompatible(writer, @field(self, name));
                        }
                    }
                    if (!known(entry.key_ptr.*)) if (get(unknown, entry.key_ptr.*)) |value| {
                        try writeField(writer, entry.key_ptr.*);
                        try writeCompatible(writer, value);
                    };
                }
            }
            inline for (info.field_names) |name| {
                if (comptime metadata(name)) continue;
                const is_tag = if (tag) |override| std.mem.eql(u8, name, override.name) else false;
                if (!is_tag and get(self.jsonFields, name) == null and include(self, name, writer.options)) {
                    try writeField(writer, name);
                    if (retainedValue(self, name)) try writeCompatible(writer, get(self.jsonFields, name) orelse .null) else try writeCompatible(writer, @field(self, name));
                }
            }
            if (comptime @hasDecl(T, "jsonExtra")) {
                if (unknown == .object) {
                    var entries = unknown.object.iterator();
                    while (entries.next()) |entry| {
                        if (arrayIndex(entry.key_ptr.*) != null) continue;
                        if (!known(entry.key_ptr.*) and get(self.jsonFields, entry.key_ptr.*) == null) {
                            try writeField(writer, entry.key_ptr.*);
                            try writeCompatible(writer, entry.value_ptr.*);
                        }
                    }
                }
            }
        }
        pub fn jsonParse(a: std.mem.Allocator, source: anytype, options: std.json.ParseOptions) !T {
            var configured = options;
            configured.duplicate_field_behavior = .use_last;
            configured.parse_numbers = false;
            var value = try Value.jsonParse(a, source, configured);
            try normalizeParsed(&value);
            return jsonParseFromValue(a, value, options);
        }
        fn parseField(comptime Field: type, a: std.mem.Allocator, value: Value, options: std.json.ParseOptions) !Field {
            const Native = if (@typeInfo(Field) == .optional) @typeInfo(Field).optional.child else Field;
            if (comptime @typeInfo(Native) == .float or @typeInfo(Native) == .int) {
                // std.json also accepts quoted numbers for typed numeric
                // fields. A decoded source object keeps that string value.
                if (value == .string) return error.UnexpectedToken;
            }
            return std.json.innerParseFromValue(Field, a, value, options);
        }
        pub fn jsonParseFromValue(a: std.mem.Allocator, value: Value, options: std.json.ParseOptions) !T {
            if (value != .object) return error.UnexpectedToken;
            var result: T = undefined;
            var nulls: u64 = 0;
            inline for (info.field_names, info.field_types, info.field_attrs) |name, Field, attrs| {
                if (comptime std.mem.eql(u8, name, "jsonFields")) @field(result, name) = try clone(a, value) else if (comptime metadata(name)) @field(result, name) = attrs.defaultValue(Field).? else if (get(value, name)) |field_name| {
                    @field(result, name) = parseField(Field, a, field_name, options) catch |err| blk: {
                        if (err == error.OutOfMemory) return err;
                        if (comptime attrs.defaultValue(Field)) |default| {
                            nulls |= fieldMask(T, name);
                            break :blk default;
                        }
                        return err;
                    };
                } else if (comptime attrs.defaultValue(Field)) |default| @field(result, name) = default else return error.MissingField;
            }
            result.jsonRetained = nulls;
            return result;
        }
    };
}
/// A native assignment of an absent field_name to its default value has no value
/// change to detect. Presence marks express explicit assignment/deletion
/// without allocation; ordinary changed values are detected automatically.
fn fieldMask(comptime T: type, comptime name: []const u8) u64 {
    inline for (@typeInfo(T).@"struct".field_names, 0..) |field_name, index| {
        if (comptime std.mem.eql(u8, name, field_name)) {
            if (index >= 64) @compileError("JSON record exceeds the property-presence mask");
            return @as(u64, 1) << @as(u6, @intCast(index));
        }
    }
    @compileError("Unknown JSON record field_name: " ++ name);
}
pub fn markPresent(record: anytype, comptime name: []const u8) void {
    const mask = fieldMask(@typeInfo(@TypeOf(record)).pointer.child, name);
    record.jsonAssigned |= mask;
    record.jsonOmitted &= ~mask;
    record.jsonRetained &= ~mask;
}
pub fn markAbsent(record: anytype, comptime name: []const u8) void {
    const T = @typeInfo(@TypeOf(record)).pointer.child;
    const mask = fieldMask(T, name);
    inline for (@typeInfo(T).@"struct".field_names, @typeInfo(T).@"struct".field_types, @typeInfo(T).@"struct".field_attrs) |field_name, Field, attrs| if (comptime std.mem.eql(u8, name, field_name)) {
        if (comptime attrs.defaultValue(Field)) |default| @field(record, name) = default;
    };
    record.jsonOmitted |= mask;
    record.jsonAssigned &= ~mask;
    record.jsonRetained &= ~mask;
}
pub fn fieldPresent(record: anytype, comptime name: []const u8) bool {
    return Record(@TypeOf(record)).include(record, name, .{ .emit_null_optional_fields = false });
}
/// The selected field_name borrows retained JSON; typed tree nodes use a.
pub fn fieldValue(a: std.mem.Allocator, record: anytype, comptime name: []const u8) !?Value {
    if (!fieldPresent(record, name)) return null;
    if (Record(@TypeOf(record)).retainedValue(record, name)) return get(record.jsonFields, name) orelse .null;
    return try toValue(a, @field(record, name), .{ .emit_null_optional_fields = false });
}
pub fn retainField(a: std.mem.Allocator, record: anytype, comptime name: []const u8, value: Value) !void {
    const T = @typeInfo(@TypeOf(record)).pointer.child;
    // Preserve the implicit presence of a freshly constructed native record's
    // other fields before introducing wire metadata or resetting this field.
    if (record.jsonFields == .null) record.jsonFields = try Record(T).jsonToValue(record.*, a, .{ .emit_null_optional_fields = false });
    inline for (@typeInfo(T).@"struct".field_names, @typeInfo(T).@"struct".field_types, @typeInfo(T).@"struct".field_attrs) |field_name, Field, attrs| if (comptime std.mem.eql(u8, name, field_name)) {
        // Required native fields keep the fallback initialized by the caller.
        // Optional/defaulted fields continue to use their declared default.
        if (comptime attrs.defaultValue(Field)) |default| @field(record, name) = default;
    };
    try put(a, &record.jsonFields, name, try clone(a, value));
    const mask = fieldMask(T, name);
    record.jsonRetained |= mask;
    record.jsonAssigned &= ~mask;
    record.jsonOmitted &= ~mask;
}
