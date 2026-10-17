//! Named RPC declarations lower to the existing completed staged builders.
const effects = @import("../effect.zig");
const protocol = @import("../rpc/protocol.zig");
const builder = @import("../rpc/protocol_builder.zig");
const codec_contract = @import("../schema/internal/codec_contract.zig");
const roles = @import("roles.zig");
const inference = @import("inference.zig");
const std = @import("std");
const rt = @import("../internal/runtime.zig");
const outcomes = @import("../outcome.zig");
const ownership = @import("ownership.zig");
const declarations = @import("declarations.zig");
const captured_handlers = @import("../rpc/captured_handler.zig");
const runtime_registries = @import("../rpc/runtime_registry.zig");

fn CodecType(comptime selected: anytype) type {
    const C = if (@TypeOf(selected) == type) selected else @TypeOf(selected);
    codec_contract.require(C);
    return C;
}
fn codec(comptime selected: anytype) CodecType(selected) {
    return if (@TypeOf(selected) == type) .{} else selected;
}
const environment = @import("declaration_environment.zig").environment;

fn KeyAdapter(comptime Request: type, comptime select: anytype) type {
    const Returned = inference.unary(select, *const Request, "RPC.idempotency");
    const Check = struct {
        fn accepts(_: []const u8) void {}
    };
    _ = @TypeOf(Check.accepts(@as(Returned, undefined)));
    return struct {
        fn run(input: *const Request) []const u8 {
            return select(input);
        }
    };
}
fn Definition(comptime supplied: anytype, comptime is_stream: bool) type {
    const name = if (is_stream) "rpcStream" else "rpcUnary";
    roles.validate(roles.recordType(supplied), name, if (is_stream)
        &.{ "name", "version", "request", "chunk", "terminal", "failure", "handler" }
    else
        &.{ "name", "version", "request", "success", "failure", "handler" }, &.{ "idempotency", "environment" });
    const request = codec(roles.get("request", supplied));
    const failure = codec(roles.get("failure", supplied));
    const E = @TypeOf(failure).Decoded;
    const handler = roles.get("handler", supplied);
    const R = environment(handler, supplied, if (is_stream) .{
        .{ .codec = request, .decoding = true }, .{ .codec = roles.get("chunk", supplied), .decoding = false }, .{ .codec = roles.get("terminal", supplied), .decoding = false }, .{ .codec = failure, .decoding = false },
    } else .{
        .{ .codec = request, .decoding = true }, .{ .codec = roles.get("success", supplied), .decoding = false }, .{ .codec = failure, .decoding = false },
    }, name ++ ".handler", false);
    const Build = builder.ProtocolBuilder(E, R);
    const Initial = if (is_stream)
        Build.streaming(roles.get("name", supplied), roles.get("version", supplied)).request(request).chunk(codec(roles.get("chunk", supplied))).terminal(codec(roles.get("terminal", supplied))).failure(failure)
    else
        Build.unary(roles.get("name", supplied), roles.get("version", supplied)).request(request).success(codec(roles.get("success", supplied))).failure(failure);
    const Stage = if (roles.has(roles.recordType(supplied), "idempotency")) Initial.idempotent(KeyAdapter(Initial.OperationType.Request, roles.get("idempotency", supplied)).run) else Initial;
    const Operation = Stage.OperationType;
    const Expected = if (is_stream) effects.Stream(Operation.Event, E, R) else effects.Effect(Operation.Success, E, R);
    const contextual = inference.info(handler, name ++ ".handler").param_types.len == 3;
    const Returned = inference.result(handler, if (contextual) &.{ protocol.Context, *const Operation.Request, *const R } else &.{ *const Operation.Request, *const R }, name ++ ".handler");
    if (Returned != Expected) @compileError(name ++ ".handler must return the exact declared " ++ (if (is_stream) "Stream(Event,E,R)" else "Effect(Success,E,R)"));
    return struct {
        const Adapter = struct {
            fn run(context: protocol.Context, input: *const Operation.Request, env: *const R) Expected {
                return if (contextual) handler(context, input, env) else handler(input, env);
            }
        };
        pub const Result = if (is_stream) protocol.StreamHandler(Operation, R) else protocol.UnaryHandler(Operation, R);
        fn make() Result {
            return Stage.handledByContext(Adapter.run);
        }
    };
}

/// Derive unary channels from codecs and infer R from the actual handler.
pub fn unary(comptime supplied: anytype) Definition(supplied, false).Result {
    return Definition(supplied, false).make();
}
/// Keep chunk and terminal codecs distinct under the existing stream contract.
pub fn streaming(comptime supplied: anytype) Definition(supplied, true).Result {
    return Definition(supplied, true).make();
}

