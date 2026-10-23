const std = @import("std");
const j = @import("../json/value.zig");
const Value = std.json.Value;
const schemas = @import("type.zig");
const references = @import("references.zig");
pub const ReferenceCodecs = references.Codecs;
pub const ReferenceUrl = references.Url;
pub const ReferenceUrlInput = references.UrlInput;
pub const ReferenceFragment = references.DecodedFragment;
pub const Issue = struct {
    path: []const u8,
    keyword: []const u8,
    message: []const u8,
    required: ?struct { instancePath: []const u8, property: []const u8 } = null,
};
pub const RegExpMode = enum { ecmascript, ecmascript_unicode };
pub const Capabilities = struct {
    context: ?*anyopaque = null,
    pattern: ?*const fn (?*anyopaque, []const u8, []const u8, RegExpMode) anyerror!bool = null,
    format: ?*const fn (?*anyopaque, []const u8, []const u8) anyerror!bool = null,
    /// Lookup in an explicitly supplied schema context. Null means absent;
    /// URLs and local fragments are resolved by the scoped resolver below.
    reference: ?*const fn (?*anyopaque, []const u8) anyerror!Value = null,
    /// Already-loaded schema documents; no resolver path fetches remote URLs.
    referenceDocuments: Value = .null,
    referenceCodecs: ReferenceCodecs = .{},
    number: ?*const fn (?*anyopaque, []const u8) anyerror!?f64 = null,
};
pub const Evaluated = struct {
    keys: std.StringHashMapUnmanaged(void) = .{},
    indices: std.AutoHashMapUnmanaged(usize, void) = .{},
    pub fn deinit(self: *@This(), a: std.mem.Allocator) void {
        self.keys.deinit(a);
        self.indices.deinit(a);
        self.* = .{};
    }
    fn merge(self: *@This(), a: std.mem.Allocator, other: @This()) !void {
        var keys = other.keys.keyIterator();
        while (keys.next()) |key| try self.keys.put(a, key.*, {});
        var indices = other.indices.keyIterator();
        while (indices.next()) |index| try self.indices.put(a, index.*, {});
    }
};
pub const Result = struct {
    value: Value,
    issues: []const Issue,
    evaluated: Evaluated = .{},
    pub fn valid(self: @This()) bool {
        return self.issues.len == 0;
    }
};
fn checkPattern(a: std.mem.Allocator, caps: Capabilities, pattern: []const u8, value: []const u8, mode: RegExpMode) !bool {
    if (caps.pattern) |check| return check(caps.context, pattern, value, mode);
    return @import("../regex/regexp.zig").matches(a, null, pattern, value, .{ .unicode = mode == .ecmascript_unicode });
}
fn builtinFormat(name: []const u8) bool {
    for ([_][]const u8{ "date-time", "date", "duration", "email", "hostname", "idn-email", "idn-hostname", "ipv4", "ipv6", "iri-reference", "iri", "json-pointer-uri-fragment", "json-pointer", "regex", "relative-json-pointer", "time", "uri-reference", "uri-template", "uri", "url", "uuid" }) |known| if (std.mem.eql(u8, name, known)) return true;
    return false;
}
fn standardFormat(a: std.mem.Allocator, name: []const u8, value: []const u8) !?bool {
    if (try @import("formats.zig").check(a, name, value)) |valid| return valid;
    if (std.mem.eql(u8, name, "ipv4")) {
        _ = std.Io.net.Ip4Address.parse(value, 0) catch return false;
        return true;
    }

    // Ip6Address's ::ffff: special case requires dotted IPv4 and rejects
    // valid hexadecimal suffixes. Other embedded-IPv4 forms and scoped names
    // also differ from the pinned predicate, so it is not an exact substitute.
    return null;
}
pub fn validate(a: std.mem.Allocator, schema: Value, value: Value, coerce: bool, capabilities: Capabilities) !Result {
    var scope = references.Stack.init(a, &schema, .{ .context = capabilities.context, .documents = capabilities.referenceDocuments, .lookup = capabilities.reference, .codecs = capabilities.referenceCodecs });
    defer scope.deinit();
    return validateAt(a, &schema, value, "", coerce, capabilities, &scope);
}
fn validateAt(a: std.mem.Allocator, schema: references.Node, value: Value, path: []const u8, coerce: bool, capabilities: Capabilities, scope: *references.Stack) anyerror!Result {
    var issues: std.ArrayList(Issue) = .empty;
    var evaluated: Evaluated = .{};
    var out = try j.clone(a, value);
    if (coerce) out = try coerceSchema(a, schema.*, out, capabilities);
    try visit(a, schema, &out, path, capabilities, &issues, &evaluated, scope);
    return .{ .value = out, .issues = try issues.toOwnedSlice(a), .evaluated = evaluated };
}
fn issue(a: std.mem.Allocator, issues: *std.ArrayList(Issue), path: []const u8, keyword: []const u8, message: []const u8) !void {
    try issues.append(a, .{ .path = path, .keyword = keyword, .message = message });
}
fn appendIssues(a: std.mem.Allocator, issues: *std.ArrayList(Issue), additional: []const Issue) !void {
    try issues.appendSlice(a, additional);
}
fn writeValidationPath(writer: *std.Io.Writer, entry: Issue) !void {
    const instance_path = if (entry.required) |required| required.instancePath else entry.path;
    const path = if (std.mem.startsWith(u8, instance_path, "/")) instance_path[1..] else instance_path;
    for (path) |byte| try writer.writeByte(if (byte == '/') '.' else byte);
    if (entry.required) |required| if (required.property.len > 0) {
        if (path.len > 0) try writer.writeByte('.');
        try writer.writeAll(required.property);
        return;
    };
    if (path.len == 0) try writer.writeAll("root");
}
fn equal(a: std.mem.Allocator, left: Value, right: Value) !bool {
    if (j.number(left)) |l| if (j.number(right)) |r| return l == r;
    if (std.meta.activeTag(left) != std.meta.activeTag(right)) return false;
    return switch (left) {
        .null => true,
        .bool => |v| v == right.bool,
        .string => |v| std.mem.eql(u8, v, right.string),
        .array => |v| blk: {
            if (v.items.len != right.array.items.len) break :blk false;
            for (v.items, right.array.items) |l, r| if (!try equal(a, l, r)) break :blk false;
            break :blk true;
        },
        .object => |v| blk: {
            if (v.count() != right.object.count()) break :blk false;
            var it = v.iterator();
            while (it.next()) |e| {
                const r = right.object.get(e.key_ptr.*) orelse break :blk false;
                if (!try equal(a, e.value_ptr.*, r)) break :blk false;
            }
            break :blk true;
        },
        else => false,
    };
}
fn matches(value: Value, kind: []const u8) bool {
    if (std.mem.eql(u8, kind, "null")) return value == .null;
    if (std.mem.eql(u8, kind, "boolean")) return value == .bool;
    if (std.mem.eql(u8, kind, "string")) return value == .string;
    if (std.mem.eql(u8, kind, "array")) return value == .array;
    if (std.mem.eql(u8, kind, "object")) return value == .object;
    if (std.mem.eql(u8, kind, "number")) {
        const n = j.number(value) orelse return false;
        return std.math.isFinite(n);
    }
    if (std.mem.eql(u8, kind, "integer")) {
        const n = j.number(value) orelse return false;
        return std.math.isFinite(n) and @floor(n) == n;
    }
    return false;
}
fn schemaMatches(value: Value, kind: []const u8) bool {
    for ([_][]const u8{ "null", "boolean", "string", "array", "object", "number", "integer" }) |name| if (std.mem.eql(u8, kind, name)) return matches(value, kind);
    for ([_][]const u8{ "bigint", "constructor", "function", "symbol", "undefined", "void" }) |name| if (std.mem.eql(u8, kind, name)) return false;
    // The pinned schema engine treats other extension type names as unguarded.
    return true;
}
fn multipleOf(number: f64, divisor: f64) bool {
    if (!std.math.isFinite(number)) return true;
    if (divisor == 0) return false;
    const reciprocal = 1 / divisor;
    if (@trunc(number) == number and std.math.isFinite(reciprocal) and @rem(reciprocal, 1) == 0) return true;
    const remainder = @rem(number, divisor);
    return @min(@abs(remainder), @min(@abs(remainder - divisor), @abs(remainder + divisor))) < 1e-10;
}
fn numericString(a: std.mem.Allocator, input: []const u8) !?f64 {
    return @import("../utils/numbers.zig").number(a, null, input);
}
fn numberText(a: std.mem.Allocator, number: f64) ![]const u8 {
    return @import("../utils/numbers.zig").toString(a, number);
}
fn coerceTypeMatches(value: Value, kind: []const u8) bool {
    if (std.mem.eql(u8, kind, "number")) return j.number(value) != null;
    return matches(value, kind);
}
fn isGraphemeModifier(point: u21) bool {
    return (point >= 0x0300 and point <= 0x036f) or
        (point >= 0x1ab0 and point <= 0x1aff) or
        (point >= 0x1dc0 and point <= 0x1dff) or
        (point >= 0xfe20 and point <= 0xfe2f) or
        (point >= 0xfe00 and point <= 0xfe0f);
}
fn consumeGraphemeModifiers(iterator: *std.unicode.Wtf8Iterator) void {
    while (true) {
        var lookahead = iterator.*;
        const point = lookahead.nextCodepoint() orelse return;
        if (!isGraphemeModifier(point)) return;
        iterator.* = lookahead;
    }
}
fn isRegionalIndicator(point: u21) bool {
    return point >= 0x1f1e6 and point <= 0x1f1ff;
}
fn typeboxGraphemeCount(text: []const u8) !usize {
    var iterator = (try std.unicode.Wtf8View.init(text)).iterator();
    var count: usize = 0;
    while (iterator.nextCodepoint()) |start| {
        count += 1;
        consumeGraphemeModifiers(&iterator);
        while (true) {
            var lookahead = iterator;
            if (lookahead.nextCodepoint() != 0x200d) break;
            _ = lookahead.nextCodepoint() orelse break;
            iterator = lookahead;
            consumeGraphemeModifiers(&iterator);
        }
        if (isRegionalIndicator(start)) {
            var lookahead = iterator;
            if (lookahead.nextCodepoint()) |next| if (isRegionalIndicator(next)) {
                iterator = lookahead;
            };
        }
    }
    return count;
}
fn coercePrimitive(a: std.mem.Allocator, value: Value, kind: []const u8) !Value {
    if (matches(value, kind)) return value;
    if (std.mem.eql(u8, kind, "number") or std.mem.eql(u8, kind, "integer")) {
        const n: ?f64 = switch (value) {
            .null => 0,
            .bool => |v| if (v) 1 else 0,
            .string => |s| blk: {
                const normalized = try @import("../utils/unicode.zig").trimWhitespace(a, s);
                defer a.free(normalized);
                break :blk if (normalized.len > 0) try numericString(a, normalized) else null;
            },
            else => null,
        };
        if (n) |v| if (std.math.isFinite(v) and (!std.mem.eql(u8, kind, "integer") or @floor(v) == v)) return j.num(v);
    } else if (std.mem.eql(u8, kind, "boolean")) {
        if (value == .null) return .{ .bool = false };
        if (value == .string) {
            if (std.mem.eql(u8, value.string, "true")) return .{ .bool = true };
            if (std.mem.eql(u8, value.string, "false")) return .{ .bool = false };
        }
        if (j.number(value)) |n| {
            if (n == 0) return .{ .bool = false };
            if (n == 1) return .{ .bool = true };
        }
    } else if (std.mem.eql(u8, kind, "string")) {
        if (value == .null) return j.str("");
        if (value == .bool) return j.str(if (value.bool) "true" else "false");
        if (j.number(value)) |number| return j.str(try numberText(a, number));
    } else if (std.mem.eql(u8, kind, "null")) {
        if ((value == .string and value.string.len == 0) or (value == .bool and !value.bool)) return .null;
        if (j.number(value)) |n| if (n == 0) return .null;
    }
    return value;
}
fn branchValid(a: std.mem.Allocator, schema: Value, value: Value, caps: Capabilities) anyerror!bool {
    var checked = validate(a, schema, value, false, caps) catch |err| switch (err) {
        error.OutOfMemory, error.Canceled, error.SchemaReferenceUrlCodecUnavailable, error.SchemaReferenceFragmentCodecUnavailable, error.SchemaReferencePointerCodecUnavailable => return err,
        else => return false,
    };
    defer checked.evaluated.deinit(a);
    return checked.valid();
}
fn coerceUnion(a: std.mem.Allocator, branches: Value, value: Value, caps: Capabilities) anyerror!Value {
    if (branches != .array) return value;
    for (branches.array.items) |branch| if (try branchValid(a, branch, value, caps)) return value;
    for (branches.array.items) |branch| {
        const candidate = try coerceSchema(a, branch, try j.clone(a, value), caps);
        if (try branchValid(a, branch, candidate, caps)) return candidate;
    }
    return value;
}
fn coerceSchema(a: std.mem.Allocator, schema: Value, value: Value, caps: Capabilities) anyerror!Value {
    if (schema != .object) return value;
    var next = value;
    if (j.get(schema, "allOf")) |branches| if (branches == .array) {
        for (branches.array.items) |branch| next = try coerceSchema(a, branch, next, caps);
    };
    if (j.get(schema, "anyOf")) |branches| next = try coerceUnion(a, branches, next, caps);
    if (j.get(schema, "oneOf")) |branches| next = try coerceUnion(a, branches, next, caps);
    const kinds = j.get(schema, "type") orelse return next;
    const type_names: []const Value = if (kinds == .array) kinds.array.items else if (kinds == .string) &.{kinds} else &.{};
    var already_matches = false;
    if (type_names.len > 1) for (type_names) |kind| if (kind == .string and coerceTypeMatches(next, kind.string)) {
        already_matches = true;
        break;
    };
    if (!already_matches) for (type_names) |kind| {
        if (kind != .string) continue;
        const candidate = try coercePrimitive(a, next, kind.string);
        if (!try equal(a, candidate, next)) {
            next = candidate;
            break;
        }
    };
    var object_type = false;
    var array_type = false;
    for (type_names) |kind| if (kind == .string) {
        object_type = object_type or std.mem.eql(u8, kind.string, "object");
        array_type = array_type or std.mem.eql(u8, kind.string, "array");
    };
    if (object_type and next == .object) {
        const properties = j.get(schema, "properties");
        if (properties) |props| if (props == .object) {
            var it = props.object.iterator();
            while (it.next()) |property| {
                const current = next.object.getPtr(property.key_ptr.*) orelse continue;
                current.* = try coerceSchema(a, property.value_ptr.*, current.*, caps);
            }
        };
        if (j.get(schema, "additionalProperties")) |additional| if (additional == .object) {
            var it = next.object.iterator();
            while (it.next()) |property| {
                if (properties) |props| if (j.get(props, property.key_ptr.*) != null) continue;
                property.value_ptr.* = try coerceSchema(a, additional, property.value_ptr.*, caps);
            }
        };
    }
    if (array_type and next == .array) if (j.get(schema, "items")) |items| {
        for (next.array.items, 0..) |*item, index| {
            const child = if (items == .array) (if (index < items.array.items.len) items.array.items[index] else continue) else if (items == .object) items else continue;
            item.* = try coerceSchema(a, child, item.*, caps);
        }
    };
    return next;
}
fn nativeNumber(a: std.mem.Allocator, value: Value, caps: Capabilities) !?f64 {
    return switch (value) {
        .null => 0,
        .bool => |boolean| if (boolean) @as(f64, 1) else 0,
        .integer, .float, .number_string => if (j.number(value)) |number| (if (std.math.isFinite(number)) number else null) else null,
        .string => |text| blk: {
            if (caps.number) |convert| break :blk try convert(caps.context, text);
            const normalized = try @import("../utils/unicode.zig").trimWhitespace(a, text);
            defer a.free(normalized);
            if (normalized.len == 0) break :blk 0;
            if (try numericString(a, normalized)) |number| break :blk if (std.math.isFinite(number)) number else null;
            if (std.ascii.eqlIgnoreCase(text, "true")) break :blk 1;
            if (std.ascii.eqlIgnoreCase(text, "false")) break :blk 0;
            var regex = try @import("../regex/regexp.zig").Regex.init(a, null,
                \\^(-?(?:0|[1-9]\d*))n$
            , .{});
            defer regex.deinit();
            var match = (try regex.exec(a, text, 0)) orelse break :blk null;
            defer match.deinit();
            if (match.captures[0].?.end != match.input.len) return error.InvalidBigIntLiteral;
            const digits = (try match.text(a, 1)).?;
            defer a.free(digits);
            const integer = std.fmt.parseInt(i64, digits, 10) catch break :blk null;
            if (integer < -9007199254740991 or integer > 9007199254740991) break :blk null;
            break :blk @as(f64, @floatFromInt(integer));
        },
        else => null,
    };
}
fn nativeBoolean(value: Value) Value {
    if (value == .null) return .{ .bool = false };
    if (value == .string) {
        if (std.ascii.eqlIgnoreCase(value.string, "true") or std.mem.eql(u8, value.string, "1")) return .{ .bool = true };
        if (std.ascii.eqlIgnoreCase(value.string, "false") or std.mem.eql(u8, value.string, "0")) return .{ .bool = false };
    }
    if (j.number(value)) |number| {
        if (number == 0) return .{ .bool = false };
        if (number == 1) return .{ .bool = true };
    }
    return value;
}
fn convertNative(a: std.mem.Allocator, schema: schemas.TSchema, value: Value, caps: Capabilities) anyerror!Value {
    return switch (schema.kind) {
        .unsafe => value,
        .number, .integer => if (try nativeNumber(a, value, caps)) |number| j.num(if (schema.kind == .integer) @trunc(number) else number) else value,
        .boolean => nativeBoolean(value),
        .string => blk: {
            if (value == .null) break :blk j.str("null");
            if (j.number(value)) |number| if (!std.math.isFinite(number)) break :blk value;
            break :blk try coercePrimitive(a, value, "string");
        },
        .null => blk: {
            if (value == .string and (std.ascii.eqlIgnoreCase(value.string, "null") or std.ascii.eqlIgnoreCase(value.string, "undefined") or std.mem.eql(u8, value.string, "0"))) break :blk .null;
            break :blk try coercePrimitive(a, value, "null");
        },
        .array => blk: {
            var result = j.array(a);
            if (value == .array) {
                for (value.array.items) |item| try result.array.append(if (schema.items) |child| try convertNative(a, child.*, item, caps) else item);
            } else try result.array.append(if (schema.items) |child| try convertNative(a, child.*, value, caps) else value);
            break :blk result;
        },
        .tuple => blk: {
            if (value != .array) break :blk value;
            for (value.array.items[0..@min(value.array.items.len, schema.variants.len)], schema.variants[0..@min(value.array.items.len, schema.variants.len)]) |*item, child| item.* = try convertNative(a, child, item.*, caps);
            break :blk value;
        },
        .object => blk: {
            if (value != .object) break :blk value;
            var result = value;
            var literal_keys = true;
            var keys = result.object.iterator();
            while (keys.next()) |entry| {
                for (entry.key_ptr.*) |byte| if (!std.ascii.isAlphanumeric(byte) and byte != '_' and byte != '-') {
                    literal_keys = false;
                    break;
                };
                if (!literal_keys) break;
            }
            for (schema.properties) |property| {
                var literal_key = literal_keys;
                for (property.name) |byte| if (!std.ascii.isAlphanumeric(byte) and byte != '_' and byte != '-') {
                    literal_key = false;
                    break;
                };
                if (literal_key) {
                    if (result.object.getPtr(property.name)) |item| item.* = try convertNative(a, property.schema, item.*, caps);
                } else {
                    const pattern = try std.fmt.allocPrint(a, "^{s}$", .{property.name});
                    var it = result.object.iterator();
                    while (it.next()) |item| {
                        if (try checkPattern(a, caps, pattern, item.key_ptr.*, .ecmascript)) item.value_ptr.* = try convertNative(a, property.schema, item.value_ptr.*, caps);
                    }
                }
            }
            break :blk result;
        },
        .record => blk: {
            if (value != .object) break :blk value;
            var result = value;
            for (schema.properties) |property| {
                const entries = try j.objectEntries(a, result.object);
                defer a.free(entries);
                for (entries) |entry| if (try checkPattern(a, caps, property.name, entry.key, .ecmascript)) {
                    const item = result.object.getPtr(entry.key).?;
                    item.* = try convertNative(a, property.schema, item.*, caps);
                };
            }
            break :blk result;
        },
        .union_ => blk: {
            for (schema.variants) |variant| if (try branchValid(a, variant.value, value, caps)) break :blk value;
            const candidates = try a.alloc(Value, schema.variants.len);
            for (schema.variants, 0..) |variant, index| candidates[index] = try convertNative(a, variant, try j.clone(a, value), caps);
            for (candidates) |candidate| if (try branchValid(a, schema.value, candidate, caps)) break :blk candidate;
            break :blk value;
        },
        .literal => blk: {
            const constant = j.get(schema.value, "const") orelse return error.InvalidSchema;
            var child = schema;
            child.kind = switch (constant) {
                .string => .string,
                .bool => .boolean,
                .integer, .float => .number,
                else => return error.InvalidSchema,
            };
            const candidate = try convertNative(a, child, value, caps);
            break :blk if (try equal(a, constant, candidate)) candidate else value;
        },
    };
}
fn visitChild(a: std.mem.Allocator, schema: references.Node, value: *Value, path: []const u8, caps: Capabilities, issues: *std.ArrayList(Issue), scope: *references.Stack) !bool {
    var child: Evaluated = .{};
    defer child.deinit(a);
    const previous = issues.items.len;
    try visit(a, schema, value, path, caps, issues, &child, scope);
    return issues.items.len == previous;
}
fn visit(a: std.mem.Allocator, schema_node: references.Node, value: *Value, path: []const u8, caps: Capabilities, issues: *std.ArrayList(Issue), evaluated: *Evaluated, scope: *references.Stack) anyerror!void {
    const schema = schema_node.*;
    try scope.push(schema_node);
    defer scope.pop(schema_node);
    if (schema == .bool) {
        if (!schema.bool) try issue(a, issues, path, "boolean", "schema is false");
        return;
    }
    if (schema != .object) return error.InvalidSchema;
    if (j.get(schema, "type")) |kinds| {
        var valid_type = false;
        if (kinds == .string) {
            valid_type = schemaMatches(value.*, kinds.string);
        } else if (kinds == .array) {
            for (kinds.array.items) |kind| if (kind == .string and schemaMatches(value.*, kind.string)) {
                valid_type = true;
                break;
            };
        } else return error.InvalidSchema;
        if (!valid_type) {
            const message = if (kinds == .string) try std.fmt.allocPrint(a, "must be {s}", .{kinds.string}) else blk: {
                var names: std.ArrayList([]const u8) = .empty;
                for (kinds.array.items) |kind| {
                    if (kind != .string) return error.InvalidSchema;
                    try names.append(a, kind.string);
                }
                break :blk try std.fmt.allocPrint(a, "must be either {s}", .{try std.mem.join(a, " or ", names.items)});
            };
            try issue(a, issues, path, "type", message);
        }
    }
    switch (value.*) {
        .object => |*object| {
            const value_entries = try j.objectEntries(a, object.*);
            defer a.free(value_entries);
            const properties = j.get(schema, "properties");
            const required = j.get(schema, "required");
            if (required) |keys| {
                if (keys != .array) return error.InvalidSchema;
                var missing: std.ArrayList([]const u8) = .empty;
                for (keys.array.items) |key| {
                    if (key != .string) return error.InvalidSchema;
                    if (!object.contains(key.string)) try missing.append(a, key.string);
                }
                if (missing.items.len > 0) try issues.append(a, .{
                    .path = try std.fmt.allocPrint(a, "{s}/{s}", .{ path, missing.items[0] }),
                    .keyword = "required",
                    .message = try std.fmt.allocPrint(a, "must have required properties {s}", .{try std.mem.join(a, ", ", missing.items)}),
                    .required = .{ .instancePath = path, .property = missing.items[0] },
                });
            }
            if (references.child(schema_node, "additionalProperties")) |additional| {
                var invalid = false;
                for (value_entries) |entry| {
                    var declared = if (properties) |props| j.get(props, entry.key) != null else false;
                    if (j.get(schema, "patternProperties")) |patterns| {
                        if (patterns != .object) return error.InvalidSchema;
                        if (patterns.object.count() > 0) {
                            var patterns_it = patterns.object.iterator();
                            while (patterns_it.next()) |pattern| if (try checkPattern(a, caps, pattern.key_ptr.*, entry.key, .ecmascript_unicode)) {
                                declared = true;
                                break;
                            };
                        }
                    }
                    if (declared) continue;
                    const subpath = try std.fmt.allocPrint(a, "{s}/{s}", .{ path, entry.key });
                    if (try visitChild(a, additional, object.getPtr(entry.key).?, subpath, caps, issues, scope)) try evaluated.keys.put(a, entry.key, {}) else invalid = true;
                }
                if (invalid) try issue(a, issues, path, "additionalProperties", "must not have additional properties");
            }
            for ([_][]const u8{ "dependencies", "dependentRequired" }) |key| if (j.get(schema, key)) |dependencies| {
                if (dependencies != .object) return error.InvalidSchema;
                const dependency_entries = try j.objectEntries(a, dependencies.object);
                defer a.free(dependency_entries);
                for (dependency_entries) |dependency| {
                    if (!object.contains(dependency.key)) continue;
                    if (dependency.node.* == .array) {
                        var names: std.ArrayList([]const u8) = .empty;
                        for (dependency.node.array.items) |name| {
                            if (name != .string) return error.InvalidSchema;
                            try names.append(a, name.string);
                        }
                        for (names.items) |name| {
                            if (!object.contains(name)) {
                                try issue(a, issues, path, key, try std.fmt.allocPrint(a, "must have properties {s} when property {s} is present", .{ try std.mem.join(a, ", ", names.items), dependency.key }));
                                if (std.mem.eql(u8, key, "dependencies")) break;
                            }
                        }
                    } else if (std.mem.eql(u8, key, "dependencies")) try visit(a, dependency.node, value, path, caps, issues, evaluated, scope) else return error.InvalidSchema;
                }
            };
            if (j.get(schema, "dependentSchemas")) |dependencies| {
                if (dependencies != .object) return error.InvalidSchema;
                const dependency_entries = try j.objectEntries(a, dependencies.object);
                defer a.free(dependency_entries);
                for (dependency_entries) |dependency| if (object.contains(dependency.key)) try visit(a, dependency.node, value, path, caps, issues, evaluated, scope);
            }
            if (j.get(schema, "patternProperties")) |patterns| {
                if (patterns != .object) return error.InvalidSchema;
                if (patterns.object.count() > 0) {
                    const pattern_entries = try j.objectEntries(a, patterns.object);
                    defer a.free(pattern_entries);
                    for (pattern_entries) |pattern| {
                        for (value_entries) |entry| if (try checkPattern(a, caps, pattern.key, entry.key, .ecmascript_unicode)) {
                            if (try visitChild(a, pattern.node, object.getPtr(entry.key).?, try std.fmt.allocPrint(a, "{s}/{s}", .{ path, entry.key }), caps, issues, scope)) try evaluated.keys.put(a, entry.key, {});
                        };
                    }
                }
            }
            if (properties) |props| {
                if (props != .object) return error.InvalidSchema;
                const property_entries = try j.objectEntries(a, props.object);
                defer a.free(property_entries);
                for (property_entries) |property| {
                    const current = object.getPtr(property.key) orelse continue;
                    if (try visitChild(a, property.node, current, try std.fmt.allocPrint(a, "{s}/{s}", .{ path, property.key }), caps, issues, scope)) try evaluated.keys.put(a, property.key, {});
                }
            }
            if (references.child(schema_node, "propertyNames")) |names| {
                var invalid_names: std.ArrayList([]const u8) = .empty;
                for (value_entries) |entry| {
                    var name = j.str(entry.key);
                    if (!try visitChild(a, names, &name, try std.fmt.allocPrint(a, "{s}/{s}", .{ path, entry.key }), caps, issues, scope)) try invalid_names.append(a, entry.key);
                }
                if (invalid_names.items.len > 0) try issue(a, issues, path, "propertyNames", try std.fmt.allocPrint(a, "property names {s} are invalid", .{try std.mem.join(a, ", ", invalid_names.items)}));
            }
            if (j.get(schema, "minProperties")) |n| if (@as(f64, @floatFromInt(object.count())) < (j.number(n) orelse return error.InvalidSchema)) try issue(a, issues, path, "minProperties", try std.fmt.allocPrint(a, "must not have fewer than {d} properties", .{j.number(n).?}));
            if (j.get(schema, "maxProperties")) |n| if (@as(f64, @floatFromInt(object.count())) > (j.number(n) orelse return error.InvalidSchema)) try issue(a, issues, path, "maxProperties", try std.fmt.allocPrint(a, "must not have more than {d} properties", .{j.number(n).?}));
        },
        .array => |*array| {
            if (j.get(schema, "items")) |items| if (items == .array) if (references.child(schema_node, "additionalItems")) |additional| {
                for (array.items, 0..) |*item, index| {
                    if (index < items.array.items.len) continue;
                    if (!try visitChild(a, additional, item, try std.fmt.allocPrint(a, "{s}/{d}", .{ path, index }), caps, issues, scope)) break;
                    try evaluated.indices.put(a, index, {});
                }
            };
            var contains_count: ?f64 = null;
            if (references.child(schema_node, "contains")) |contains| {
                var count: usize = 0;
                for (array.items, 0..) |item, index| if ((try validateAt(a, contains, item, path, false, caps, scope)).valid()) {
                    count += 1;
                    try evaluated.indices.put(a, index, {});
                };
                contains_count = @floatFromInt(count);
                const minimum = j.number(j.get(schema, "minContains") orelse j.int(1)) orelse return error.InvalidSchema;
                if (minimum != 0 and count == 0) try issue(a, issues, path, "contains", "must contain at least 1 valid item");
            }
            if (j.get(schema, "items")) |items| for (array.items, 0..) |*item, i| {
                if (items != .array) {
                    if (j.get(schema, "prefixItems")) |prefix| if (prefix == .array and i < prefix.array.items.len) continue;
                }
                const items_node = references.child(schema_node, "items").?;
                const child = if (items == .array) (if (i < items.array.items.len) &items_node.array.items[i] else continue) else items_node;
                if (try visitChild(a, child, item, try std.fmt.allocPrint(a, "{s}/{d}", .{ path, i }), caps, issues, scope)) try evaluated.indices.put(a, i, {});
            };
            if (contains_count) |count| if (j.get(schema, "maxContains")) |maximum| if (count > (j.number(maximum) orelse return error.InvalidSchema)) try issue(a, issues, path, "contains", "must contain at least 1 valid item");
            if (j.get(schema, "maxItems")) |n| if (@as(f64, @floatFromInt(array.items.len)) > (j.number(n) orelse return error.InvalidSchema)) try issue(a, issues, path, "maxItems", try std.fmt.allocPrint(a, "must not have more than {d} items", .{j.number(n).?}));
            if (contains_count) |count| if (j.get(schema, "minContains")) |minimum| if (count < (j.number(minimum) orelse return error.InvalidSchema)) try issue(a, issues, path, "contains", "must contain at least 1 valid item");
            if (j.get(schema, "minItems")) |n| if (@as(f64, @floatFromInt(array.items.len)) < (j.number(n) orelse return error.InvalidSchema)) try issue(a, issues, path, "minItems", try std.fmt.allocPrint(a, "must not have fewer than {d} items", .{j.number(n).?}));
            if (j.get(schema, "prefixItems")) |prefix| {
                if (prefix != .array) return error.InvalidSchema;
                for (array.items[0..@min(array.items.len, prefix.array.items.len)], prefix.array.items[0..@min(array.items.len, prefix.array.items.len)], 0..) |*item, *subschema, index| if (try visitChild(a, subschema, item, try std.fmt.allocPrint(a, "{s}/{d}", .{ path, index }), caps, issues, scope)) try evaluated.indices.put(a, index, {});
            }
            if (j.get(schema, "uniqueItems")) |v| if (v == .bool and v.bool) {
                const duplicate = outer: {
                    for (array.items, 0..) |item, i| for (array.items[0..i]) |other| if (try equal(a, item, other)) break :outer true;
                    break :outer false;
                };
                if (duplicate) try issue(a, issues, path, "uniqueItems", "must not have duplicate items");
            };
        },
        .string => |string| {
            const length = try typeboxGraphemeCount(string);
            if (j.get(schema, "maxLength")) |n| if (@as(f64, @floatFromInt(length)) > (j.number(n) orelse return error.InvalidSchema)) try issue(a, issues, path, "maxLength", try std.fmt.allocPrint(a, "must not have more than {d} characters", .{j.number(n).?}));
            if (j.get(schema, "minLength")) |n| if (@as(f64, @floatFromInt(length)) < (j.number(n) orelse return error.InvalidSchema)) try issue(a, issues, path, "minLength", try std.fmt.allocPrint(a, "must not have fewer than {d} characters", .{j.number(n).?}));
            if (j.field(schema, "format")) |format| {
                if (caps.format) |check| {
                    if (!try check(caps.context, format, string)) try issue(a, issues, path, "format", try std.fmt.allocPrint(a, "must match format \"{s}\"", .{format}));
                } else if (try standardFormat(a, format, string)) |valid| {
                    if (!valid) try issue(a, issues, path, "format", try std.fmt.allocPrint(a, "must match format \"{s}\"", .{format}));
                } else if (builtinFormat(format)) return error.SchemaFormatRequiresDecision;
            }
            if (j.field(schema, "pattern")) |pattern| {
                if (!try checkPattern(a, caps, pattern, string, .ecmascript_unicode)) try issue(a, issues, path, "pattern", try std.fmt.allocPrint(a, "must match pattern \"{s}\"", .{pattern}));
            }
        },
        .integer, .float, .number_string => numeric: {
            const n = j.number(value.*) orelse return error.InvalidNumber;
            if (!std.math.isFinite(n)) break :numeric;
            for ([_][]const u8{ "exclusiveMaximum", "exclusiveMinimum", "maximum", "minimum", "multipleOf" }) |keyword| if (j.get(schema, keyword)) |bound| {
                const b = j.number(bound) orelse return error.InvalidSchema;
                const invalid = if (std.mem.eql(u8, keyword, "minimum")) n < b else if (std.mem.eql(u8, keyword, "maximum")) n > b else if (std.mem.eql(u8, keyword, "exclusiveMinimum")) n <= b else if (std.mem.eql(u8, keyword, "exclusiveMaximum")) n >= b else !multipleOf(n, b);
                if (invalid) try issue(a, issues, path, keyword, if (std.mem.eql(u8, keyword, "multipleOf")) try std.fmt.allocPrint(a, "must be multiple of {d}", .{b}) else try std.fmt.allocPrint(a, "must be {s} {d}", .{ if (std.mem.eql(u8, keyword, "minimum")) ">=" else if (std.mem.eql(u8, keyword, "maximum")) "<=" else if (std.mem.eql(u8, keyword, "exclusiveMinimum")) ">" else "<", b }));
            };
        },
        else => {},
    }
    // Pinned keyword order applies references after the primitive constraints.
    // A missing target is a false schema; a missing codec propagates explicitly.
    for ([_][]const u8{ "$ref", "$recursiveRef", "$dynamicRef" }) |keyword| if (j.field(schema, keyword)) |text| {
        const target = if (std.mem.eql(u8, keyword, "$ref")) try scope.resolveRef(text) else if (std.mem.eql(u8, keyword, "$recursiveRef")) try scope.resolveRecursiveRef(text) else try scope.resolveDynamicRef(text);
        const missing: Value = .{ .bool = false };
        const resolved = if (target) |node| (if (node.* == .object or node.* == .bool) node else &missing) else &missing;
        if (std.mem.eql(u8, keyword, "$ref")) {
            const result = try validateAt(a, resolved, value.*, path, false, caps, scope);
            defer {
                var marks = result.evaluated;
                marks.deinit(a);
            }
            if (result.valid()) try evaluated.merge(a, result.evaluated) else try appendIssues(a, issues, result.issues);
        } else try visit(a, resolved, value, path, caps, issues, evaluated, scope);
    };
    if (j.get(schema, "const")) |constant| if (!try equal(a, value.*, constant)) try issue(a, issues, path, "const", "must be equal to constant");
    if (j.get(schema, "enum")) |choices| {
        if (choices != .array) return error.InvalidSchema;
        var found = false;
        for (choices.array.items) |choice| if (try equal(a, value.*, choice)) {
            found = true;
            break;
        };
        if (!found) try issue(a, issues, path, "enum", "must be equal to one of the allowed values");
    }
    if (references.child(schema_node, "if")) |condition| {
        const is_then = (try validateAt(a, condition, value.*, path, false, caps, scope)).valid();
        if (references.child(schema_node, if (is_then) "then" else "else")) |branch| {
            const result = try validateAt(a, branch, value.*, path, false, caps, scope);
            if (!result.valid()) {
                if (!is_then) try appendIssues(a, issues, result.issues);
                try issue(a, issues, path, "if", if (is_then) "must match \"then\" schema" else "must match \"else\" schema");
            }
            if (result.valid() or !is_then) try evaluated.merge(a, result.evaluated);
        }
    }
    if (references.child(schema_node, "not")) |negated| if ((try validateAt(a, negated, value.*, path, false, caps, scope)).valid()) try issue(a, issues, path, "not", "must not be valid");
    for ([_][]const u8{ "allOf", "anyOf", "oneOf" }) |key| if (j.get(schema, key)) |branches| {
        if (branches != .array) return error.InvalidSchema;
        var matches_count: usize = 0;
        var failures: std.ArrayList(Issue) = .empty;
        var passing: Evaluated = .{};
        defer passing.deinit(a);
        for (branches.array.items) |*branch| {
            const result = try validateAt(a, branch, value.*, path, false, caps, scope);
            if (result.valid()) {
                matches_count += 1;
                try passing.merge(a, result.evaluated);
            } else try failures.appendSlice(a, result.issues);
        }
        if (std.mem.eql(u8, key, "allOf")) {
            try appendIssues(a, issues, failures.items);
            if (matches_count == branches.array.items.len) try evaluated.merge(a, passing);
        } else if (matches_count == 0 or (std.mem.eql(u8, key, "oneOf") and matches_count != 1)) {
            if (matches_count == 0) try appendIssues(a, issues, failures.items);
            try issue(a, issues, path, key, if (std.mem.eql(u8, key, "oneOf")) "must match exactly one schema in oneOf" else "must match a schema in anyOf");
        } else try evaluated.merge(a, passing);
    };
    if (value.* == .array) if (references.child(schema_node, "unevaluatedItems")) |remaining| {
        var invalid = false;
        for (value.array.items, 0..) |item, index| {
            if (evaluated.indices.contains(index)) continue;
            if ((try validateAt(a, remaining, item, path, false, caps, scope)).valid()) try evaluated.indices.put(a, index, {}) else invalid = true;
        }
        if (invalid) try issue(a, issues, path, "unevaluatedItems", "must not have unevaluated items");
    };
    if (value.* == .object) if (references.child(schema_node, "unevaluatedProperties")) |remaining| {
        var invalid = false;
        const value_entries = try j.objectEntries(a, value.object);
        defer a.free(value_entries);
        for (value_entries) |entry| {
            if (evaluated.keys.contains(entry.key)) continue;
            if ((try validateAt(a, remaining, entry.node.*, path, false, caps, scope)).valid()) try evaluated.keys.put(a, entry.key, {}) else invalid = true;
        }
        if (invalid) try issue(a, issues, path, "unevaluatedProperties", "must not have unevaluated properties");
    };
}
pub fn checkToolArguments(a: std.mem.Allocator, tool: @import("../types.zig").Tool, call: @import("../types.zig").ToolCall, caps: Capabilities) !Result {
    if (call.argumentsError) |message| if (message.len > 0) return .{ .value = try j.clone(a, call.arguments), .issues = try a.dupe(Issue, &.{.{ .path = "", .keyword = "arguments", .message = try a.dupe(u8, message) }}) };
    var arguments = try j.clone(a, call.arguments);
    try normalizeOptionalNulls(a, &arguments, tool.parameters.value, caps);
    // Pi deliberately ignores Value.Convert's returned root. Interior object
    // mutations survive; a replaced primitive or array root does not.
    _ = try convertNative(a, tool.parameters, arguments, caps);
    const coerced = try coerceSchema(a, tool.parameters.value, arguments, caps);
    if (arguments != .object and arguments != .array and !try equal(a, arguments, coerced)) {
        return .{ .value = if (try branchValid(a, tool.parameters.value, coerced, caps)) coerced else arguments, .issues = &.{} };
    }
    return validate(a, tool.parameters.value, coerced, false, caps);
}
fn normalizeOptionalNulls(a: std.mem.Allocator, value: *Value, schema: Value, caps: Capabilities) anyerror!void {
    if (value.* == .array) {
        const items = j.get(schema, "items") orelse return;
        for (value.array.items, 0..) |*item, index| {
            const child = if (items == .array) (if (index < items.array.items.len) items.array.items[index] else continue) else items;
            try normalizeOptionalNulls(a, item, child, caps);
        }
        return;
    }
    if (value.* != .object) return;
    const properties = j.get(schema, "properties") orelse return;
    if (properties != .object) return;
    const required = j.get(schema, "required");
    var it = properties.object.iterator();
    while (it.next()) |property| {
        const current = value.object.getPtr(property.key_ptr.*) orelse continue;
        const is_required = if (required) |keys| blk: {
            if (keys != .array) break :blk false;
            for (keys.array.items) |key| if (key == .string and std.mem.eql(u8, key.string, property.key_ptr.*)) break :blk true;
            break :blk false;
        } else false;
        if (current.* == .null and !is_required and j.field(property.value_ptr.*, "$ref") == null and !(try validate(a, property.value_ptr.*, .null, false, caps)).valid()) {
            _ = value.object.orderedRemove(property.key_ptr.*);
        } else try normalizeOptionalNulls(a, current, property.value_ptr.*, caps);
    }
}
pub fn checkToolCall(a: std.mem.Allocator, tools: []const @import("../types.zig").Tool, call: @import("../types.zig").ToolCall, caps: Capabilities) !Result {
    for (tools) |tool| if (std.mem.eql(u8, tool.name, call.name)) return checkToolArguments(a, tool, call, caps);
    return error.ToolNotFound;
}
pub fn validateToolArguments(a: std.mem.Allocator, tool: @import("../types.zig").Tool, call: @import("../types.zig").ToolCall, caps: Capabilities) !Value {
    return validateToolArgumentsWithError(a, tool, call, caps, null);
}
pub fn validateToolArgumentsWithError(a: std.mem.Allocator, tool: @import("../types.zig").Tool, call: @import("../types.zig").ToolCall, caps: Capabilities, error_message: ?*?[]const u8) !Value {
    if (call.argumentsError) |message| if (message.len > 0) {
        if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Invalid arguments for tool \"{s}\": {s}", .{ call.name, message });
        return error.InvalidToolArguments;
    };
    const result = try checkToolArguments(a, tool, call, caps);
    if (result.valid()) return result.value;
    if (error_message) |slot| {
        var message = std.Io.Writer.Allocating.init(a);
        defer message.deinit();
        try message.writer.print("Validation failed for tool \"{s}\":\n", .{call.name});
        for (result.issues, 0..) |entry, index| {
            if (index > 0) try message.writer.writeByte('\n');
            try message.writer.writeAll("  - ");
            try writeValidationPath(&message.writer, entry);
            try message.writer.print(": {s}", .{entry.message});
        }
        try message.writer.writeAll("\n\nReceived arguments:\n");
        try message.writer.writeAll(try j.stringify(a, call.arguments, .{ .whitespace = .indent_2 }));
        slot.* = try message.toOwnedSlice();
    }
    return error.ToolArgumentsValidationFailed;
}
pub fn validateToolCall(a: std.mem.Allocator, tools: []const @import("../types.zig").Tool, call: @import("../types.zig").ToolCall, caps: Capabilities) !Value {
    return validateToolCallWithError(a, tools, call, caps, null);
}
pub fn validateToolCallWithError(a: std.mem.Allocator, tools: []const @import("../types.zig").Tool, call: @import("../types.zig").ToolCall, caps: Capabilities, error_message: ?*?[]const u8) !Value {
    for (tools) |tool| if (std.mem.eql(u8, tool.name, call.name)) return validateToolArgumentsWithError(a, tool, call, caps, error_message);
    if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Tool \"{s}\" not found", .{call.name});
    return error.ToolNotFound;
}
