//! Named field declarations derive the record layout and retain the existing
//! schema/native builders as the completeness and execution authorities.
const std = @import("std");
const schemas = @import("../schema/schema_builder.zig");
const native = @import("../schema/native_codec.zig");
const Issue = @import("../schema/schema_issue.zig").Issue;
const outcomes = @import("../outcome.zig");
const inference = @import("inference.zig");
const roles = @import("roles.zig");
const ownership = @import("ownership.zig");

fn FieldMetadata(comptime descriptor: anytype) type {
    return struct {
        const presence: native.Presence = blk: {
            roles.validate(@TypeOf(descriptor), "record.field", &.{ "codec", "presence" }, &.{"default"});
            const selected: native.Presence = descriptor.presence;
            const has_default = roles.has(@TypeOf(descriptor), "default");
            if (selected == .defaulted and !has_default) @compileError("record defaulted field requires an explicit default value or factory");
            if (selected != .defaulted and has_default) @compileError("record field default conflicts with required/optional presence");
            break :blk selected;
        };
    };
}
fn fieldPresence(comptime descriptor: anytype) native.Presence {
    return FieldMetadata(descriptor).presence;
}
fn extension(comptime descriptor: anytype) bool {
    if (comptime !roles.has(@TypeOf(descriptor), "extensions")) return false;
    roles.validate(@TypeOf(descriptor), "record.extensions", &.{"extensions"}, &.{});
    if (@TypeOf(descriptor.extensions) != bool or !descriptor.extensions) @compileError("record.extensions must explicitly select preservation");
    return true;
}
fn Child(comptime descriptor: anytype, comptime is_native: bool) type {
    _ = fieldPresence(descriptor);
    const child = descriptor.codec;
    const C = if (@TypeOf(child) == type) child else @TypeOf(child);
    if (!@hasDecl(C, "Decoded")) @compileError("record field requires a completed typed codec");
    if (is_native and @TypeOf(child) == type and @hasDecl(C, "asCodec")) return @TypeOf(C.asCodec());
    if (is_native and @TypeOf(child) != type and @hasDecl(C, "asCodec")) return @TypeOf(child.asCodec());
    return C;
}
fn nativeChild(comptime descriptor: anytype) @TypeOf(if (@TypeOf(descriptor.codec) == type) descriptor.codec.asCodec() else if (@hasDecl(@TypeOf(descriptor.codec), "asCodec")) descriptor.codec.asCodec() else descriptor.codec) {
    if (comptime @TypeOf(descriptor.codec) == type) return descriptor.codec.asCodec();
    if (comptime @hasDecl(@TypeOf(descriptor.codec), "asCodec")) return descriptor.codec.asCodec();
    return descriptor.codec;
}
fn Value(comptime supplied: anytype, comptime is_native: bool) type {
    const fields = supplied.fields;
    const shape = @typeInfo(@TypeOf(fields));
    if (shape != .@"struct" or shape.@"struct".is_tuple) @compileError("record.fields requires named fields, each with explicit codec and presence");
    var types: [shape.@"struct".field_names.len]type = undefined;
    inline for (shape.@"struct".field_names, 0..) |name, index| {
        const descriptor = @field(fields, name);
        if (comptime !is_native and extension(descriptor)) {
            types[index] = schemas.UnknownFields;
        } else {
            const C = Child(descriptor, is_native);
            ownership.validate(C.Decoded);
            if (ownership.containsOwned(C.Decoded) and !outcomes.ownsPayload(C.Decoded)) @compileError("record field owning decoded values require a declared or generated codec ownership contract");
            types[index] = if (fieldPresence(descriptor) == .optional) ?C.Decoded else C.Decoded;
        }
    }
    const Generated = @Struct(.auto, null, shape.@"struct".field_names, &types, &@splat(.{}));
    const T = if (roles.has(roles.recordType(supplied), "value")) roles.get("value", supplied) else Generated;
    if (@TypeOf(T) != type) @compileError("record.value anchor must be a type");
    ownership.validate(T);
    return ownership.Moved(T);
}
fn adapted(comptime Expected: type, value: anytype) Expected {
    if (comptime Expected == @TypeOf(value)) return value;
    if (comptime Expected == ownership.Moved(@TypeOf(value))) return ownership.move(value);
    return @as(Expected, value);
}
fn Default(comptime Expected: type, comptime R: type, comptime selected: anytype) type {
    const function = @typeInfo(@TypeOf(selected)) == .@"fn" or (@typeInfo(@TypeOf(selected)) == .pointer and @typeInfo(@typeInfo(@TypeOf(selected)).pointer.child) == .@"fn");
    const Result = if (function) blk: {
        const f = inference.info(selected, "record.default");
        break :blk inference.result(selected, if (f.param_types.len == 0) &.{} else &.{*const R}, "record.default");
    } else @TypeOf(selected);
    if (!function and ownership.containsOwned(Result)) @compileError("record owning defaults require a fresh factory");
    if (function and (inference.isEffect(Result) or @typeInfo(Result) == .error_union)) @compileError("record.default requires a native value or exact Exit; map native errors explicitly");
    if (function and inference.isExit(Result) and Result.Error != Issue) @compileError("record.default Exit must retain the schema Issue channel");
    const Payload = if (function and inference.isExit(Result)) Result.Success else Result;
    const generated = Expected == ownership.Moved(Payload);
    if (Payload != Expected and !generated) {
        const Check = struct {
            fn accepts(_: Expected) void {}
        };
        // Unevaluated argument coercion asks Zig to check actual compatibility.
        // Constants retain literal/range coercions; factories are never called.
        if (function) {
            _ = @TypeOf(Check.accepts(@as(Payload, undefined)));
        } else {
            _ = @TypeOf(Check.accepts(selected));
        }
    }
    return struct {
        fn create(env: *const R) outcomes.Exit(Expected, Issue) {
            if (comptime !function) {
                return .{ .success = adapted(Expected, selected) };
            }
            const f = inference.info(selected, "record.default");
            const value = if (comptime f.param_types.len == 0) selected() else selected(env);
            if (comptime inference.isExit(Result)) {
                return if (value == .success) .{ .success = adapted(Expected, value.success) } else outcomes.castFailure(Expected, value);
            }
            return .{ .success = adapted(Expected, value) };
        }
    };
}
fn schemaField(comptime name: []const u8, comptime descriptor: anytype, comptime R: type) @TypeOf(blk: {
    if (comptime extension(descriptor)) break :blk schemas.extensions(name);
    const C = Child(descriptor, false);
    const nested = @TypeOf(descriptor.codec) == type;
    break :blk switch (comptime fieldPresence(descriptor)) {
        .required => if (nested) schemas.nestedRequired(name, descriptor.codec) else schemas.required(name, descriptor.codec),
        .optional => if (nested) schemas.nestedOptional(name, descriptor.codec) else schemas.optional(name, descriptor.codec),
        .defaulted => if (nested) schemas.nestedDefaulted(name, descriptor.codec, Default(C.Decoded, R, descriptor.default).create) else schemas.defaulted(name, descriptor.codec, Default(C.Decoded, R, descriptor.default).create),
    };
}) {
    if (comptime extension(descriptor)) return schemas.extensions(name);
    const C = Child(descriptor, false);
    const nested = @TypeOf(descriptor.codec) == type;
    return switch (comptime fieldPresence(descriptor)) {
        .required => if (nested) schemas.nestedRequired(name, descriptor.codec) else schemas.required(name, descriptor.codec),
        .optional => if (nested) schemas.nestedOptional(name, descriptor.codec) else schemas.optional(name, descriptor.codec),
        .defaulted => if (nested) schemas.nestedDefaulted(name, descriptor.codec, Default(C.Decoded, R, descriptor.default).create) else schemas.defaulted(name, descriptor.codec, Default(C.Decoded, R, descriptor.default).create),
    };
}
fn SchemaFields(comptime fields: anytype, comptime R: type) type {
    const names = @typeInfo(@TypeOf(fields)).@"struct".field_names;
    var types: [names.len]type = undefined;
    for (names, 0..) |name, i| types[i] = @TypeOf(schemaField(name, @field(fields, name), R));
    return @Tuple(&types);
}
fn schemaFields(comptime fields: anytype, comptime R: type) SchemaFields(fields, R) {
    var result: SchemaFields(fields, R) = undefined;
    inline for (@typeInfo(@TypeOf(fields)).@"struct".field_names, 0..) |name, i| result[i] = schemaField(name, @field(fields, name), R);
    return result;
}

