//! Inferred runtime activity inputs reuse the native record/replay authority.
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const roles = @import("../dsl/roles.zig");
const inference = @import("../dsl/inference.zig");
const execution = @import("activity_execution.zig");
const engines = @import("workflow_engine.zig");
const activities = @import("activity.zig");
const types = @import("workflow_types.zig");
pub fn Declaration(comptime C: type, comptime R: type, comptime E: type, comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "workflow.activityCaptured", &.{ "handler", "engine", "activation", "activity", "payload", "encode_exit", "decode_exit" }, &.{"map_error"});
    const handler = roles.get("handler", supplied);
    const H = @TypeOf(handler);
    if (@typeInfo(H) != .@"struct" or !@hasDecl(H, "ActivityDefinition") or !@hasDecl(H, "Environment") or H != activities.Handler(H.ActivityDefinition, R)) @compileError("workflow.activityCaptured requires a completed activity handler with the current environment");
    handler.definition.validate();
    const D = H.ActivityDefinition;
    const Pointer = inference.result(roles.get("engine", supplied), &.{ *const C, *const R }, "workflow.activityCaptured.engine");
    if (@typeInfo(Pointer) != .pointer or @typeInfo(Pointer).pointer.size != .one) @compileError("workflow.activityCaptured.engine requires a native WorkflowEngine pointer");
    const Engine = @typeInfo(Pointer).pointer.child;
    const PE = @FieldType(Engine.Failure, "provider");
    if (Engine != engines.WorkflowEngine(R, PE, Engine.storage_guarantee)) @compileError("workflow.activityCaptured rejects foreign engines or fabricated guarantees");
    const Exit = outcomes.Exit(D.Success, D.DomainError);
    const Encoding = inference.result(roles.get("encode_exit", supplied), &.{ *const Exit, *const R }, "workflow.activityCaptured.encode_exit");
    if (!inference.isExit(Encoding) or Encoding.Success != outcomes.OwnedSlice(u8)) @compileError("workflow.activityCaptured.encode_exit requires exact Exit(OwnedSlice(u8),CodecError)");
    const CE = Encoding.Error;
    if (inference.result(roles.get("decode_exit", supplied), &.{ []const u8, *const R }, "workflow.activityCaptured.decode_exit") != outcomes.Exit(Exit, CE)) @compileError("workflow.activityCaptured.decode_exit must decode the complete declared Exit");
    const RawError = execution.RunError(Engine.Failure, CE);
    const has_mapper = roles.has(roles.recordType(supplied), "map_error");
    if (has_mapper) {
        if (inference.unary(roles.get("map_error", supplied), RawError, "workflow.activityCaptured.map_error") != E) @compileError("workflow.activityCaptured.map_error must return the current workflow error");
    } else if (RawError != E) @compileError("workflow.activityCaptured needs explicit error mapping into the current workflow error");
    return struct {
        pub const Success = execution.Recorded(D.Success, D.DomainError);
        pub const Out = effects.Effect(Success, E, R);
        fn engine(input: *const C, env: *const R) *const Engine {
            return roles.get("engine", supplied)(input, env);
        }
        fn activation(input: *const C, env: *const R) types.Activation {
            return roles.get("activation", supplied)(input, env);
        }
        fn activity(input: *const C, env: *const R) types.ActivityId {
            return roles.get("activity", supplied)(input, env);
        }
        fn payload(input: *const C, env: *const R) *const D.Payload {
            return roles.get("payload", supplied)(input, env);
        }
        fn encode(value: *const Exit, env: *const R) Encoding {
            return roles.get("encode_exit", supplied)(value, env);
        }
        fn decode(bytes: []const u8, env: *const R) outcomes.Exit(Exit, CE) {
            return roles.get("decode_exit", supplied)(bytes, env);
        }
        pub fn run(input: *const C, env: *const R) Out {
            var source = execution.runRecordedCaptured(handler, R, PE, CE, Engine.storage_guarantee, C, input, engine, activation, activity, payload, encode, decode);
            return if (comptime has_mapper) source.mapErrorOwned(env, roles.get("map_error", supplied)) else source;
        }
    };
}
