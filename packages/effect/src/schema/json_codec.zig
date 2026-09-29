//! Primitive JSON codecs. Structural codecs compose these without changing
//! the distinction between decoded application values and encoded JSON data.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const issue = @import("schema_issue.zig");
const PureCodec = @import("codec.zig").PureCodec;

/// Pure JSON codec for Zig `bool`.
pub const boolean = PureCodec(bool, std.json.Value).init(struct {
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
}.encode);

/// Pure JSON codec for Zig `i64`.
pub const signed_integer = PureCodec(i64, std.json.Value).init(struct {
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
}.encode);

/// Pure JSON codec for finite Zig `f64` values.
pub const finite_number = PureCodec(f64, std.json.Value).init(struct {
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
}.encode);

/// A decoded string borrows the source JSON document. `Schema.parse` retains
/// that document in Parsed(A); standalone decoders must retain their input.
pub const borrowed_string = PureCodec([]const u8, std.json.Value).init(struct {
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
}.encode);

/// Pure JSON codec for Zig `void` represented as JSON null.
pub const null_value = PureCodec(void, std.json.Value).init(struct {
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
}.encode);

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
    return PureCodec(?Value, std.json.Value).init(struct {
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
    return PureCodec(C.Decoded, std.json.Value).init(struct {
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
    return PureCodec(A, std.json.Value).init(struct {
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
}
