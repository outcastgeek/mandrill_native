//! Captured unary handlers retain typed configuration on the existing kernel.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const rt = @import("../internal/runtime.zig");
const storage = @import("../internal/capture_owner.zig");
const protocol = @import("protocol.zig");
const inference = @import("../dsl/inference.zig");
const ownership = @import("../dsl/ownership.zig");

pub fn Unary(comptime source_operation: anytype, comptime C: type, comptime R: type, comptime callback: anytype) type {
    const Operation = @TypeOf(source_operation);
    protocol.requireDefinition(Operation);
    source_operation.validate();
    if (!@hasDecl(Operation, "Success")) @compileError("captured unary handler requires a unary operation definition");
    const info = inference.info(callback, "makeRpcHandler.handler");
    if (info.param_types.len != 3 and info.param_types.len != 4) @compileError("captured RPC handler requires configuration/request/environment with optional context");
    const contextual = info.param_types.len == 4;
    const ContextInput = if (contextual and info.param_types[1] == *const protocol.Context) *const protocol.Context else protocol.Context;
    const Body = inference.result(callback, if (contextual) &.{ *const C, ContextInput, *const Operation.Request, *const R } else &.{ *const C, *const Operation.Request, *const R }, "makeRpcHandler.handler");
    if (Body != effects.Effect(Operation.Success, Operation.DomainError, R)) @compileError("captured RPC handler must return the definition's exact Effect(Success,Error,R)");
    inline for (.{ Operation.Success, Operation.DomainError }) |T| {
        ownership.validate(T);
        if (ownership.containsOwned(T) and !outcomes.ownsPayload(T)) @compileError("captured RPC owning channels require declared ownership before Effect composition");
    }
    const Storage = storage.Owner(C);
    return struct {
        const Self = @This();
        pub const effect_owned_payload = true;
        pub const Descriptor = Operation;
        pub const Environment = R;
        pub const captured_unary_binding = struct {
            pub const definition = source_operation;
            pub const Configuration = C;
            pub const Required = R;
            pub const handler = callback;
        };
        pub const Success = Storage.Held(Operation.Success);
        pub const Error = Storage.Held(Operation.DomainError);
        pub const Result = effects.Effect(Success, Error, R);
        comptime operation: Operation = source_operation,
        capture: Storage,
        pub fn create(allocator: std.mem.Allocator, configuration: C) error{OutOfMemory}!Self {
            return .{ .capture = try Storage.create(allocator, configuration) };
        }
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return self.retain();
        }
        pub fn retain(self: Self) Self {
            return .{ .capture = self.capture.retain() };
        }
        pub fn deinit(self: *Self) void {
            self.capture.deinit();
        }
        const Capture = struct {
            pub const effect_owned_payload = true;
            owner: Storage,
            context: protocol.Context,
            request: *const Operation.Request,
            pub fn deinit(self: *@This()) void {
                self.owner.deinit();
            }
        };
        const Frame = struct {
            allocator: std.mem.Allocator,
            parent: *Result.RuntimeAdapter.Job,
            capture: *const Capture,
            child: Body.RuntimeAdapter.Job = undefined,
            outcome: outcomes.Exit(Operation.Success, Operation.DomainError) = undefined,
            fn mapError(owner: Storage, err: Operation.DomainError) Error {
                return owner.keep(err);
            }
            fn complete(_: *rt.State, data: ?*anyopaque) rt.Step {
                const frame: *@This() = @ptrCast(@alignCast(data.?));
                const mapped = outcomes.mapErrorCaptured(Error, frame.outcome, frame.allocator, frame.capture.owner, mapError);
                const result: outcomes.Exit(Success, Error) = switch (mapped) {
                    .success => |value| .{ .success = frame.capture.owner.keep(value) },
                    else => outcomes.castFailure(Success, mapped),
                };
                const parent = frame.parent;
                frame.allocator.destroy(frame);
                return parent.adapterFinish(result);
            }
        };
        fn start(state: *rt.State, parent: *Result.RuntimeAdapter.Job) rt.Step {
            const captured: *const Capture = @ptrCast(@alignCast(parent.program.value.borrowed));
            const allocator = state.services.allocatorValue() orelse return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
            const frame = allocator.create(Frame) catch return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
            frame.* = .{ .allocator = allocator, .parent = parent, .capture = captured };
            const program = if (contextual)
                callback(captured.owner.borrow(), if (ContextInput == protocol.Context) captured.context else &captured.context, captured.request, parent.env)
            else
                callback(captured.owner.borrow(), captured.request, parent.env);
            frame.child = .{ .program = program, .owned = program.ownership, .env = parent.env, .inherited = state.services, .sink = .{ .output = &frame.outcome, .next_step = .{ .data = frame, .advance = Frame.complete } } };
            return frame.child.adapterStep();
        }
        /// Borrow the request through join; returned owners retain configuration.
        pub fn invoke(self: Self, request: *const Operation.Request) Result {
            return self.invokeWith(protocol.Context.empty, request);
        }
        /// Metadata entries remain caller borrows through join. The context
        /// record itself has stable generated storage for pointer callbacks.
        pub fn invokeWith(self: Self, context: protocol.Context, request: *const Operation.Request) Result {
            var seed = effects.Effect(void, Error, R).succeed({});
            return seed.flatMapCapturedAllocated(self.capture.allocator(), Success, Capture{ .owner = self.capture.retain(), .context = context, .request = request }, struct {
                fn next(captured: *const Capture, _: void, _: *const R) Result {
                    return .{ .value = .{ .borrowed = captured }, .start = start };
                }
            }.next);
        }
    };
}

