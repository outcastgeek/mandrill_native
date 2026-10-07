//! Immutable structural metadata. The AST describes representations and
//! constraints; it is not a witness that can construct an arbitrary Zig type.
//! Typed constructors, projections and codecs supply that separate proof.
const std = @import("std");
const outcomes = @import("../outcome.zig");

/// Wire-level primitive kind. It describes representation, not a Zig value constructor.
pub const Primitive = enum {
    boolean,
    signed_integer,
    unsigned_integer,
    finite_number,
    decimal,
    string,
    bytes,
    identifier,
    timestamp,
    null_value,
};

/// Record-field presence contract: required, optional, or supplied by a typed default.
pub const Presence = enum { required, optional, defaulted };
/// Policy for unrecognized JSON record fields: reject, ignore, or preserve.
pub const UnknownFields = enum { reject, ignore, preserve };
/// Policy for duplicate JSON field names during decoding.
pub const DuplicateFields = enum { reject, first_wins, last_wins };
/// Validation reporting policy: stop at the first issue or accumulate within a bound.
pub const Issues = enum { first, accumulate };
/// Whether exported representation metadata is exact or conservative.
pub const Accuracy = enum { exact, conservative };

/// A textual format name is an explicit contract tag, never inferred from a
/// Zig type name or provider. Pattern translation is explicitly supported or
/// rejected by a representation exporter.
pub const Annotation = struct { name: []const u8, value: []const u8 };

/// Optional inclusive minimum and maximum for integer constraints.
pub const Bounds = struct {
    minimum: ?i128 = null,
    maximum: ?i128 = null,
};

/// Optional minimum and maximum length with an explicit byte, Unicode-scalar, or element unit.
pub const Length = struct {
    minimum: ?usize = null,
    maximum: ?usize = null,
    unit: Unit,

    /// `Unit` cases: `bytes`, `unicode_scalars`, `elements`.
    pub const Unit = enum { bytes, unicode_scalars, elements };
};

/// Representation constraint, including integer bounds, measured length, and a named pattern dialect.
pub const Constraint = union(enum) {
    integer_bounds: Bounds,
    length: Length,
    /// A named pattern dialect and text are supplied explicitly. Exporters
    /// reject unsupported dialects instead of silently changing semantics.
    pattern: struct { dialect: []const u8, text: []const u8 },
    /// UUID assertion metadata; draft-2020-12 exact export rejects annotation-only semantics.
    format: enum { uuid, uuid_v4 },
    /// Named executable check whose portable JSON Schema semantics are unknown.
    check: []const u8,
};

/// Borrowed record-field name, presence policy, and child schema node.
pub const Field = struct {
    name: []const u8,
    presence: Presence,
    schema: *const Node,
};

/// Tagged-union wire tag and optional child payload schema.
pub const Case = struct {
    tag: []const u8,
    payload: ?*const Node,
};

/// Structural record definition with field schemas and unknown/duplicate/issue policies.
pub const Record = struct {
    fields: []const Field,
    unknown_fields: UnknownFields,
    duplicate_fields: DuplicateFields,
    issues: Issues,
};

/// Structural tagged-union definition with discriminator, payload name, and cases.
pub const TaggedUnion = struct {
    discriminator: []const u8,
    payload_name: []const u8,
    cases: []const Case,
};

/// Borrowed exact decimal constraints. Export preserves token spelling and rejects contradictory/invalid operands.
pub const ExactNumber = struct {
    /// Inclusive lower exact decimal token.
    minimum: ?[]const u8 = null,
    /// Inclusive upper exact decimal token.
    maximum: ?[]const u8 = null,
    /// Strict lower exact decimal token.
    exclusive_minimum: ?[]const u8 = null,
    /// Strict upper exact decimal token.
    exclusive_maximum: ?[]const u8 = null,
    /// Strictly positive exact decimal divisor.
    multiple_of: ?[]const u8 = null,
    /// Require an integral mathematical value when true.
    integer: bool,
};
/// Immutable alternative mode and child shapes; no candidate or parse state is stored here.
pub const Alternatives = struct {
    /// Exact-one, at-least-one or all-branches acceptance semantics.
    mode: enum { one, any, all },
    /// Borrowed immutable child shapes in declaration order.
    branches: []const *const Node,
};
/// Positional prefix/rest metadata with inclusive element bounds and mathematical uniqueness.
pub const PrefixArray = struct {
    /// Ordered prefix shapes; required positions precede optional positions.
    elements: []const struct {
        /// Borrowed shape for this zero-based position.
        schema: *const Node,
        /// Missing this position fails when true.
        required: bool,
    },
    /// Remaining element shape; null rejects elements beyond the prefix.
    rest: ?*const Node,
    /// Inclusive minimum total element count.
    minimum: usize,
    /// Inclusive maximum total element count.
    maximum: usize,
    /// Require mathematical JSON inequality between every pair of elements.
    unique: bool,
};
/// Borrowed provider pattern dialect/source plus the shape applied to every matching property.
pub const PatternProperty = struct {
    /// Explicit borrowed pattern dialect, translated only when supported.
    dialect: []const u8,
    /// Borrowed original source; conservative export may omit unsupported semantics.
    source: []const u8,
    /// Borrowed child shape applied to matching properties.
    schema: *const Node,
};
/// Object validation shape layered over a base record with name/pattern/additional constraints.
pub const Object = struct {
    /// Borrowed base record declaring named properties.
    base: *const Node,
    /// Optional shape validating every property name.
    property_names: ?*const Node,
    /// Inclusive minimum distinct property count.
    minimum: usize,
    /// Inclusive maximum distinct property count.
    maximum: usize,
    /// Every matching pattern child applies, including overlaps with named fields.
    patterns: []const PatternProperty,
    /// Unmatched property shape; null rejects unmatched properties.
    additional: ?*const Node,
};