/// Derive logical fields and ownership, then use the existing staged JSON
/// builder to validate completeness and the explicit record policy.
pub fn schema(comptime R: type, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "schemaRecord", &.{ "fields", "policy" }, &.{"value"});
    const shape = @typeInfo(@TypeOf(supplied.fields));
    const count = if (shape == .@"struct") shape.@"struct".field_names.len else 0;
    // The existing builder checks all field pairs. Cover that work and named
    // normalization with a budget derived from this declaration's actual size.
    @setEvalBranchQuota(2000 + count * count * 256);
    const T = Value(supplied, false);
    const definitions = schemaFields(supplied.fields, R);
    const policy: schemas.RecordPolicy = if (@TypeOf(supplied.policy) == schemas.RecordPolicy) supplied.policy else blk: {
        roles.validate(@TypeOf(supplied.policy), "schemaRecord.policy", &.{ "unknown_fields", "duplicate_fields", "issues" }, &.{"max_issues"});
        break :blk .{ .unknown_fields = supplied.policy.unknown_fields, .duplicate_fields = supplied.policy.duplicate_fields, .issues = supplied.policy.issues, .max_issues = if (comptime roles.has(@TypeOf(supplied.policy), "max_issues")) supplied.policy.max_issues else null };
    };
    return schemas.Builder(T, R).begin().fields(definitions).policy(policy).build();
}

