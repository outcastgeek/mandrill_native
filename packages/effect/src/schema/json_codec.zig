//! Primitive JSON codecs. Structural codecs compose these without changing
//! the distinction between decoded application values and encoded JSON data.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const issue = @import("schema_issue.zig");
const PureCodec = @import("codec.zig").PureCodec;
const Ast = @import("schema_ast.zig");
fn shape(comptime child: anytype) ?*const Ast.Node {
    const C = @TypeOf(child);
    if (@hasDecl(C, "encodedNode")) return C.encodedNode();
    if (@hasDecl(C, "encoded_node")) return &C.encoded_node;
    if (@hasField(C, "encoded_shape")) return child.encoded_shape;
    return null;
}
fn checkedShape(comptime child: anytype, comptime identifier: []const u8) ?*const Ast.Node {
    const base = shape(child) orelse return null;
    return &struct {
        const node: Ast.Node = .{ .constrained = .{ .base = base, .rules = &.{.{ .check = identifier }} } };
    }.node;
}

/// Pure JSON codec for Zig `bool`.
pub const boolean = scalarWriter(PureCodec(bool, std.json.Value).init(struct {
    fn decode(value: *const std.json.Value) outcomes.Result(bool, issue.Issue) {
        return switch (value.*) {
            .bool => |result| .{ .ok = result },
            else => .{ .err = issue.Issue.pure(.decode, .{ .expected = .boolean }) },
        };
    }
}.decode, struct {
    fn encode(value: *const bool) outcomes.Result(std.json.Value, issue.Issue) {
        return .{ .ok = .{ .bool = value.* } };
    }
}.encode));

/// Pure JSON codec for Zig `i64`.
pub const signed_integer = scalarWriter(PureCodec(i64, std.json.Value).init(struct {
    fn decode(value: *const std.json.Value) outcomes.Result(i64, issue.Issue) {
        return switch (value.*) {
            .integer => |result| .{ .ok = result },
            .number_string => |text| blk: {
                const parsed = std.fmt.parseInt(i64, text, 10) catch break :blk .{ .err = issue.Issue.pure(.decode, .invalid_number) };
                break :blk .{ .ok = parsed };
            },
            else => .{ .err = issue.Issue.pure(.decode, .{ .expected = .integer }) },
        };
    }
}.decode, struct {
    fn encode(value: *const i64) outcomes.Result(std.json.Value, issue.Issue) {
        return .{ .ok = .{ .integer = value.* } };
    }
}.encode));

/// Pure JSON codec for finite Zig `f64` values.
pub const finite_number = scalarWriter(PureCodec(f64, std.json.Value).init(struct {
    fn decode(value: *const std.json.Value) outcomes.Result(f64, issue.Issue) {
        const decoded: f64 = switch (value.*) {
            .float => |number| number,
            .integer => |number| @floatFromInt(number),
            .number_string => |text| std.fmt.parseFloat(f64, text) catch return .{ .err = issue.Issue.pure(.decode, .invalid_number) },
            else => return .{ .err = issue.Issue.pure(.decode, .{ .expected = .finite_number }) },
        };
        if (!std.math.isFinite(decoded)) return .{ .err = issue.Issue.pure(.decode, .invalid_number) };
        return .{ .ok = decoded };
    }
}.decode, struct {
    fn encode(value: *const f64) outcomes.Result(std.json.Value, issue.Issue) {
        if (!std.math.isFinite(value.*)) return .{ .err = issue.Issue.pure(.encode, .invalid_number) };
        return .{ .ok = .{ .float = value.* } };
    }
}.encode));

/// A decoded string borrows the source JSON document. `Schema.parse` retains
/// that document in Parsed(A); standalone decoders must retain their input.
pub const borrowed_string = scalarWriter(PureCodec([]const u8, std.json.Value).init(struct {
    fn decode(value: *const std.json.Value) outcomes.Result([]const u8, issue.Issue) {
        return switch (value.*) {
            .string => |text| if (std.unicode.utf8ValidateSlice(text)) .{ .ok = text } else .{ .err = issue.Issue.pure(.decode, .invalid_utf8) },
            else => .{ .err = issue.Issue.pure(.decode, .{ .expected = .string }) },
        };
    }
}.decode, struct {
    fn encode(value: *const []const u8) outcomes.Result(std.json.Value, issue.Issue) {
        if (!std.unicode.utf8ValidateSlice(value.*)) return .{ .err = issue.Issue.pure(.encode, .invalid_utf8) };
        return .{ .ok = .{ .string = value.* } };
    }
}.encode));

