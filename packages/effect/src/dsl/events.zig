//! Event roles infer the existing definition and handler from one declaration.
const effects = @import("../effect.zig");
const events = @import("../eventlog/event.zig");
const roles = @import("roles.zig");
const inference = @import("inference.zig");
const codec = @import("declarations.zig").codec;
const Codec = @import("declarations.zig").codecType;
const environment = @import("declaration_environment.zig").environment;
pub const case = @import("../eventlog/projection_cases.zig").case;
pub const caseWith = @import("../eventlog/projection_cases.zig").caseWith;
pub const projection = @import("../eventlog/projection_cases.zig").projection;
fn Definition(comptime supplied: anytype, comptime handled: bool) type {
    roles.validate(roles.recordType(supplied), "event", if (handled) &.{ "tag", "version", "payload", "success", "failure", "handler" } else &.{ "tag", "version", "payload", "success", "failure" }, if (handled) &.{"environment"} else &.{});
    return events.Event(Codec(roles.get("payload", supplied)), Codec(roles.get("success", supplied)), Codec(roles.get("failure", supplied)));
}
fn define(comptime supplied: anytype, comptime handled: bool) Definition(supplied, handled) {
    return Definition(supplied, handled).define(roles.get("tag", supplied), roles.get("version", supplied), codec(roles.get("payload", supplied)), codec(roles.get("success", supplied)), codec(roles.get("failure", supplied)));
}
pub fn definition(comptime supplied: anytype) Definition(supplied, false) {
    return define(supplied, false);
}
fn Declaration(comptime supplied: anytype) type {
    const declared = define(supplied, true);
    const D = @TypeOf(declared);
    const handler = roles.get("handler", supplied);
    const R = environment(handler, supplied, .{ .{ .codec = declared.payload, .decoding = true }, .{ .codec = declared.success, .decoding = false }, .{ .codec = declared.failure, .decoding = false } }, "event.handler", false);
    const Out = effects.Effect(D.Success, D.DomainError, R);
    if (inference.result(handler, &.{ *const D.Payload, *const R }, "event.handler") != Out) @compileError("event.handler requires the definition's exact Effect(Success,Error,R)");
    return struct {
        pub const Result = events.Handler(D, R);
        fn invoke(payload: *const D.Payload, env: *const R) Out {
            return handler(payload, env);
        }
        fn make() Result {
            return declared.handledBy(R, invoke);
        }
    };
}
pub fn from(comptime supplied: anytype) Declaration(supplied).Result {
    return Declaration(supplied).make();
}
