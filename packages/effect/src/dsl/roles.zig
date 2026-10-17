//! Checked named-role records shared by the public DSL entry points.
const std = @import("std");

pub fn has(comptime T: type, comptime name: []const u8) bool {
    return @hasField(T, name) or @hasDecl(T, name);
}

pub fn validate(comptime T: type, comptime operation: []const u8, comptime required: []const []const u8, comptime optional: []const []const u8) void {
    if (@typeInfo(T) != .@"struct") @compileError(operation ++ " requires a named role record; actual " ++ @typeName(T));
    inline for (required) |name| {
        if (!@hasField(T, name) and !@hasDecl(T, name)) @compileError(operation ++ " missing required role '" ++ name ++ "'");
    }
    inline for (@typeInfo(T).@"struct".field_names) |name_of_field| {
        var known = false;
        inline for (required ++ optional) |name| if (std.mem.eql(u8, name, name_of_field)) {
            known = true;
        };
        if (!known) @compileError(operation ++ " unknown role '" ++ name_of_field ++ "'");
    }
    inline for (@typeInfo(T).@"struct".decl_names) |name| {
        var known = false;
        inline for (required ++ optional) |permitted| if (std.mem.eql(u8, permitted, name)) {
            known = true;
        };
        if (!known) @compileError(operation ++ " unknown namespace role '" ++ name ++ "'; put helpers outside the role namespace");
    }
}

/// Type namespaces and value records share the same named role access.
pub fn get(comptime name: []const u8, supplied: anytype) if (@TypeOf(supplied) == type) @TypeOf(@field(supplied, name)) else @TypeOf(@field(supplied, name)) {
    return @field(supplied, name);
}

pub fn recordType(supplied: anytype) type {
    return if (@TypeOf(supplied) == type) supplied else @TypeOf(supplied);
}
