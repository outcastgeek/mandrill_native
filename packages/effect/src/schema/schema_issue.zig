//! Owned, structured codec failures. Field names in paths are copied so an
//! issue can outlive the JSON document or frame from which it was produced.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const runtime = @import("../internal/runtime.zig");

pub const Direction = enum { definition, decode, encode, representation };
pub const Expected = enum { null, boolean, integer, finite_number, string, array, object, tagged_case };
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
    custom: []const u8,
};

pub const PathElement = union(enum) {
    field: outcomes.OwnedSlice(u8),
    index: usize,

    pub const effect_owned_payload = true;

    pub fn deinit(self: *@This()) void {
        if (self.* == .field) self.field.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .field => |name| .{ .field = try name.clone() },
            .index => |value| .{ .index = value },
        };
    }
};

pub const Issue = struct {
    pub const effect_owned_payload = true;
    pub const PathError = error{ OutOfMemory, MissingAllocator };
    direction: Direction,
    kind: Kind,
    /// Root to leaf. Null is an empty path and owns no allocation.
    path: ?[]PathElement = null,
    allocator: ?std.mem.Allocator = null,
    related: ?[]Issue = null,
    limit_reached: bool = false,

    /// The caller selects the allocation domain through its environment.
    /// `custom` identifiers must be static definition metadata, never a view
    /// borrowed from a parsed document or transient frame.
    pub fn init(env: anytype, direction: Direction, kind: Kind) Issue {
        const R = @typeInfo(@TypeOf(env)).pointer.child;
        const Field = @tagName(runtime.EnvironmentField.allocator);
        if (R == void or !@hasField(R, Field)) @compileError("SchemaIssue requires the caller allocator in R");
        if (@FieldType(R, Field) != std.mem.Allocator) @compileError("R.allocator must be std.mem.Allocator");
        return .{ .direction = direction, .kind = kind, .allocator = env.allocator };
    }

    /// A pure codec can report a root issue without allocating. A composing
    /// Effect binds its caller allocator before adding an owned path element.
    pub fn pure(direction: Direction, kind: Kind) Issue {
        return .{ .direction = direction, .kind = kind };
    }

    pub fn bindAllocator(self: *Issue, env: anytype) void {
        const R = @typeInfo(@TypeOf(env)).pointer.child;
        const Field = @tagName(runtime.EnvironmentField.allocator);
        if (R == void or !@hasField(R, Field)) @compileError("SchemaIssue requires the caller allocator in R");
        if (@FieldType(R, Field) != std.mem.Allocator) @compileError("R.allocator must be std.mem.Allocator");
        if (self.allocator == null) self.allocator = env.allocator;
    }

    pub fn custom(env: anytype, direction: Direction, comptime identifier: []const u8) Issue {
        if (identifier.len == 0) @compileError("a custom SchemaIssue needs a stable identifier");
        return init(env, direction, .{ .custom = identifier });
    }

    /// Takes ownership of an allocator-owned list of already structured
    /// issues. A reached limit says more input may remain unchecked.
    pub fn many(env: anytype, direction: Direction, issues: []Issue, limit_reached: bool) Issue {
        var result = init(env, direction, .multiple);
        result.related = issues;
        result.limit_reached = limit_reached;
        return result;
    }

    pub fn deinit(self: *Issue) void {
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

    pub fn clone(self: *const Issue) error{OutOfMemory}!Issue {
        var result: Issue = .{ .direction = self.direction, .kind = self.kind, .allocator = self.allocator, .limit_reached = self.limit_reached };
        errdefer result.deinit();
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
    pub fn prependField(self: *Issue, name: []const u8) PathError!void {
        const allocator = self.allocator orelse return error.MissingAllocator;
        const copied_name = try allocator.dupe(u8, name);
        errdefer allocator.free(copied_name);
        try self.prepend(.{ .field = .{ .items = copied_name, .allocator = allocator } });
    }

    pub fn prependIndex(self: *Issue, index: usize) PathError!void {
        try self.prepend(.{ .index = index });
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

    pub fn at(self: *const Issue, index: usize) ?*const PathElement {
        const elements = self.path orelse return null;
        return if (index < elements.len) &elements[index] else null;
    }
};
