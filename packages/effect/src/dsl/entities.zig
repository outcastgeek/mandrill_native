//! Named entity declarations retain the existing definition/registry authority.
const effects = @import("../effect.zig");
const entities = @import("../cluster/entity.zig");
const roles = @import("roles.zig");
const inference = @import("inference.zig");
const Codec = @import("declarations.zig").codecType;
const codec = @import("declarations.zig").codec;
const environment = @import("declaration_environment.zig").environment;
fn Definition(comptime supplied: anytype, comptime handled: bool) type {
    roles.validate(roles.recordType(supplied), "entity", if (handled) &.{ "group", "version", "request", "success", "failure", "handler" } else &.{ "group", "version", "request", "success", "failure" }, if (handled) &.{"environment"} else &.{});
    return entities.Entity(Codec(roles.get("request", supplied)), Codec(roles.get("success", supplied)), Codec(roles.get("failure", supplied)));
}
fn define(comptime supplied: anytype, comptime handled: bool) Definition(supplied, handled) {
    return Definition(supplied, handled).define(roles.get("group", supplied), roles.get("version", supplied), codec(roles.get("request", supplied)), codec(roles.get("success", supplied)), codec(roles.get("failure", supplied)));
}
pub fn definition(comptime supplied: anytype) Definition(supplied, false) {
    return define(supplied, false);
}
fn Declaration(comptime supplied: anytype) type {
    const declared = define(supplied, true);
    const D = @TypeOf(declared);
    const handler = roles.get("handler", supplied);
    const R = environment(handler, supplied, .{ .{ .codec = declared.request, .decoding = true }, .{ .codec = declared.success, .decoding = false }, .{ .codec = declared.failure, .decoding = false } }, "entity.handler", false);
    const Out = effects.Effect(D.Success, D.DomainError, R);
    if (inference.result(handler, &.{ *const D.Request, *const R }, "entity.handler") != Out) @compileError("entity.handler requires the definition's exact Effect(Success,Error,R)");
    return struct {
        pub const Result = entities.Handler(D, R);
        fn invoke(request: *const D.Request, env: *const R) Out {
            return handler(request, env);
        }
        fn make() Result {
            return declared.handledBy(R, invoke);
        }
    };
}
pub fn from(comptime supplied: anytype) Declaration(supplied).Result {
    return Declaration(supplied).make();
}
