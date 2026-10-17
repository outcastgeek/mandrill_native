//! Captured schema parsing uses the schema's existing bounded parser and the
//! shared outcome mapper. Selection and parsing run at the ordinary Effect root.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const schemas = @import("../schema/schema_builder.zig");
const Parsed = @import("../schema.zig").Parsed;
const Issue = @import("../schema/schema_issue.zig").Issue;
const inference = @import("inference.zig");
const roles = @import("roles.zig");
const ownership = @import("ownership.zig");
const declarations = @import("declarations.zig");

fn function(comptime T: type) bool {
    return @typeInfo(T) == .@"fn" or (@typeInfo(T) == .pointer and @typeInfo(@typeInfo(T).pointer.child) == .@"fn");
}
fn selectedType(comptime callback: anytype, comptime Inputs: type, comptime R: type, comptime operation: []const u8) type {
    return switch (inference.info(callback, operation).param_types.len) {
        0 => inference.result(callback, &.{}, operation),
        1 => inference.result(callback, &.{*const Inputs}, operation),
        2 => inference.result(callback, &.{ *const Inputs, *const R }, operation),
        else => @compileError(operation ++ " requires zero, input, or input/environment arguments"),
    };
}
fn select(comptime callback: anytype, inputs: anytype, env: anytype, comptime operation: []const u8) selectedType(callback, @typeInfo(@TypeOf(inputs)).pointer.child, @typeInfo(@TypeOf(env)).pointer.child, operation) {
    return switch (comptime inference.info(callback, operation).param_types.len) {
        0 => callback(),
        1 => callback(inputs),
        2 => callback(inputs, env),
        else => unreachable,
    };
}
fn policy(comptime value: anytype) schemas.ParseLimits {
    return checkedLimits(value, "parseSchema");
}
/// Shared named/typed parser policy normalization; the existing parser remains
/// the authority for dynamic validation and actual resource bounds.
pub fn checkedLimits(comptime value: anytype, comptime operation: []const u8) schemas.ParseLimits {
    const selected: schemas.ParseLimits = if (@TypeOf(value) == schemas.ParseLimits) value else blk: {
        roles.validate(@TypeOf(value), operation ++ ".limits", &.{ "max_document_bytes", "max_depth", "max_value_bytes" }, &.{"max_integer_digits"});
        break :blk .{ .max_document_bytes = value.max_document_bytes, .max_depth = value.max_depth, .max_value_bytes = value.max_value_bytes, .max_integer_digits = if (comptime roles.has(@TypeOf(value), "max_integer_digits")) value.max_integer_digits else null };
    };
    if (selected.max_document_bytes == 0 or selected.max_depth == 0 or selected.max_value_bytes == 0 or (selected.max_integer_digits != null and selected.max_integer_digits.? == 0)) @compileError(operation ++ " static limits must be positive when present");
    return selected;
}
fn Mapped(comptime callback: anytype, comptime Inputs: type, comptime R: type) type {
    const E = switch (inference.info(callback, "parseSchema.map_error").param_types.len) {
        1 => inference.result(callback, &.{Issue}, "parseSchema.map_error"),
        2 => inference.result(callback, &.{ Issue, *const R }, "parseSchema.map_error"),
        3 => inference.result(callback, &.{ *const Inputs, Issue, *const R }, "parseSchema.map_error"),
        else => @compileError("parseSchema.map_error requires Issue, Issue/environment, or input/Issue/environment arguments"),
    };
    if (inference.isEffect(E) or inference.isExit(E) or @typeInfo(E) == .error_union) @compileError("parseSchema.map_error requires a native error payload; adapt fallible or effectful work explicitly");
    ownership.validate(E);
    return ownership.Moved(E);
}
fn Definition(comptime Env: type, comptime Inputs: type, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "parseSchema", &.{ "schema", "text", "limits" }, &.{"map_error"});
    const S = roles.get("schema", supplied);
    if (@TypeOf(S) != type) @compileError("parseSchema.schema requires a completed structural schema type");
    if (@typeInfo(S) != .@"struct" or !@hasDecl(S, "Decoded") or !@hasDecl(S, "Environment") or !@hasDecl(S, "parseBytesDirect")) @compileError("parseSchema.schema requires a completed structural schema type");
    const R = S.Environment;
    if (@typeInfo(Env) != .pointer or @typeInfo(Env).pointer.size != .one or @typeInfo(Env).pointer.child != R) @compileError("parseSchema construction environment must match the selected schema environment");
    if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator) @compileError("parseSchema requires the schema environment allocator");
    const A = Parsed(S.Decoded);
    if (@TypeOf(S.parseBytesDirect(@as(*const R, undefined), @as([]const u8, undefined), @as(schemas.ParseLimits, undefined))) != outcomes.Exit(A, Issue)) @compileError("parseSchema.schema parsing must return exact Exit(Parsed(Decoded),Issue)");
    const text = roles.get("text", supplied);
    const Text = selectedType(text, Inputs, R, "parseSchema.text");
    const CheckText = struct {
        fn accepts(_: []const u8) void {}
    };
    _ = @TypeOf(CheckText.accepts(@as(Text, undefined)));
    const limits = roles.get("limits", supplied);
    const dynamic_limits = function(@TypeOf(limits));
    const static_limits: ?schemas.ParseLimits = if (dynamic_limits) null else policy(limits);
    if (dynamic_limits) {
        if (selectedType(limits, Inputs, R, "parseSchema.limits") != schemas.ParseLimits) @compileError("parseSchema.limits callback must return the existing ParseLimits type");
    }
    const has_mapper = roles.has(roles.recordType(supplied), "map_error");
    const E = if (has_mapper) Mapped(roles.get("map_error", supplied), Inputs, R) else Issue;
    const Out = effects.Effect(A, E, R);
    const Capture = ownership.Capture(Inputs);
    return struct {
        pub const Result = Out;
        const Context = struct { inputs: *const Inputs, env: *const R };
        fn map(context: Context, issue: Issue) E {
            if (comptime !has_mapper) return issue;
            const callback = roles.get("map_error", supplied);
            const result = switch (comptime inference.info(callback, "parseSchema.map_error").param_types.len) {
                1 => callback(issue),
                2 => callback(issue, context.env),
                3 => callback(context.inputs, issue, context.env),
                else => unreachable,
            };
            return ownership.move(result);
        }
        fn execute(inputs: *const Inputs, env: *const R) outcomes.Exit(A, E) {
            const bytes: []const u8 = select(text, inputs, env, "parseSchema.text");
            const selected = if (comptime dynamic_limits) select(limits, inputs, env, "parseSchema.limits") else static_limits.?;
            const parsed = S.parseBytesDirect(env, bytes, selected);
            if (comptime !has_mapper) return parsed;
            return outcomes.mapErrorCaptured(E, parsed, env.allocator, Context{ .inputs = inputs, .env = env }, map);
        }
        fn run(capture: *const Capture, env: *const R) outcomes.Exit(A, E) {
            return execute(&capture.value, env);
        }
        fn staticRun(env: *const R) outcomes.Exit(A, E) {
            return execute(&@as(void, {}), env);
        }
        fn make(env: Env, inputs: Inputs) Out {
            if (comptime Inputs == void) return Out.fromOutcome(staticRun);
            return Out.fromOwned(env, Capture{ .value = inputs }, run);
        }
    };
}

/// Capture runtime input/configuration once. Text and explicit parse limits
/// are selected lazily; successful Parsed values own their copied document.
pub fn parse(env: anytype, inputs: anytype, comptime supplied: anytype) Definition(@TypeOf(env), @TypeOf(inputs), supplied).Result {
    return Definition(@TypeOf(env), @TypeOf(inputs), supplied).make(env, inputs);
}

/// Consume one declaration record with exact static metadata and runtime
/// inputs; the same parser/role validation and generated capture are reused.
pub fn declared(env: anytype, input: anytype) Definition(@TypeOf(env), declarations.Info(if (@TypeOf(input) == type) input else @TypeOf(input), "parseDeclaration").Inputs, declarations.Info(if (@TypeOf(input) == type) input else @TypeOf(input), "parseDeclaration").definition).Result {
    const Info = declarations.Info(if (@TypeOf(input) == type) input else @TypeOf(input), "parseDeclaration");
    return Definition(@TypeOf(env), Info.Inputs, Info.definition).make(env, roles.get("inputs", input));
}
