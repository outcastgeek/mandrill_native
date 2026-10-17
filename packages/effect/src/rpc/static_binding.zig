//! Retain the exact original static handler in runtime registry metadata.
const effects = @import("../effect.zig");
const protocol = @import("protocol.zig");

pub fn Binding(comptime source: anytype) type {
    const H = @TypeOf(source);
    const Op = @TypeOf(source.operation);
    const R = H.Environment;
    protocol.requireDefinition(Op);
    const unary = @hasDecl(Op, "Success");
    if (H != (if (unary) protocol.UnaryHandler(Op, R) else protocol.StreamHandler(Op, R))) @compileError("rpcStatic requires an existing static unary or stream handler");
    source.operation.validate();
    return struct {
        pub const Descriptor = Op;
        pub const Environment = R;
        pub const static_rpc_binding = struct {
            pub const handler = source;
        };
        pub const Result = if (unary) effects.Effect(Op.Success, Op.DomainError, R) else effects.Stream(Op.Event, Op.DomainError, R);
        comptime operation: Op = source.operation,
        pub fn invoke(_: *const @This(), request: *const Op.Request) Result {
            if (!unary) @compileError("a streaming operation needs the streaming invocation path");
            return source.invoke(request);
        }
        pub fn invokeWith(_: *const @This(), context: *const protocol.Context, request: *const Op.Request, env: *const R) Result {
            return invokeStatic(context, request, env);
        }
        pub fn openStatic(context: *const protocol.Context, request: *const Op.Request, env: *const R) Result {
            if (unary) @compileError("a unary operation needs the unary invocation path");
            return source.openWith(context.*, request, env);
        }
        pub fn invokeStatic(context: *const protocol.Context, request: *const Op.Request, env: *const R) Result {
            if (!unary) @compileError("a streaming operation needs the streaming invocation path");
            return source.invokeWith(context, request, env);
        }
    };
}
pub fn from(comptime source: anytype) Binding(source) {
    return .{};
}
pub fn isCanonical(comptime T: type) bool {
    if (@typeInfo(T) != .@"struct" or !@hasDecl(T, "static_rpc_binding")) return false;
    return T == Binding(T.static_rpc_binding.handler);
}