/// Pure JSON codec for Zig `void` represented as JSON null.
pub const null_value = scalarWriter(PureCodec(void, std.json.Value).init(struct {
    fn decode(value: *const std.json.Value) outcomes.Result(void, issue.Issue) {
        return switch (value.*) {
            .null => .{ .ok = {} },
            else => .{ .err = issue.Issue.pure(.decode, .{ .expected = .null }) },
        };
    }
}.decode, struct {
    fn encode(_: *const void) outcomes.Result(std.json.Value, issue.Issue) {
        return .{ .ok = .null };
    }
}.encode));

/// Present JSON null is distinct from an absent record field. Record-field
/// optionality belongs to SchemaBuilder and never uses this codec implicitly.
///
/// Parameters:
/// - `child: comptime anytype`: Compile-time statically inferred input checked when this operation is instantiated.
/// Returns (`PureCodec(?@TypeOf(child).Decoded, std.json.Value)`): A typed codec or schema preserving the declared decoded value, representation, and environments.
pub fn nullable(comptime child: anytype) PureCodec(?@TypeOf(child).Decoded, std.json.Value) {
    const Child = @TypeOf(child);
    if (!@hasDecl(Child, "Decoded") or !@hasDecl(Child, "Representation") or Child.Representation != std.json.Value)
        @compileError("nullable requires a JSON child codec");
    const Value = Child.Decoded;
    var result = PureCodec(?Value, std.json.Value).init(struct {
        fn decode(encoded: *const std.json.Value) outcomes.Result(?Value, issue.Issue) {
            if (encoded.* == .null) return .{ .ok = null };
            return switch (child.decodeDirect(encoded)) {
                .ok => |value| .{ .ok = value },
                .err => |problem| .{ .err = problem },
            };
        }
    }.decode, struct {
        fn encode(value: *const ?Value) outcomes.Result(std.json.Value, issue.Issue) {
            if (value.*) |actual| return child.encodeDirect(&actual);
            return .{ .ok = .null };
        }
    }.encode);
    if (comptime shape(child)) |base| result.encoded_shape = &struct {
        const node: Ast.Node = .{ .nullable = base };
    }.node;
    return result;
}

/// A refinement validates both directions. Its stable identifier is schema
/// metadata supplied by the author, rather than a runtime diagnostic string.
///
/// Parameters:
/// - `child: comptime anytype`: Compile-time statically inferred input checked when this operation is instantiated.
/// - `identifier: comptime []const u8`: Stable identifier used in a schema or protocol definition.
/// - `predicate: comptime fn (*const @TypeOf(child).Decoded) bool`: Compile-time typed callback; its signature is checked at comptime where static.
/// Returns (`PureCodec(@TypeOf(child).Decoded, std.json.Value)`): A typed codec or schema preserving the declared decoded value, representation, and environments.
pub fn refine(
    comptime child: anytype,
    comptime identifier: []const u8,
    comptime predicate: fn (*const @TypeOf(child).Decoded) bool,
) PureCodec(@TypeOf(child).Decoded, std.json.Value) {
    const C = @TypeOf(child);
    if (identifier.len == 0 or !std.unicode.utf8ValidateSlice(identifier))
        @compileError("refinement identifiers must be nonempty UTF-8");
    if (C.Representation != std.json.Value or C.DecodingEnvironment != void or C.EncodingEnvironment != void)
        @compileError("JSON refinements require a pure JSON codec");
    var result = PureCodec(C.Decoded, std.json.Value).init(struct {
        fn decode(encoded: *const std.json.Value) outcomes.Result(C.Decoded, issue.Issue) {
            const decoded = child.decodeDirect(encoded);
            return switch (decoded) {
                .ok => |value| if (predicate(&value)) .{ .ok = value } else blk: {
                    var discarded = value;
                    outcomes.discardPayload(&discarded);
                    break :blk .{ .err = issue.Issue.pure(.decode, .{ .custom = identifier }) };
                },
                .err => |problem| .{ .err = problem },
            };
        }
    }.decode, struct {
        fn encode(value: *const C.Decoded) outcomes.Result(std.json.Value, issue.Issue) {
            if (!predicate(value)) return .{ .err = issue.Issue.pure(.encode, .{ .custom = identifier }) };
            return child.encodeDirect(value);
        }
    }.encode);
    if (comptime @hasField(C, "writer") and child.writer != null) result.writer = struct {
        fn write(value: *const C.Decoded, writer: *@import("json_writer.zig").Writer) outcomes.Exit(void, issue.Issue) {
            if (!predicate(value)) return .{ .fail = issue.Issue.pure(.encode, .{ .custom = identifier }) };
            return child.writeJson(value, writer);
        }
    }.write;
    result.encoded_shape = checkedShape(child, identifier);
    return result;
}