/// Named references are resolved against a checked schema definition table.
/// They avoid an endlessly recursive in-memory Node construction and allow
/// recursive preparation to memoize each definition exactly once.
pub const Node = union(enum) {
    primitive: Primitive,
    exact_number: ExactNumber,
    enumeration: struct { values: []const []const u8, preserve_unknown: bool },
    alternatives: Alternatives,
    prefix_array: PrefixArray,
    object: Object,
    nullable: *const Node,
    array: *const Node,
    record: Record,
    tagged_union: TaggedUnion,
    reference: []const u8,
    constrained: struct { base: *const Node, rules: []const Constraint },
};

/// Named schema root with annotations and accuracy declaration. The caller keeps referenced nodes
/// alive.
pub const Definition = struct {
    name: []const u8,
    root: *const Node,
    annotations: []const Annotation = &.{},
    accuracy: Accuracy = .exact,
};

/// Typed reason a structural schema definition cannot be prepared or exported.
pub const DefinitionIssue = union(enum) {
    empty_name,
    invalid_utf8,
    duplicate_field: usize,
    duplicate_case: usize,
    invalid_bounds: usize,
    empty_pattern_dialect: usize,
    empty_reference,
    empty_discriminator,
    empty_payload_name,
    empty_union,
    empty_alternatives,
    invalid_prefix,
    invalid_number,
};

