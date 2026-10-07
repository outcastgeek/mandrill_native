//! Encoded-shape metadata attached to codecs rather than inferred from Zig
//! application types. Only known built-in codecs get built-in witnesses.
const std = @import("std");
const outcomes = @import("../../outcome.zig");
const ast = @import("../schema_ast.zig");
const json = @import("../json_codec.zig");
const Issue = @import("../schema_issue.zig").Issue;

/// Construct a built-in structural representation witness.
/// Parameters:
/// - `primitive: comptime ast.Primitive`: Compile-time primitive schema kind.
/// Returns (`type`): built-in structural representation witness for a primitive value shape.
pub fn Primitive(comptime primitive: ast.Primitive) type {
    const codec = if (comptime primitive == .boolean) json.boolean else if (comptime primitive == .signed_integer) json.signed_integer else if (comptime primitive == .finite_number) json.finite_number else if (comptime primitive == .string) json.borrowed_string else if (comptime primitive == .null_value) json.null_value else @compileError("this primitive has no built-in JSON codec witness");
    const C = @TypeOf(codec);
    return struct {
        /// Application value type produced by this codec.
        pub const Decoded = C.Decoded;
        /// Encoded representation type consumed and produced by this codec.
        pub const Representation = std.json.Value;
        /// Environment required to run decoding effects.
        pub const DecodingEnvironment = void;
        /// Environment required to run encoding effects.
        pub const EncodingEnvironment = void;
        pub const pure_decoding = true;
        pub const pure_encoding = true;
        pub const supports_direct_writer = true;
        pub fn writeJson(_: @This(), value: *const Decoded, writer: *@import("../json_writer.zig").Writer) outcomes.Exit(void, Issue) {
            return codec.writeJson(value, writer);
        }
        pub fn validateDirect(_: @This(), value: *const Decoded) outcomes.Result(void, Issue) {
            return codec.validateDirect(value);
        }

        /// Structural encoded-node witness for this primitive codec.
        pub const encoded_node: ast.Node = .{ .primitive = primitive };

        /// Decode immediately and return the typed validation outcome.
        /// Parameters:
        /// - `_: @This()`: Unused argument required by the capability signature.
        /// - `source: *const std.json.Value`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`outcomes.Result(Decoded, Issue)`): An ok/err Result; owned payloads must be released according to their type contract.
        pub fn decodeDirect(_: @This(), source: *const std.json.Value) outcomes.Result(Decoded, Issue) {
            return codec.decodeDirect(source);
        }

        /// Encode immediately and return a typed outcome.
        /// Parameters:
        /// - `_: @This()`: Unused argument required by the capability signature.
        /// - `value: *const Decoded`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`outcomes.Result(std.json.Value, Issue)`): An ok/err Result; owned payloads must be released according to their type contract.
        pub fn encodeDirect(_: @This(), value: *const Decoded) outcomes.Result(std.json.Value, Issue) {
            return codec.encodeDirect(value);
        }
    };
}

/// Built-in structural witness for JSON boolean.
pub const boolean: Primitive(.boolean) = .{};
/// Built-in structural witness for JSON signed integer.
pub const signed_integer: Primitive(.signed_integer) = .{};
/// Built-in structural witness for finite JSON number.
pub const finite_number: Primitive(.finite_number) = .{};
/// Built-in structural witness for borrowed JSON string.
pub const borrowed_string: Primitive(.string) = .{};
/// Built-in structural witness for JSON null.
pub const null_value: Primitive(.null_value) = .{};