pub fn isCanonical(comptime T: type) bool {
    if (@typeInfo(T) != .@"struct" or !@hasDecl(T, "captured_unary_binding")) return false;
    const Shape = T.captured_unary_binding;
    return T == Unary(Shape.definition, Shape.Configuration, Shape.Required, Shape.handler);
}

/// A declaration/configuration pair can be stored directly by the registry;
/// canonical handler admission does not execute its callback or clone data.
pub fn Declared(comptime Entry: type) type {
    return struct {
        const Info = @import("../dsl/declarations.zig").Info(Entry, "RPC runtime binding");
        const definition = Info.definition;
        comptime {
            @import("../dsl/roles.zig").validate(@TypeOf(definition), "RPC runtime binding", &.{ "operation", "handler" }, &.{"environment"});
        }
        const operation = definition.operation;
        const Op = @TypeOf(operation);
        comptime {
            protocol.requireDefinition(Op);
        }
        const stream = @hasDecl(Op, "Chunk");
        const R = @import("../dsl/declaration_environment.zig").environment(definition.handler, definition, if (stream) .{
            .{ .codec = operation.request, .decoding = true }, .{ .codec = operation.chunk, .decoding = false }, .{ .codec = operation.terminal, .decoding = false }, .{ .codec = operation.failure, .decoding = false },
        } else .{
            .{ .codec = operation.request, .decoding = true }, .{ .codec = operation.success, .decoding = false }, .{ .codec = operation.failure, .decoding = false },
        }, "RPC runtime binding.handler", true);
        pub const Prototype = if (stream) @import("captured_stream_handler.zig").Streaming(operation, Info.Inputs, R, definition.handler) else Unary(operation, Info.Inputs, R, definition.handler);
        pub const Program = if (stream) effects.Stream(Op.Event, Op.DomainError, R) else effects.Effect(Op.Success, Op.DomainError, R);
        pub const Configuration = Info.Inputs;
        const callback_info = inference.info(definition.handler, "RPC runtime binding.handler");
        const contextual = callback_info.param_types.len == 4;
        const pointer_context = contextual and callback_info.param_types[1] == *const protocol.Context;
        pub fn launch(entry: *const Entry, context: *const protocol.Context, request: *const Op.Request, env: *const R) Program {
            return launchConfiguration(&entry.inputs, context, request, env);
        }
        pub fn launchConfiguration(configuration: *const Configuration, context: *const protocol.Context, request: *const Op.Request, env: *const R) Program {
            return if (contextual) definition.handler(configuration, if (pointer_context) context else context.*, request, env) else definition.handler(configuration, request, env);
        }
    };
}
