//! Managed socket roles lower to the existing connection cleanup kernel.
const effects = @import("../effect.zig");
const sockets = @import("../websocket.zig");
const outcomes = @import("../outcome.zig");
const inference = @import("inference.zig");
const roles = @import("roles.zig");
const ownership = @import("ownership.zig");

fn Factory(comptime T: type, comptime R: type, comptime supplied: anytype, comptime connecting: bool) type {
    roles.validate(roles.recordType(supplied), "socket", if (connecting) &.{ "options", "body" } else &.{ "limits", "acquire", "body" }, &.{"map_error"});
    const Out = inference.result(roles.get("body", supplied), &.{ *const T, *sockets.Connection, *const R }, "socket.body");
    inference.requireEffect(Out, "socket.body");
    if (Out.Environment != R) @compileError("socket.body environment mismatch; use an explicit projection");
    inline for (.{ Out.Success, Out.Error }) |A| {
        ownership.validate(A);
        if (ownership.containsOwned(A) and !outcomes.ownsPayload(A)) @compileError("socket.body owning result/error channels require declared payload ownership before composition");
    }
    const has_mapper = roles.has(roles.recordType(supplied), "map_error");
    if (has_mapper) {
        const mapper = roles.get("map_error", supplied);
        const arity = inference.info(mapper, "socket.map_error").param_types.len;
        const Mapped = if (arity == 3) inference.result(mapper, &.{ *const T, sockets.Error, *const R }, "socket.map_error") else inference.continuation(mapper, sockets.Error, R, "socket.map_error");
        if (Mapped != Out.Error) @compileError("socket.map_error must return the body Effect error type");
    } else if (Out.Error != sockets.Error) @compileError("socket requires map_error when the body uses a different error type");
    const Config = if (connecting) sockets.ConnectOptions else sockets.Limits;
    const config_role = if (connecting) "options" else "limits";
    const selected = roles.get(config_role, supplied);
    const static_config = @TypeOf(selected) == Config;
    if (static_config) {
        const limits = if (connecting) selected.limits else selected;
        if (sockets.validate(limits) != .success) @compileError("socket static limits are invalid; supply explicit compatible positive bounds");
    } else if (inference.result(selected, &.{ *const T, *const R }, "socket.configure") != Config) @compileError("socket configuration role must return the existing Limits or ConnectOptions type");
    const Native = effects.Effect(sockets.NativeSocket, sockets.Error, R);
    if (!connecting and inference.result(roles.get("acquire", supplied), &.{ *const T, *const R }, "socket.acquire") != Native) @compileError("socket.acquire requires exact Effect(NativeSocket,WebSocket.Error,R)");
    return struct {
        pub const Result = Out;
        fn configure(inputs: *const T, current: *const R) Config {
            return if (comptime static_config) selected else selected(inputs, current);
        }
        fn acquire(inputs: *const T, current: *const R) Native {
            return roles.get("acquire", supplied)(inputs, current);
        }
        fn body(inputs: *const T, connection: *sockets.Connection, current: *const R) Out {
            return roles.get("body", supplied)(inputs, connection, current);
        }
        fn map(inputs: *const T, err: sockets.Error, current: *const R) Out.Error {
            if (comptime has_mapper) {
                const mapper = roles.get("map_error", supplied);
                if (comptime inference.info(mapper, "socket.map_error").param_types.len == 3) return mapper(inputs, err, current);
                return inference.call(mapper, err, current);
            } else return err;
        }
        fn make(env: *const R, inputs: T) Out {
            if (comptime connecting) return effects.EffectWebSocket(R).withConnectionCaptured(env, inputs, configure, body, map);
            return effects.EffectWebSocket(R).withSocketCaptured(env, inputs, configure, acquire, body, map);
        }
    };
}

pub fn withSocket(env: anytype, inputs: anytype, comptime supplied: anytype) Factory(@TypeOf(inputs), @typeInfo(@TypeOf(env)).pointer.child, supplied, false).Result {
    return Factory(@TypeOf(inputs), @typeInfo(@TypeOf(env)).pointer.child, supplied, false).make(env, inputs);
}
pub fn withConnection(env: anytype, inputs: anytype, comptime supplied: anytype) Factory(@TypeOf(inputs), @typeInfo(@TypeOf(env)).pointer.child, supplied, true).Result {
    return Factory(@TypeOf(inputs), @typeInfo(@TypeOf(env)).pointer.child, supplied, true).make(env, inputs);
}
