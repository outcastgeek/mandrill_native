//! Named durable declarations reuse the existing completed staged builders.
const effects = @import("../effect.zig");
const roles = @import("roles.zig");
const inference = @import("inference.zig");
const contract = @import("../schema/internal/codec_contract.zig");
const environment = @import("declaration_environment.zig").environment;
fn CodecType(comptime selected: anytype) type {
    const C = if (@TypeOf(selected) == type) selected else @TypeOf(selected);
    contract.require(C);
    return C;
}
fn codec(comptime selected: anytype) CodecType(selected) {
    return if (@TypeOf(selected) == type) .{} else selected;
}
fn description(comptime supplied: anytype) @import("../workflow/workflow_types.zig").Definition {
    const Definition = @import("../workflow/workflow_types.zig").Definition;
    if (@TypeOf(supplied) == Definition) return supplied;
    roles.validate(roles.recordType(supplied), "workflow.description", &.{ "name", "version", "payload_schema_version", "success_schema_version", "error_schema_version" }, &.{});
    return .{
        .name = roles.get("name", supplied),
        .version = roles.get("version", supplied),
        .payload_schema_version = roles.get("payload_schema_version", supplied),
        .success_schema_version = roles.get("success_schema_version", supplied),
        .error_schema_version = roles.get("error_schema_version", supplied),
    };
}
fn Declaration(comptime supplied: anytype, comptime is_workflow: bool) type {
    const operation = if (is_workflow) "workflow" else "activity";
    roles.validate(roles.recordType(supplied), operation, if (is_workflow) &.{ "description", "payload", "success", "failure", "handler" } else &.{ "name", "version", "payload", "success", "failure", "handler" }, &.{"environment"});
    const payload = codec(roles.get("payload", supplied));
    const success = codec(roles.get("success", supplied));
    const failure = codec(roles.get("failure", supplied));
    const E = @TypeOf(failure).Decoded;
    const handler = roles.get("handler", supplied);
    if (inference.info(handler, operation ++ ".handler").param_types.len != 2) @compileError(operation ++ ".handler requires payload/environment arguments");
    const R = environment(handler, supplied, .{
        .{ .codec = payload, .decoding = true },
        .{ .codec = success, .decoding = false },
        .{ .codec = failure, .decoding = false },
    }, operation ++ ".handler", false);
    const Stage = if (is_workflow)
        @import("../workflow/workflow_builder.zig").WorkflowBuilder(E, R).begin(description(roles.get("description", supplied))).payload(payload).success(success).failure(failure)
    else
        @import("../workflow/activity_builder.zig").ActivityBuilder(E, R).begin(roles.get("name", supplied), roles.get("version", supplied)).payload(payload).success(success).failure(failure);
    const Out = effects.Effect(@TypeOf(success).Decoded, E, R);
    const Adapter = struct {
        fn invoke(input: *const @TypeOf(payload).Decoded, env: *const R) Out {
            return handler(input, env);
        }
    };
    if (!is_workflow) {
        const Actual = inference.result(handler, &.{ *const @TypeOf(payload).Decoded, *const R }, "activity.handler");
        if (Actual != Out) @compileError("activity.handler requires the exact declared Effect(Success,Error,R)");
    }
    return struct {
        const Result = if (is_workflow) @TypeOf(Stage.handledBy(handler)) else @TypeOf(Stage.handledBy(Adapter.invoke));
        fn make() Result {
            return if (is_workflow) Stage.handledBy(handler) else Stage.handledBy(Adapter.invoke);
        }
    };
}
/// Infer all activity channels and R from codecs and callback evidence.
pub fn activity(comptime supplied: anytype) Declaration(supplied, false).Result {
    return Declaration(supplied, false).make();
}
/// Accept only the exact durable Step handler through the controlled builder.
pub fn workflow(comptime supplied: anytype) Declaration(supplied, true).Result {
    return Declaration(supplied, true).make();
}
