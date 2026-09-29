//! Encoded-shape metadata attached to codecs rather than inferred from Zig
//! application types. Only known built-in codecs get built-in witnesses.
const std = @import("std");
const outcomes = @import("../../outcome.zig");
const ast = @import("../schema_ast.zig");
const json = @import("../json_codec.zig");
const Issue = @import("../schema_issue.zig").Issue;

pub fn Primitive(comptime primitive: ast.Primitive) type {
    const codec = if (comptime primitive == .boolean) json.boolean else if (comptime primitive == .signed_integer) json.signed_integer else if (comptime primitive == .finite_number) json.finite_number else if (comptime primitive == .string) json.borrowed_string else if (comptime primitive == .null_value) json.null_value else @compileError("this primitive has no built-in JSON codec witness");
    const C = @TypeOf(codec);
    return struct {
        pub const Decoded = C.Decoded;
        pub const Representation = std.json.Value;
        pub const DecodingEnvironment = void;
        pub const EncodingEnvironment = void;
        pub const encoded_node: ast.Node = .{ .primitive = primitive };

        pub fn decodeDirect(_: @This(), source: *const std.json.Value) outcomes.Result(Decoded, Issue) {
            return codec.decodeDirect(source);
        }

        pub fn encodeDirect(_: @This(), value: *const Decoded) outcomes.Result(std.json.Value, Issue) {
            return codec.encodeDirect(value);
        }
    };
}

pub const boolean: Primitive(.boolean) = .{};
pub const signed_integer: Primitive(.signed_integer) = .{};
pub const finite_number: Primitive(.finite_number) = .{};
pub const borrowed_string: Primitive(.string) = .{};
pub const null_value: Primitive(.null_value) = .{};

pub fn Nullable(comptime child: anytype) type {
    const C = @TypeOf(child);
    comptime {
        if (!@hasDecl(C, "encoded_node") or C.Representation != std.json.Value or
            C.DecodingEnvironment != void or C.EncodingEnvironment != void)
            @compileError("described nullable requires a described pure JSON codec");
    }
    const codec = json.nullable(child);
    return struct {
        pub const Decoded = ?C.Decoded;
        pub const Representation = std.json.Value;
        pub const DecodingEnvironment = void;
        pub const EncodingEnvironment = void;
        pub const encoded_node: ast.Node = .{ .nullable = &C.encoded_node };

        pub fn decodeDirect(_: @This(), source: *const std.json.Value) outcomes.Result(Decoded, Issue) {
            return codec.decodeDirect(source);
        }

        pub fn encodeDirect(_: @This(), value: *const Decoded) outcomes.Result(std.json.Value, Issue) {
            return codec.encodeDirect(value);
        }
    };
}

pub fn nullable(comptime child: anytype) Nullable(child) {
    return .{};
}

/// A structural builder exposes encodedNode(); a known pure codec exposes
/// encoded_node. Arbitrary callbacks do not acquire a representation witness.
pub fn nodePointer(comptime child: anytype) *const ast.Node {
    const C = if (@TypeOf(child) == type) child else @TypeOf(child);
    if (@hasDecl(C, "encodedNode")) return C.encodedNode();
    if (@hasDecl(C, "encoded_node")) return &C.encoded_node;
    @compileError("JSON Schema export needs an encoded-shape witness from every child codec");
}

pub fn named(comptime Schema: type, comptime name: []const u8, comptime annotations: []const ast.Annotation, comptime accuracy: ast.Accuracy) ast.Definition {
    if (name.len == 0 or !std.unicode.utf8ValidateSlice(name))
        @compileError("named schema identity must be nonempty UTF-8");
    if (!@hasDecl(Schema, "encodedNode")) @compileError("named schema requires an encoded structural witness");
    return .{ .name = name, .root = Schema.encodedNode(), .annotations = annotations, .accuracy = accuracy };
}
