//! Named layer roles preserve the existing scoped acquisition authority.
const effects = @import("../effect.zig");
const inference = @import("inference.zig");
const roles = @import("roles.zig");
const ownership = @import("ownership.zig");

fn buildType(comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "layer", &.{"acquire"}, &.{"release"});
    const Build = @TypeOf(roles.get("acquire", supplied));
    inference.requireEffect(Build, "layer.acquire");
    if (ownership.containsOwned(Build.Success)) @compileError("layer services must be borrowed dependency records or handles; retain owning acquisitions in their Scope");
    if (roles.has(roles.recordType(supplied), "release")) {
        if (inference.continuation(roles.get("release", supplied), Build.Success, Build.Environment, "layer.release") != effects.Effect(void, Build.Error, Build.Environment)) @compileError("layer.release requires exact Effect(void,E,R)");
    }
    return Build;
}
pub fn from(comptime supplied: anytype) effects.Layer(buildType(supplied).Environment, buildType(supplied).Error, buildType(supplied).Success) {
    const Build = buildType(supplied);
    const Layer = effects.Layer(Build.Environment, Build.Error, Build.Success);
    if (comptime roles.has(roles.recordType(supplied), "release")) return Layer.scoped(roles.get("acquire", supplied), roles.get("release", supplied));
    return Layer.fromEffect(roles.get("acquire", supplied));
}

fn requireLayer(comptime T: type, comptime operation: []const u8) void {
    if (@typeInfo(T) != .@"struct" or !@hasDecl(T, "Service") or !@hasDecl(T, "Error") or !@hasDecl(T, "Environment")) @compileError(operation ++ " requires the existing Layer definition");
    if (T != effects.Layer(T.Environment, T.Error, T.Service)) @compileError(operation ++ " rejects forged Layer evidence");
    if (ownership.containsOwned(T.Service)) @compileError("layer services must be borrowed dependency records or handles");
}

fn mergedType(comptime Left: type, comptime Right: type, comptime supplied: anytype) type {
    requireLayer(Left, "mergeLayers.left");
    requireLayer(Right, "mergeLayers.right");
    roles.validate(roles.recordType(supplied), "mergeLayers", &.{"combine"}, &.{});
    if (Left.Environment != Right.Environment or Left.Error != Right.Error) @compileError("mergeLayers requires matching E/R; supply explicit error mapping and environment projections");
    const Combined = inference.result(roles.get("combine", supplied), &.{ Left.Service, Right.Service }, "mergeLayers.combine");
    if (inference.isEffect(Combined) or inference.isExit(Combined) or @typeInfo(Combined) == .error_union) @compileError("mergeLayers.combine requires a native borrowed service value");
    if (ownership.containsOwned(Combined)) @compileError("mergeLayers cannot expose owning services without their scope owner");
    return effects.Layer(Left.Environment, Left.Error, Combined);
}
pub fn merge(comptime left: anytype, comptime right: anytype, comptime supplied: anytype) mergedType(@TypeOf(left), @TypeOf(right), supplied) {
    return left.merge(right, roles.get("combine", supplied));
}

pub fn provide(comptime layer: anytype, comptime program: anytype) effects.Effect(@TypeOf(program).Success, @TypeOf(layer).Error, @TypeOf(layer).Environment) {
    comptime requireLayer(@TypeOf(layer), "provideLayer");
    comptime inference.requireEffect(@TypeOf(program), "provideLayer.program");
    return layer.provideTo(program);
}

pub fn compose(comptime first: anytype, comptime second: anytype) effects.Layer(@TypeOf(first).Environment, @TypeOf(first).Error, @TypeOf(second).Service) {
    comptime requireLayer(@TypeOf(first), "composeLayers.first");
    comptime requireLayer(@TypeOf(second), "composeLayers.second");
    return first.provide(second);
}