fn Declaration(comptime supplied: anytype, comptime stream: bool) type {
    const name = if (stream) "rpcStreamDefinition" else "rpcUnaryDefinition";
    roles.validate(roles.recordType(supplied), name, if (stream)
        &.{ "name", "version", "request", "chunk", "terminal", "failure" }
    else
        &.{ "name", "version", "request", "success", "failure" }, &.{"idempotency"});
    const request = codec(roles.get("request", supplied));
    const failure = codec(roles.get("failure", supplied));
    const Operation = if (stream)
        protocol.Streaming(@TypeOf(request), CodecType(roles.get("chunk", supplied)), CodecType(roles.get("terminal", supplied)), @TypeOf(failure))
    else
        protocol.Unary(@TypeOf(request), CodecType(roles.get("success", supplied)), @TypeOf(failure));
    const defined = if (stream)
        Operation.init(roles.get("name", supplied), roles.get("version", supplied), request, codec(roles.get("chunk", supplied)), codec(roles.get("terminal", supplied)), failure)
    else
        Operation.init(roles.get("name", supplied), roles.get("version", supplied), request, codec(roles.get("success", supplied)), failure);
    return struct {
        const value = if (roles.has(roles.recordType(supplied), "idempotency")) defined.withIdempotency(KeyAdapter(Operation.Request, roles.get("idempotency", supplied)).run) else defined;
        const Result = Operation;
    };
}
pub fn unaryDefinition(comptime supplied: anytype) Declaration(supplied, false).Result {
    return Declaration(supplied, false).value;
}
pub fn streamDefinition(comptime supplied: anytype) Declaration(supplied, true).Result {
    return Declaration(supplied, true).value;
}
fn Pair(comptime supplied: anytype) type {
    roles.validate(roles.recordType(supplied), "rpcHandled", &.{ "definition", "handler" }, &.{"environment"});
    const operation = roles.get("definition", supplied);
    const Operation = @TypeOf(operation);
    protocol.requireDefinition(Operation);
    operation.validate();
    const stream = !@hasDecl(Operation, "Success");
    const handler = roles.get("handler", supplied);
    const R = environment(handler, supplied, if (stream) .{
        .{ .codec = operation.request, .decoding = true }, .{ .codec = operation.chunk, .decoding = false }, .{ .codec = operation.terminal, .decoding = false }, .{ .codec = operation.failure, .decoding = false },
    } else .{
        .{ .codec = operation.request, .decoding = true }, .{ .codec = operation.success, .decoding = false }, .{ .codec = operation.failure, .decoding = false },
    }, "rpcHandled.handler", false);
    const Expected = if (stream) effects.Stream(Operation.Event, Operation.DomainError, R) else effects.Effect(Operation.Success, Operation.DomainError, R);
    const contextual = inference.info(handler, "rpcHandled.handler").param_types.len == 3;
    if (inference.result(handler, if (contextual) &.{ protocol.Context, *const Operation.Request, *const R } else &.{ *const Operation.Request, *const R }, "rpcHandled.handler") != Expected) @compileError("rpcHandled.handler category or channels do not match its definition");
    return struct {
        const Result = if (stream) protocol.StreamHandler(Operation, R) else protocol.UnaryHandler(Operation, R);
        fn run(context: protocol.Context, input: *const Operation.Request, env: *const R) Expected {
            return if (contextual) handler(context, input, env) else handler(input, env);
        }
        fn make() Result {
            return operation.handledByContext(R, run);
        }
    };
}
/// Pair one validated definition with its handler, deriving all channels from
/// that same definition. Stream handlers can name definition.Event directly.
pub fn handled(comptime supplied: anytype) Pair(supplied).Result {
    return Pair(supplied).make();
}

fn Captured(comptime Carrier: type) type {
    return struct {
        const Info = declarations.Info(Carrier, "makeRpcHandler");
        const supplied = Info.definition;
        comptime {
            roles.validate(roles.recordType(supplied), "makeRpcHandler", &.{ "operation", "handler" }, &.{"environment"});
        }
        const operation = roles.get("operation", supplied);
        comptime {
            protocol.requireDefinition(@TypeOf(operation));
        }
        const callback = roles.get("handler", supplied);
        const stream = @hasDecl(@TypeOf(operation), "Chunk");
        const R = environment(callback, supplied, if (stream) .{
            .{ .codec = operation.request, .decoding = true }, .{ .codec = operation.chunk, .decoding = false }, .{ .codec = operation.terminal, .decoding = false }, .{ .codec = operation.failure, .decoding = false },
        } else .{
            .{ .codec = operation.request, .decoding = true }, .{ .codec = operation.success, .decoding = false }, .{ .codec = operation.failure, .decoding = false },
        }, "makeRpcHandler.handler", true);
        const Handler = if (stream) @import("../rpc/captured_stream_handler.zig").Streaming(operation, Info.Inputs, R, callback) else captured_handlers.Unary(operation, Info.Inputs, R, callback);
    };
}
fn ConstructionEnvironment(comptime P: type) type {
    if (@typeInfo(P) != .pointer or @typeInfo(P).pointer.size != .one) @compileError("makeRpcHandler construction environment requires a borrowed pointer");
    return @typeInfo(P).pointer.child;
}

