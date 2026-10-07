//! Owned, structured codec failures. Field names in paths are copied so an
//! issue can outlive the JSON document or frame from which it was produced.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const runtime = @import("../internal/runtime.zig");

/// `Direction` cases: `definition`, `decode`, `encode`, `representation`.
pub const Direction = enum { definition, decode, encode, representation };
/// `Expected` cases: `null`, `boolean`, `integer`, `finite_number`, `string`, `array`, `object`,
/// `tagged_case`.
pub const Expected = enum { null, boolean, integer, finite_number, string, array, object, tagged_case };
/// Validation issue category with expected-shape or caller-defined detail.
pub const Kind = union(enum) {
    expected: Expected,
    missing_field,
    unknown_field,
    duplicate_field,
    invalid_value,
    invalid_utf8,
    invalid_number,
    incompatible_version,
    unsupported_representation,
    depth_limit,
    size_limit,
    multiple,
    /// A declared value constraint rejected the input; detail may own operands.
    constraint,
    /// Alternative acceptance/ambiguity failed; detail records matching branches.
    alternatives,
    /// Child encoders produced incompatible overlapping representations.
    conflict,
    /// A caller-selected structural/input/output budget was exceeded.
    resource_limit,
    /// Expanded decimal digits exceed policy or cannot be represented by the counter.
    integer_digits,
    /// Provider-reported matching timeout/work exhaustion.
    pattern_timeout,
    /// Provider rejected or failed a declared pattern.
    pattern_failure,
    custom: []const u8,
};

/// One field name or array index identifying where validation failed.
pub const PathElement = union(enum) {
    field: outcomes.OwnedSlice(u8),
    index: usize,
    /// Owned object key where its name codec failed.
    property_name: outcomes.OwnedSlice(u8),
    /// Zero-based branch index where a child codec failed.
    alternative: usize,

    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        switch (self.*) {
            .field => self.field.deinit(),
            .property_name => self.property_name.deinit(),
            else => {},
        }
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .field => |name| .{ .field = try name.clone() },
            .index => |value| .{ .index = value },
            .property_name => |name| .{ .property_name = try name.clone() },
            .alternative => |value| .{ .alternative = value },
        };
    }
};

/// Structured details own all runtime text and matching-alternative metadata.
pub const Detail = union(enum) {
    /// Constraint identifier and expected/actual values; all three runtime strings are owned.
    constraint: struct { name: outcomes.OwnedSlice(u8), expected: outcomes.OwnedSlice(u8), actual: outcomes.OwnedSlice(u8) },
    /// Owned zero-based indices of successful alternatives, including ambiguous matches.
    alternatives: outcomes.OwnedSlice(usize),
    /// Exceeded budget with its inclusive limit and attempted count in that resource’s units.
    resource: struct { resource: Resource, limit: usize, actual: usize },
    /// Expanded decimal integer-part bound; null required means its count could not fit.
    integer_digits: struct { limit: usize, required: ?usize },
    /// Owned dialect/source and the provider timeout in nanoseconds.
    pattern: struct { dialect: outcomes.OwnedSlice(u8), source: outcomes.OwnedSlice(u8), timeout_nanoseconds: i96 },
    /// Identity of the explicit caller budget that was exceeded; units follow the selected resource.
    pub const Resource = enum {
        /// Distinct immutable schema nodes visited during preparation.
        schema_nodes,
        /// Child links counted during schema preparation.
        schema_edges,
        /// Schema traversal levels, with the root at level one.
        schema_depth,
        /// Simultaneous JSON values, including root and scalars.
        json_depth,
        /// UTF-8 bytes in an input envelope/document.
        input_bytes,
        /// Emitted UTF-8 bytes, including syntax and escapes.
        output_bytes,
        /// Unescaped string/key or lexical numeric bytes.
        value_bytes,
        /// Retained structured validation issue count.
        issues,
        /// Decimal integer-part digits after exponent expansion.
        integer_digits,
        /// Work steps reported by a selected pattern provider.
        pattern_steps,
    };
    /// Release owned text/index arrays in this detail once; numeric resource counters own no storage.
    pub fn deinit(self: *@This()) void {
        switch (self.*) {
            .constraint => |*value| {
                value.name.deinit();
                value.expected.deinit();
                value.actual.deinit();
            },
            .alternatives => |*value| value.deinit(),
            .pattern => |*value| {
                value.dialect.deinit();
                value.source.deinit();
            },
            else => {},
        }
        self.* = undefined;
    }
    /// Copy owned diagnostic text/index arrays into an independent detail; primitive counters copy by value.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .constraint => |value| blk: {
                var name = try value.name.clone();
                errdefer name.deinit();
                var expected = try value.expected.clone();
                errdefer expected.deinit();
                break :blk .{ .constraint = .{ .name = name, .expected = expected, .actual = try value.actual.clone() } };
            },
            .alternatives => |value| .{ .alternatives = try value.clone() },
            .pattern => |value| blk: {
                var dialect = try value.dialect.clone();
                errdefer dialect.deinit();
                break :blk .{ .pattern = .{ .dialect = dialect, .source = try value.source.clone(), .timeout_nanoseconds = value.timeout_nanoseconds } };
            },
            .resource, .integer_digits => self.*,
        };
    }
};

