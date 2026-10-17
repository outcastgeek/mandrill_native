//! Type discovery never evaluates application callbacks. Known input evidence
//! specializes generic functions through Zig's unevaluated @TypeOf expression.
const std = @import("std");
const effects = @import("../effect.zig");

pub fn info(comptime callback: anytype, comptime operation: []const u8) std.lang.Type.Fn {
    const T = @TypeOf(callback);
    const F = switch (@typeInfo(T)) {
        .@"fn" => T,
        .pointer => |p| if (@typeInfo(p.child) == .@"fn") p.child else @compileError(operation ++ " role requires a function; actual " ++ @typeName(T)),
        else => @compileError(operation ++ " role requires a function; actual " ++ @typeName(T)),
    };
    const f = @typeInfo(F).@"fn";
    if (f.attrs.varargs) @compileError(operation ++ " role does not admit variadic callbacks");
    return f;
}

pub fn result(comptime callback: anytype, comptime Inputs: []const type, comptime operation: []const u8) type {
    const f = info(callback, operation);
    if (f.param_types.len != Inputs.len) @compileError(operation ++ " callback arity mismatch");
    inline for (f.param_types, Inputs) |actual, required| {
        if (actual != null and actual.? != required)
            @compileError(operation ++ " callback expected input " ++ @typeName(required) ++ "; actual " ++ @typeName(actual.?));
    }
    const Arguments = @Tuple(Inputs);
    return @TypeOf(@call(.auto, callback, @as(Arguments, undefined)));
}

pub fn unary(comptime callback: anytype, comptime Input: type, comptime operation: []const u8) type {
    return result(callback, &.{Input}, operation);
}

pub fn continuation(comptime callback: anytype, comptime Input: type, comptime R: type, comptime operation: []const u8) type {
    return if (info(callback, operation).param_types.len == 2)
        result(callback, &.{ Input, *const R }, operation)
    else
        unary(callback, Input, operation);
}

pub inline fn call(comptime callback: anytype, value: anytype, env: anytype) continuation(callback, @TypeOf(value), @typeInfo(@TypeOf(env)).pointer.child, "continuation") {
    if (comptime info(callback, "continuation").param_types.len == 2) return callback(value, env);
    return callback(value);
}

pub fn requireEffect(comptime T: type, comptime operation: []const u8) void {
    if (@typeInfo(T) != .@"struct" or !@hasDecl(T, "Success") or !@hasDecl(T, "Error") or !@hasDecl(T, "Environment"))
        @compileError(operation ++ " requires the exact Effect(A,E,R); actual " ++ @typeName(T));
    if (T != effects.Effect(T.Success, T.Error, T.Environment))
        @compileError(operation ++ " requires the exact Effect(A,E,R); actual " ++ @typeName(T));
}

pub fn environment(comptime callback: anytype, comptime operation: []const u8) type {
    const f = info(callback, operation);
    if (f.param_types.len == 0 or f.param_types[f.param_types.len - 1] == null)
        @compileError(operation ++ " needs an environment anchor: supply Dsl(E,R) or a typed *const R callback parameter");
    const P = f.param_types[f.param_types.len - 1].?;
    if (@typeInfo(P) != .pointer or @typeInfo(P).pointer.size != .one or !@typeInfo(P).pointer.attrs.@"const")
        @compileError(operation ++ " environment role expects *const R; actual " ++ @typeName(P));
    return @typeInfo(P).pointer.child;
}

/// Exact constructor recognition; similarly named application records are not Effects.
pub fn isEffect(comptime T: type) bool {
    return @typeInfo(T) == .@"struct" and @hasDecl(T, "Success") and @hasDecl(T, "Error") and @hasDecl(T, "Environment") and T == effects.Effect(T.Success, T.Error, T.Environment);
}
pub fn isExit(comptime T: type) bool {
    return @typeInfo(T) == .@"union" and @hasDecl(T, "Success") and @hasDecl(T, "Error") and T == @import("../outcome.zig").Exit(T.Success, T.Error);
}

/// Primary outcomes passed by value are callback-local borrows. The explicit
/// const-pointer role borrows bracket-frame storage through delayed cleanup.
pub fn primaryInput(comptime callback: anytype, comptime index: usize, comptime E: type) type {
    const f = info(callback, "resource.release");
    const Primary = @import("../outcome.zig").PrimaryOutcome(E);
    if (f.param_types.len <= index) @compileError("resource.release callback arity mismatch");
    return if (f.param_types[index] == *const Primary) *const Primary else Primary;
}