/// Validate only local metadata. Dynamic graphs need a bounded resolver that
/// accounts for every occurrence of a shared node; this local check neither
/// executes defaults nor fabricates a typed codec from AST metadata.
///
/// Parameters:
/// - `node: *const Node`: Borrowed schema or representation node being inspected.
/// Returns (`outcomes.Result(void, DefinitionIssue)`): An ok/err Result; owned payloads must be released according to their type contract.
pub fn validateLocal(node: *const Node) outcomes.Result(void, DefinitionIssue) {
    switch (node.*) {
        .enumeration => |value| {
            if (value.values.len == 0) return .{ .err = .empty_alternatives };
            for (value.values, 0..) |name, index| {
                if (!std.unicode.utf8ValidateSlice(name)) return .{ .err = .invalid_utf8 };
                for (value.values[0..index]) |prior| if (std.mem.eql(u8, name, prior)) return .{ .err = .{ .duplicate_case = index } };
            }
        },
        .exact_number => |number| {
            for ([_]?[]const u8{ number.minimum, number.maximum, number.exclusive_minimum, number.exclusive_maximum, number.multiple_of }) |operand| if (operand) |text| {
                _ = @import("json_number_codec.zig").Number.parse(text) catch return .{ .err = .invalid_number };
            };
            const Numeric = @import("json_number_codec.zig").Number;
            if (number.multiple_of) |text| {
                const divisor = Numeric.parse(text) catch unreachable;
                if (divisor.digits == 0 or divisor.negative) return .{ .err = .invalid_number };
            }
            for ([_]?[]const u8{ number.minimum, number.exclusive_minimum }, 0..) |lower, lower_index| {
                for ([_]?[]const u8{ number.maximum, number.exclusive_maximum }, 0..) |upper, upper_index| {
                    if (lower != null and upper != null) {
                        const ordering = (Numeric.parse(lower.?) catch unreachable).order(Numeric.parse(upper.?) catch unreachable);
                        if (ordering == .gt or (ordering == .eq and (lower_index == 1 or upper_index == 1))) return .{ .err = .invalid_number };
                    }
                }
            }
        },
        .alternatives => |value| if (value.branches.len == 0) return .{ .err = .empty_alternatives },
        .prefix_array => |value| {
            if (value.minimum > value.maximum) return .{ .err = .invalid_prefix };
            var optional_seen = false;
            for (value.elements) |element| {
                if (!element.required) optional_seen = true else if (optional_seen) return .{ .err = .invalid_prefix };
            }
        },
        .object => |value| {
            if (value.minimum > value.maximum) return .{ .err = .invalid_prefix };
            for (value.patterns, 0..) |pattern, index| {
                if (pattern.dialect.len == 0) return .{ .err = .{ .empty_pattern_dialect = index } };
                if (!std.unicode.utf8ValidateSlice(pattern.source) or !std.unicode.utf8ValidateSlice(pattern.dialect)) return .{ .err = .invalid_utf8 };
            }
        },
        .record => |record| {
            for (record.fields, 0..) |field, index| {
                if (field.name.len == 0) return .{ .err = .empty_name };
                if (!std.unicode.utf8ValidateSlice(field.name)) return .{ .err = .invalid_utf8 };
                for (record.fields[0..index]) |earlier| {
                    if (std.mem.eql(u8, earlier.name, field.name))
                        return .{ .err = .{ .duplicate_field = index } };
                }
            }
        },
        .tagged_union => |tagged| {
            if (tagged.discriminator.len == 0) return .{ .err = .empty_discriminator };
            if (!std.unicode.utf8ValidateSlice(tagged.discriminator)) return .{ .err = .invalid_utf8 };
            if (tagged.payload_name.len == 0 or std.mem.eql(u8, tagged.discriminator, tagged.payload_name))
                return .{ .err = .empty_payload_name };
            if (!std.unicode.utf8ValidateSlice(tagged.payload_name)) return .{ .err = .invalid_utf8 };
            if (tagged.cases.len == 0) return .{ .err = .empty_union };
            for (tagged.cases, 0..) |case, index| {
                if (case.tag.len == 0) return .{ .err = .empty_name };
                if (!std.unicode.utf8ValidateSlice(case.tag)) return .{ .err = .invalid_utf8 };
                for (tagged.cases[0..index]) |earlier| {
                    if (std.mem.eql(u8, earlier.tag, case.tag))
                        return .{ .err = .{ .duplicate_case = index } };
                }
            }
        },
        .reference => |name| {
            if (name.len == 0) return .{ .err = .empty_reference };
            if (!std.unicode.utf8ValidateSlice(name)) return .{ .err = .invalid_utf8 };
        },
        .constrained => |constrained| {
            for (constrained.rules, 0..) |rule, index| {
                switch (rule) {
                    .integer_bounds => |bounds| {
                        if (bounds.minimum != null and bounds.maximum != null and bounds.minimum.? > bounds.maximum.?)
                            return .{ .err = .{ .invalid_bounds = index } };
                    },
                    .length => |length| {
                        if (length.minimum != null and length.maximum != null and length.minimum.? > length.maximum.?)
                            return .{ .err = .{ .invalid_bounds = index } };
                    },
                    .format => {},
                    .check => |name| {
                        if (name.len == 0 or !std.unicode.utf8ValidateSlice(name)) return .{ .err = .invalid_utf8 };
                    },
                    .pattern => |pattern| {
                        if (pattern.dialect.len == 0) return .{ .err = .{ .empty_pattern_dialect = index } };
                        if (!std.unicode.utf8ValidateSlice(pattern.dialect) or !std.unicode.utf8ValidateSlice(pattern.text))
                            return .{ .err = .invalid_utf8 };
                    },
                }
            }
        },
        else => {},
    }
    return .{ .ok = {} };
}

/// Check structural schema identities, references, and constraints.
/// Parameters:
/// - `definition: Definition`: Static schema, workflow, or protocol definition.
/// Returns (`outcomes.Result(void, DefinitionIssue)`): An ok/err Result; owned payloads must be released according to their type contract.
pub fn validateDefinition(definition: Definition) outcomes.Result(void, DefinitionIssue) {
    if (definition.name.len == 0) return .{ .err = .empty_name };
    if (!std.unicode.utf8ValidateSlice(definition.name)) return .{ .err = .invalid_utf8 };
    for (definition.annotations) |annotation| {
        if (annotation.name.len == 0) return .{ .err = .empty_name };
        if (!std.unicode.utf8ValidateSlice(annotation.name) or !std.unicode.utf8ValidateSlice(annotation.value))
            return .{ .err = .invalid_utf8 };
    }
    return validateLocal(definition.root);
}