/// Typed validation failure with kind and path; release any owned path components.
pub const Issue = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    /// Zig error set for path error, distinct from Effect’s typed `Exit` failure channel.
    pub const PathError = error{ OutOfMemory, MissingAllocator };
    direction: Direction,
    kind: Kind,
    /// Root to leaf. Null is an empty path and owns no allocation.
    path: ?[]PathElement = null,
    allocator: ?std.mem.Allocator = null,
    related: ?[]Issue = null,
    limit_reached: bool = false,
    detail: ?Detail = null,
    owned_custom: ?outcomes.OwnedSlice(u8) = null,

    /// The caller selects the allocation domain through its environment.
    /// `custom` identifiers must be static definition metadata, never a view
    /// borrowed from a parsed document or transient frame.
    ///
    /// Parameters:
    /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
    /// - `direction: Direction`: Encode or decode direction for the validation issue.
    /// - `kind: Kind`: Discriminator selecting the operation or issue variant.
    /// Returns (`Issue`): A typed schema issue carrying direction, path, and failure kind.
    pub fn init(env: anytype, direction: Direction, kind: Kind) Issue {
        const R = @typeInfo(@TypeOf(env)).pointer.child;
        const Field = @tagName(runtime.EnvironmentField.allocator);
        if (R == void or !@hasField(R, Field)) @compileError("SchemaIssue requires the caller allocator in R");
        if (@FieldType(R, Field) != std.mem.Allocator) @compileError("R.allocator must be std.mem.Allocator");
        return .{ .direction = direction, .kind = kind, .allocator = env.allocator };
    }

    /// A pure codec can report a root issue without allocating. A composing
    /// Effect binds its caller allocator before adding an owned path element.
    ///
    /// Parameters:
    /// - `direction: Direction`: Encode or decode direction for the validation issue.
    /// - `kind: Kind`: Discriminator selecting the operation or issue variant.
    /// Returns (`Issue`): A typed schema issue carrying direction, path, and failure kind.
    pub fn pure(direction: Direction, kind: Kind) Issue {
        return .{ .direction = direction, .kind = kind };
    }

    /// Bind an allocator to issue payload ownership.
    /// Parameters:
    /// - `self: *Issue`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
    /// Returns (`void`): no value.
    pub fn bindAllocator(self: *Issue, env: anytype) void {
        const R = @typeInfo(@TypeOf(env)).pointer.child;
        const Field = @tagName(runtime.EnvironmentField.allocator);
        if (R == void or !@hasField(R, Field)) @compileError("SchemaIssue requires the caller allocator in R");
        if (@FieldType(R, Field) != std.mem.Allocator) @compileError("R.allocator must be std.mem.Allocator");
        if (self.allocator == null) self.allocator = env.allocator;
    }

    /// Construct a custom typed validation issue.
    /// Parameters:
    /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
    /// - `direction: Direction`: Encode or decode direction for the validation issue.
    /// - `identifier: comptime []const u8`: Stable identifier used in a schema or protocol definition.
    /// Returns (`Issue`): A typed schema issue carrying direction, path, and failure kind.
    pub fn custom(env: anytype, direction: Direction, comptime identifier: []const u8) Issue {
        if (identifier.len == 0) @compileError("a custom SchemaIssue needs a stable identifier");
        return init(env, direction, .{ .custom = identifier });
    }

    /// Takes ownership of an allocator-owned list of already structured
    /// issues. A reached limit says more input may remain unchecked.
    ///
    /// Parameters:
    /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
    /// - `direction: Direction`: Encode or decode direction for the validation issue.
    /// - `issues: []Issue`: Owned validation issues accumulated for this result.
    /// - `limit_reached: bool`: Whether issue accumulation stopped at the caller’s bound.
    /// Returns (`Issue`): A typed schema issue carrying direction, path, and failure kind.
    pub fn many(env: anytype, direction: Direction, issues: []Issue, limit_reached: bool) Issue {
        var result = init(env, direction, .multiple);
        result.related = issues;
        result.limit_reached = limit_reached;
        return result;
    }

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *Issue`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *Issue) void {
        if (self.detail) |*detail| detail.deinit();
        if (self.owned_custom) |*custom_| custom_.deinit();
        if (self.path) |elements| {
            for (elements) |*element| element.deinit();
            self.allocator.?.free(elements);
        }
        if (self.related) |issues| {
            for (issues) |*issue| issue.deinit();
            self.allocator.?.free(issues);
        }
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const Issue`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!Issue`): A `Issue` on success; `OutOfMemory` if required allocation fails.
    pub fn clone(self: *const Issue) error{OutOfMemory}!Issue {
        var result: Issue = .{ .direction = self.direction, .kind = self.kind, .allocator = self.allocator, .limit_reached = self.limit_reached };
        errdefer result.deinit();
        if (self.detail) |*detail| result.detail = try detail.clone();
        if (self.owned_custom) |custom_| {
            result.owned_custom = try custom_.clone();
            result.kind = .{ .custom = result.owned_custom.?.items };
        }
        if (self.path) |elements| {
            const allocator = self.allocator.?;
            const owned = try allocator.alloc(PathElement, elements.len);
            var copied: usize = 0;
            errdefer {
                for (owned[0..copied]) |*element| element.deinit();
                allocator.free(owned);
            }
            for (elements, owned) |element, *slot| {
                slot.* = try element.clone();
                copied += 1;
            }
            result.path = owned;
        }
        if (self.related) |issues| {
            const allocator = self.allocator.?;
            const owned = try allocator.alloc(Issue, issues.len);
            var copied: usize = 0;
            errdefer {
                for (owned[0..copied]) |*issue| issue.deinit();
                allocator.free(owned);
            }
            for (issues, owned) |*issue, *slot| {
                slot.* = try issue.clone();
                copied += 1;
            }
            result.related = owned;
        }
        return result;
    }

    /// Prepends a named path component after copying it. Allocation failure
    /// leaves the original issue untouched, including its existing path.
    ///
    /// Parameters:
    /// - `self: *Issue`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `name: []const u8`: Stable, nonempty identity component validated at definition time.
    /// Returns (`PathError!void`): A `void` on success, or a Zig error in `PathError`.
    pub fn prependField(self: *Issue, name: []const u8) PathError!void {
        const allocator = self.allocator orelse return error.MissingAllocator;
        const copied_name = try allocator.dupe(u8, name);
        errdefer allocator.free(copied_name);
        try self.prepend(.{ .field = .{ .items = copied_name, .allocator = allocator } });
    }

    /// Prepend an array index to a validation path.
    /// Parameters:
    /// - `self: *Issue`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `index: usize`: Position in the collection or definition table.
    /// Returns (`PathError!void`): A `void` on success, or a Zig error in `PathError`.
    pub fn prependIndex(self: *Issue, index: usize) PathError!void {
        try self.prepend(.{ .index = index });
    }

    /// Add a zero-based branch index at the front of an owning issue path.
    pub fn prependAlternative(self: *Issue, index: usize) PathError!void {
        try self.prepend(.{ .alternative = index });
    }
    /// Copy a property name into the issue’s allocator and prepend a property-name validation location.
    /// The name may be released after this call; a missing allocator or OOM leaves the issue unchanged.
    pub fn prependPropertyName(self: *Issue, name: []const u8) PathError!void {
        const allocator = self.allocator orelse return error.MissingAllocator;
        const copied = try allocator.dupe(u8, name);
        errdefer allocator.free(copied);
        try self.prepend(.{ .property_name = .{ .items = copied, .allocator = allocator } });
    }

    fn prepend(self: *Issue, first: PathElement) PathError!void {
        const allocator = self.allocator orelse return error.MissingAllocator;
        const old: []const PathElement = if (self.path) |existing| existing else &.{};
        const length = std.math.add(usize, old.len, 1) catch return error.OutOfMemory;
        const new_path = try allocator.alloc(PathElement, length);
        new_path[0] = first;
        std.mem.copyForwards(PathElement, new_path[1..], old);
        if (self.path) |previous| allocator.free(previous);
        self.path = new_path;
    }

    /// Construct or access the value at the selected position.
    /// Parameters:
    /// - `self: *const Issue`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// - `index: usize`: Position in the collection or definition table.
    /// Returns (`?*const PathElement`): path element at this index, or `null` when out of range.
    pub fn at(self: *const Issue, index: usize) ?*const PathElement {
        const elements = self.path orelse return null;
        return if (index < elements.len) &elements[index] else null;
    }
};