/// Bidirectional value mapping keeps the representation codec intact and
/// requires explicit inverse and failure contracts at definition time.
///
/// Parameters:
/// - `child: comptime anytype`: Compile-time statically inferred input checked when this operation is instantiated.
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `forward: comptime fn (@TypeOf(child).Decoded) outcomes.Result(A, issue.Issue)`: Compile-time typed function selected at comptime; no runtime callback lookup is
///   required.
/// - `backward: comptime fn (*const A) outcomes.Result(@TypeOf(child).Decoded, issue.Issue)`: Compile-time typed function selected at comptime; no runtime callback lookup is
///   required.
/// Returns (`PureCodec(A, std.json.Value)`): A typed codec or schema preserving the declared decoded value, representation, and environments.
pub fn transform(
    comptime child: anytype,
    comptime A: type,
    comptime forward: fn (@TypeOf(child).Decoded) outcomes.Result(A, issue.Issue),
    comptime backward: fn (*const A) outcomes.Result(@TypeOf(child).Decoded, issue.Issue),
) PureCodec(A, std.json.Value) {
    const C = @TypeOf(child);
    if (C.Representation != std.json.Value or C.DecodingEnvironment != void or C.EncodingEnvironment != void)
        @compileError("JSON transformations require a pure JSON codec");
    if (outcomes.ownsPayload(C.Decoded))
        @compileError("a pure JSON transformation cannot return a representation borrowing an owned temporary");
    var result = PureCodec(A, std.json.Value).init(struct {
        fn decode(encoded: *const std.json.Value) outcomes.Result(A, issue.Issue) {
            return switch (child.decodeDirect(encoded)) {
                .ok => |value| forward(value),
                .err => |problem| .{ .err = problem },
            };
        }
    }.decode, struct {
        fn encode(value: *const A) outcomes.Result(std.json.Value, issue.Issue) {
            const mapped = backward(value);
            return switch (mapped) {
                .ok => |decoded| blk: {
                    var owned = decoded;
                    defer outcomes.discardPayload(&owned);
                    break :blk child.encodeDirect(&owned);
                },
                .err => |problem| .{ .err = problem },
            };
        }
    }.encode);
    if (comptime @hasField(C, "writer") and child.writer != null) result.writer = struct {
        fn write(value: *const A, writer: *@import("json_writer.zig").Writer) outcomes.Exit(void, issue.Issue) {
            const mapped = backward(value);
            return switch (mapped) {
                .err => |problem| .{ .fail = problem },
                .ok => |decoded| blk: {
                    var owned = decoded;
                    defer outcomes.discardPayload(&owned);
                    break :blk child.writeJson(&owned, writer);
                },
            };
        }
    }.write;
    result.encoded_shape = checkedShape(child, "transform");
    return result;
}

/// Require actual public direct JSON writer evidence and expose a lazy bounded Effect adapter.
pub const requireWriter = @import("json_writer.zig").requireWriter;
/// Positive inclusive output, depth, lexical-value and decimal-digit budgets for direct writing.
pub const WriteLimits = @import("json_writer.zig").Limits;
fn scalarWriter(comptime codec: anytype) @TypeOf(codec) {
    var result = codec;
    const Primitive = @TypeOf(codec).Decoded;
    result.encoded_shape = &struct {
        const node: Ast.Node = .{ .primitive = if (Primitive == bool) .boolean else if (Primitive == i64) .signed_integer else if (Primitive == f64) .finite_number else if (Primitive == []const u8) .string else if (Primitive == void) .null_value else @compileError("unknown built-in JSON primitive") };
    }.node;
    result.writer = struct {
        fn write(value: *const @TypeOf(codec).Decoded, writer: *@import("json_writer.zig").Writer) outcomes.Exit(void, issue.Issue) {
            const encoded = codec.encodeDirect(value);
            if (encoded == .err) return .{ .fail = encoded.err };
            const representation_value = encoded.ok;
            const written = if (comptime @TypeOf(codec).Decoded == bool) writer.boolean(representation_value.bool) else writer.json(&representation_value);
            written catch |problem| {
                if (problem == error.OutOfMemory) return .{ .die = outcomes.Defect.out_of_memory };
                var failure = issue.Issue.pure(.encode, switch (problem) {
                    error.OutOfMemory => .resource_limit,
                    error.SizeLimit => .size_limit,
                    error.DepthLimit => .depth_limit,
                    error.InvalidUnicode => .invalid_utf8,
                    error.InvalidNumber => .invalid_number,
                    error.IntegerDigits => .integer_digits,
                });
                failure.detail = writer.resource_failure;
                return .{ .fail = failure };
            };
            return .{ .success = {} };
        }
    }.write;
    return result;
}