fn anchor(comptime supplied: anytype, comptime name: []const u8) ?type {
    if (comptime !roles.has(roles.recordType(supplied), name)) return null;
    const selected = roles.get(name, supplied);
    if (@TypeOf(selected) != type) @compileError("nativeRecord environment anchor must be a type");
    return selected;
}
fn mergeEnvironment(comptime selected: ?type, comptime next: type) ?type {
    if (next == void) return selected;
    if (selected) |previous| if (previous != next) @compileError("nativeRecord has conflicting direction environments; use explicit typed codec adapters");
    return next;
}
fn callbackEnvironment(comptime callback: anytype, comptime operation: []const u8) ?type {
    const info = inference.info(callback, operation);
    if (info.param_types.len == 0) return null;
    const P = info.param_types[0] orelse return null;
    if (@typeInfo(P) != .pointer or @typeInfo(P).pointer.size != .one or !@typeInfo(P).pointer.attrs.@"const") @compileError(operation ++ " requires a borrowed const environment pointer");
    return @typeInfo(P).pointer.child;
}
fn directionEnvironment(comptime supplied: anytype, comptime O: type, comptime decoding: bool) type {
    var selected = anchor(supplied, if (decoding) "decode_env" else "encode_env");
    for (@typeInfo(@TypeOf(supplied.fields)).@"struct".field_names) |name| {
        const descriptor = @field(supplied.fields, name);
        const C = Child(descriptor, true);
        selected = mergeEnvironment(selected, if (decoding) C.DecodingEnvironment else C.EncodingEnvironment);
        if (decoding and fieldPresence(descriptor) == .defaulted) {
            const info = @typeInfo(@TypeOf(descriptor.default));
            if (info == .@"fn" or (info == .pointer and @typeInfo(info.pointer.child) == .@"fn")) {
                if (callbackEnvironment(descriptor.default, "nativeRecord.default")) |D| selected = mergeEnvironment(selected, D);
            }
        }
    }
    if (!decoding) if (callbackEnvironment(O.init, "nativeRecord.object.init")) |W| {
        selected = mergeEnvironment(selected, W);
    };
    return selected orelse @compileError("nativeRecord direction environment is ambiguous; supply decode_env or encode_env for the direction without service evidence");
}
fn nativeField(comptime name: []const u8, comptime descriptor: anytype, comptime D: type) @TypeOf(blk: {
    const child = comptime nativeChild(descriptor);
    break :blk switch (comptime fieldPresence(descriptor)) {
        .required => native.required(name, child),
        .optional => native.optional(name, child),
        .defaulted => native.defaulted(name, child, Default(@TypeOf(child).Representation, D, descriptor.default).create),
    };
}) {
    const child = comptime nativeChild(descriptor);
    return switch (comptime fieldPresence(descriptor)) {
        .required => native.required(name, child),
        .optional => native.optional(name, child),
        .defaulted => native.defaulted(name, child, Default(@TypeOf(child).Representation, D, descriptor.default).create),
    };
}
fn NativeFields(comptime fields: anytype, comptime D: type) type {
    const names = @typeInfo(@TypeOf(fields)).@"struct".field_names;
    var types: [names.len]type = undefined;
    for (names, 0..) |name, i| types[i] = @TypeOf(nativeField(name, @field(fields, name), D));
    return @Tuple(&types);
}
fn nativeFields(comptime fields: anytype, comptime D: type) NativeFields(fields, D) {
    var result: NativeFields(fields, D) = undefined;
    inline for (@typeInfo(@TypeOf(fields)).@"struct".field_names, 0..) |name, i| result[i] = nativeField(name, @field(fields, name), D);
    return result;
}

/// Derive native fields and independent directional services from actual child
/// codecs, defaults and the object contract. Ambiguous services need anchors.
pub fn nativeRecord(comptime O: type, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "nativeRecord", &.{ "fields", "reject_unknown" }, &.{ "value", "decode_env", "encode_env" });
    const shape = @typeInfo(@TypeOf(supplied.fields));
    const count = if (shape == .@"struct") shape.@"struct".field_names.len else 0;
    @setEvalBranchQuota(3000 + count * count * 384);
    const T = Value(supplied, true);
    const D = directionEnvironment(supplied, O, true);
    const W = directionEnvironment(supplied, O, false);
    const definitions = nativeFields(supplied.fields, D);
    return native.Builder(T, O, D, W).fields(definitions).build(supplied.reject_unknown);
}