/// Own runtime handler configuration from one static/runtime declaration
/// record. Creation and invocation both expose ordinary inferred Effects.
pub fn make(env: anytype, input: anytype) effects.Effect(Captured(if (@TypeOf(input) == type) input else @TypeOf(input)).Handler, Captured(if (@TypeOf(input) == type) input else @TypeOf(input)).Handler.Error, ConstructionEnvironment(@TypeOf(env))) {
    const Carrier = if (@TypeOf(input) == type) input else @TypeOf(input);
    const Selected = Captured(Carrier);
    const R = ConstructionEnvironment(@TypeOf(env));
    const Out = effects.Effect(Selected.Handler, Selected.Handler.Error, R);
    const allocator: std.mem.Allocator = rt.services(R, env).allocatorValue() orelse {
        var abandoned = roles.get("inputs", input);
        ownership.discard(&abandoned);
        return Out.die(.{ .kind = .invalid_configuration, .message = "makeRpcHandler requires construction environment allocator" });
    };
    const owner = Selected.Handler.create(allocator, roles.get("inputs", input)) catch return Out.die(outcomes.Defect.out_of_memory);
    return Out.fromOwnedAllocated(allocator, owner, struct {
        fn run(stored: *const Selected.Handler, _: *const R) outcomes.Exit(Selected.Handler, Selected.Handler.Error) {
            return .{ .success = stored.clone() catch return .{ .die = outcomes.Defect.out_of_memory } };
        }
    }.run);
}

fn ScopedRegistry(comptime Env: type, comptime Carrier: type) type {
    return struct {
        const Info = declarations.Info(Carrier, "withRpcRegistry");
        const supplied = Info.definition;
        comptime {
            roles.validate(roles.recordType(supplied), "withRpcRegistry", &.{"use"}, &.{});
        }
        const Registry = runtime_registries.Registry(Info.Inputs);
        const R = ConstructionEnvironment(Env);
        const use = roles.get("use", supplied);
        const count = inference.info(use, "withRpcRegistry.use").param_types.len;
        const Result = if (count == 2)
            inference.result(use, &.{ *const Registry, *const R }, "withRpcRegistry.use")
        else if (count == 3)
            inference.result(use, &.{ *const Registry, *const Info.Inputs, *const R }, "withRpcRegistry.use")
        else
            @compileError("withRpcRegistry.use requires registry/environment or registry/inputs/environment arguments");
        comptime {
            inference.requireEffect(Result, "withRpcRegistry.use");
            if (Result.Environment != R) @compileError("withRpcRegistry use environment mismatch; provide an explicit projection");
        }
        fn run(registry_value: *const Registry, env: *const R) Result {
            return if (count == 2) use(registry_value, env) else use(registry_value, registry_value.inputValues(), env);
        }
    };
}
/// Move bindings and invocation data into one retained registry scope. A use
/// callback's inferred Effect controls its complete lifetime, including join.
pub fn scopedRegistry(env: anytype, input: anytype) ScopedRegistry(@TypeOf(env), if (@TypeOf(input) == type) input else @TypeOf(input)).Result {
    const Carrier = if (@TypeOf(input) == type) input else @TypeOf(input);
    const Selected = ScopedRegistry(@TypeOf(env), Carrier);
    const Out = Selected.Result;
    const allocator = rt.services(Out.Environment, env).allocatorValue() orelse {
        var abandoned = roles.get("inputs", input);
        ownership.discard(&abandoned);
        return Out.die(.{ .kind = .invalid_configuration, .message = "withRpcRegistry requires construction environment allocator" });
    };
    const registry_value = Selected.Registry.create(allocator, roles.get("inputs", input)) catch return Out.die(outcomes.Defect.out_of_memory);
    return @import("../dsl.zig").owned(env, registry_value, Selected.run);
}
pub const staticBinding = @import("../rpc/static_binding.zig").from;
pub const registry = protocol.Protocol;