/// Pure Unicode/UUID validation and the explicitly named literal substring
/// dialect. The static matcher uses a caller-declared operation bound; regex
/// dialects and wall-clock timeouts require JsonConstraints.PatternCapability.
/// Pure UTF-8 string checks in both directions with inclusive Unicode-scalar bounds.
/// UUID v4 validates version/variant as well as shape. literal is a substring,
/// not regex syntax; matching uses the explicit value-bytes × pattern-bytes work bound.
/// Results borrow their input document/value. Use JsonConstraints.String for
/// caller-selected effectful pattern providers and owned runtime diagnostics.
pub fn constrainedString(comptime minimum: usize, comptime maximum: usize, comptime format: enum { none, uuid, uuid_v4 }, comptime literal: ?[]const u8, comptime maximum_match_steps: ?usize) PureCodec([]const u8, std.json.Value) {
    if (minimum > maximum) @compileError("invalid static Unicode string bounds");
    if (literal != null and (maximum_match_steps == null or maximum_match_steps.? == 0)) @compileError("literal patterns require a positive step bound");
    const Check = struct {
        fn validate(value: []const u8, direction: issue.Direction) outcomes.Result(void, issue.Issue) {
            const count = std.unicode.utf8CountCodepoints(value) catch return .{ .err = issue.Issue.pure(direction, .invalid_utf8) };
            if (count < minimum or count > maximum) return .{ .err = issue.Issue.pure(direction, .constraint) };
            if (format != .none) {
                if (value.len != 36) return .{ .err = issue.Issue.pure(direction, .constraint) };
                for (value, 0..) |character, index| {
                    const separator = index == 8 or index == 13 or index == 18 or index == 23;
                    if (if (separator) character != '-' else !std.ascii.isHex(character)) return .{ .err = issue.Issue.pure(direction, .constraint) };
                }
            }
            if (format == .uuid_v4 and (value[14] != '4' or std.mem.indexOfScalar(u8, "89aAbB", value[19]) == null)) return .{ .err = issue.Issue.pure(direction, .constraint) };
            if (literal) |pattern| {
                const work = std.math.mul(usize, value.len, @max(pattern.len, 1)) catch return .{ .err = issue.Issue.pure(direction, .pattern_timeout) };
                if (work > maximum_match_steps.?) return .{ .err = issue.Issue.pure(direction, .pattern_timeout) };
                if (std.mem.indexOf(u8, value, pattern) == null) return .{ .err = issue.Issue.pure(direction, .constraint) };
            }
            return .{ .ok = {} };
        }
    };
    const codec = PureCodec([]const u8, std.json.Value).init(struct {
        fn decode(input: *const std.json.Value) outcomes.Result([]const u8, issue.Issue) {
            if (input.* != .string) return .{ .err = issue.Issue.pure(.decode, .{ .expected = .string }) };
            const checked = Check.validate(input.string, .decode);
            return switch (checked) {
                .ok => .{ .ok = input.string },
                .err => |problem| .{ .err = problem },
            };
        }
    }.decode, struct {
        fn encode(input: *const []const u8) outcomes.Result(std.json.Value, issue.Issue) {
            const checked = Check.validate(input.*, .encode);
            return switch (checked) {
                .ok => .{ .ok = .{ .string = input.* } },
                .err => |problem| .{ .err = problem },
            };
        }
    }.encode);
    var result = scalarWriter(codec);
    result.encoded_shape = &struct {
        const base: Ast.Node = .{ .primitive = .string };
        const lengths: Ast.Node = .{ .constrained = .{ .base = &base, .rules = &.{.{ .length = .{ .minimum = minimum, .maximum = maximum, .unit = .unicode_scalars } }} } };
        const formatted: Ast.Node = if (format == .none) lengths else .{ .constrained = .{ .base = &lengths, .rules = &.{.{ .format = if (format == .uuid) .uuid else .uuid_v4 }} } };
        const node: Ast.Node = if (literal == null) formatted else .{ .constrained = .{ .base = &formatted, .rules = &.{.{ .pattern = .{ .dialect = "literal-substring", .text = literal.? } }} } };
    }.node;
    return result;
}
