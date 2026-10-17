//! Named cases infer application payloads; wire identities stay explicit and
//! the existing Variant schema remains the validation/execution authority.
const std = @import("std");
const variants = @import("../schema/variant_schema.zig");
const outcomes = @import("../outcome.zig");
const roles = @import("roles.zig");
const ownership = @import("ownership.zig");

fn Child(comptime descriptor: anytype) type {
    roles.validate(@TypeOf(descriptor), "variant.case", &.{ "tag", "codec" }, &.{});
    const C = if (@TypeOf(descriptor.codec) == type) descriptor.codec else @TypeOf(descriptor.codec);
    if (!@hasDecl(C, "Decoded")) @compileError("variant case requires a completed typed codec");
    ownership.validate(C.Decoded);
    if (ownership.containsOwned(C.Decoded) and !outcomes.ownsPayload(C.Decoded)) @compileError("variant owning case requires a declared or generated codec ownership contract");
    return C;
}
fn Value(comptime supplied: anytype) type {
    const shape = @typeInfo(@TypeOf(supplied.cases));
    if (shape != .@"struct" or shape.@"struct".is_tuple) @compileError("variant.cases requires named cases with explicit tags and codecs");
    var types: [shape.@"struct".field_names.len]type = undefined;
    for (shape.@"struct".field_names, 0..) |name, i| types[i] = Child(@field(supplied.cases, name)).Decoded;
    const Layout = @Struct(.auto, null, shape.@"struct".field_names, &types, &@splat(.{}));
    const Union = @Union(.auto, std.meta.FieldEnum(Layout), shape.@"struct".field_names, &types, &@splat(.{}));
    const T = if (roles.has(roles.recordType(supplied), "value")) roles.get("value", supplied) else Union;
    if (@TypeOf(T) != type) @compileError("variant.value anchor must be a type");
    ownership.validate(T);
    return ownership.Moved(T);
}
fn namedCase(comptime name: []const u8, comptime descriptor: anytype) @TypeOf(if (@TypeOf(descriptor.codec) == type) variants.nestedCase(name, descriptor.tag, descriptor.codec) else variants.taggedCase(name, descriptor.tag, descriptor.codec)) {
    _ = Child(descriptor);
    if (comptime @TypeOf(descriptor.codec) == type) return variants.nestedCase(name, descriptor.tag, descriptor.codec);
    return variants.taggedCase(name, descriptor.tag, descriptor.codec);
}
fn Cases(comptime cases: anytype) type {
    const names = @typeInfo(@TypeOf(cases)).@"struct".field_names;
    var types: [names.len]type = undefined;
    for (names, 0..) |name, i| types[i] = @TypeOf(namedCase(name, @field(cases, name)));
    return @Tuple(&types);
}
fn namedCases(comptime cases: anytype) Cases(cases) {
    var result: Cases(cases) = undefined;
    inline for (@typeInfo(@TypeOf(cases)).@"struct".field_names, 0..) |name, i| result[i] = namedCase(name, @field(cases, name));
    return result;
}

/// Generate the tagged application layout from named codecs while retaining
/// explicit discriminator/payload keys and each case's stable wire tag.
pub fn from(comptime R: type, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "variant", &.{ "cases", "discriminator", "payload" }, &.{"value"});
    const shape = @typeInfo(@TypeOf(supplied.cases));
    const count = if (shape == .@"struct") shape.@"struct".field_names.len else 0;
    @setEvalBranchQuota(3000 + count * count * 384);
    const T = Value(supplied);
    return variants.Variant(T, R, supplied.discriminator, supplied.payload, namedCases(supplied.cases));
}