/// Construct a built-in structural representation witness.
/// Parameters:
/// - `child: comptime anytype`: Compile-time statically inferred input checked when this operation is instantiated.
/// Returns (`type`): built-in structural representation witness for a nullable value shape.
pub fn Nullable(comptime child: anytype) type {
    const C = @TypeOf(child);
    comptime {
        if (!@hasDecl(C, "encoded_node") or C.Representation != std.json.Value or
            C.DecodingEnvironment != void or C.EncodingEnvironment != void)
            @compileError("described nullable requires a described pure JSON codec");
    }
    const codec = json.nullable(child);
    return struct {
        /// Application value type produced by this codec.
        pub const Decoded = ?C.Decoded;
        /// Encoded representation type consumed and produced by this codec.
        pub const Representation = std.json.Value;
        /// Environment required to run decoding effects.
        pub const DecodingEnvironment = void;
        /// Environment required to run encoding effects.
        pub const EncodingEnvironment = void;
        pub const pure_decoding = true;
        pub const pure_encoding = true;
        pub const supports_direct_writer = @hasDecl(C, "supports_direct_writer") and C.supports_direct_writer;
        pub fn writeJson(_: @This(), value: *const Decoded, writer: *@import("../json_writer.zig").Writer) outcomes.Exit(void, Issue) {
            if (!supports_direct_writer) @compileError("nullable child has no writer evidence");
            if (value.*) |*present| return child.writeJson(present, writer);
            const absent: std.json.Value = .null;
            writer.json(&absent) catch |problem| {
                if (problem == error.OutOfMemory) return .{ .die = outcomes.Defect.out_of_memory };
                var failure = Issue.pure(.encode, if (problem == error.DepthLimit) .depth_limit else .size_limit);
                failure.detail = writer.resource_failure;
                return .{ .fail = failure };
            };
            return .{ .success = {} };
        }

        /// Structural encoded-node witness for this primitive codec.
        pub const encoded_node: ast.Node = .{ .nullable = &C.encoded_node };

        /// Decode immediately and return the typed validation outcome.
        /// Parameters:
        /// - `_: @This()`: Unused argument required by the capability signature.
        /// - `source: *const std.json.Value`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`outcomes.Result(Decoded, Issue)`): An ok/err Result; owned payloads must be released according to their type contract.
        pub fn decodeDirect(_: @This(), source: *const std.json.Value) outcomes.Result(Decoded, Issue) {
            return codec.decodeDirect(source);
        }

        /// Encode immediately and return a typed outcome.
        /// Parameters:
        /// - `_: @This()`: Unused argument required by the capability signature.
        /// - `value: *const Decoded`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`outcomes.Result(std.json.Value, Issue)`): An ok/err Result; owned payloads must be released according to their type contract.
        pub fn encodeDirect(_: @This(), value: *const Decoded) outcomes.Result(std.json.Value, Issue) {
            return codec.encodeDirect(value);
        }
    };
}

/// Wrap a representation so it can encode a null value.
/// Parameters:
/// - `child: comptime anytype`: Compile-time statically inferred input checked when this operation is instantiated.
/// Returns (`Nullable(child)`): A typed codec or schema preserving the declared decoded value, representation, and environments.
pub fn nullable(comptime child: anytype) Nullable(child) {
    return .{};
}

/// A structural builder exposes encodedNode(); a known pure codec exposes
/// encoded_node. Arbitrary callbacks do not acquire a representation witness.
///
/// Parameters:
/// - `child: comptime anytype`: Compile-time statically inferred input checked when this operation is instantiated.
/// Returns (`*const ast.Node`): A borrowed read-only pointer; keep its owner alive while using it.
pub fn nodePointer(comptime child: anytype) *const ast.Node {
    const C = if (@TypeOf(child) == type) child else @TypeOf(child);
    if (@hasDecl(C, "encodedNode")) return C.encodedNode();
    if (@hasDecl(C, "encoded_node")) return &C.encoded_node;
    if (@TypeOf(child) != type and @hasField(C, "encoded_shape")) {
        if (child.encoded_shape) |node| return node;
    }
    @compileError("JSON Schema export needs an encoded-shape witness from every child codec");
}

/// Construct a named structural schema definition.
/// Parameters:
/// - `Schema: comptime type`: Compile-time schema type whose decoded value and representation define this API.
/// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition time.
/// - `annotations: comptime []const ast.Annotation`: Metadata attached to the definition or record.
/// - `accuracy: comptime ast.Accuracy`: Compile-time accuracy mode for this schema.
/// Returns (`ast.Definition`): Structural schema definition with a named root and annotations.
pub fn named(comptime Schema: type, comptime name: []const u8, comptime annotations: []const ast.Annotation, comptime accuracy: ast.Accuracy) ast.Definition {
    if (name.len == 0 or !std.unicode.utf8ValidateSlice(name))
        @compileError("named schema identity must be nonempty UTF-8");
    if (!@hasDecl(Schema, "encodedNode")) @compileError("named schema requires an encoded structural witness");
    return .{ .name = name, .root = Schema.encodedNode(), .annotations = annotations, .accuracy = accuracy };
}
