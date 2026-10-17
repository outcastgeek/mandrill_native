//! Runtime stream configuration is retained by the existing reader scope.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const scopes = @import("../scope.zig");
const storage = @import("../internal/capture_owner.zig");
const protocol = @import("protocol.zig");
const inference = @import("../dsl/inference.zig");
const ownership = @import("../dsl/ownership.zig");

pub fn Streaming(comptime definition: anytype, comptime C: type, comptime R: type, comptime callback: anytype) type {
    const Op = @TypeOf(definition);
    protocol.requireDefinition(Op);
    definition.validate();
    if (!@hasDecl(Op, "Chunk") or !@hasDecl(Op, "Terminal")) @compileError("captured streaming handler requires a streaming operation definition");
    const info = inference.info(callback, "makeRpcHandler.handler");
    if (info.param_types.len != 3 and info.param_types.len != 4) @compileError("captured RPC streaming handler requires configuration/request/environment with optional context");
    const contextual = info.param_types.len == 4;
    const ContextInput = if (contextual and info.param_types[1] == *const protocol.Context) *const protocol.Context else protocol.Context;
    const Stream = effects.Stream(Op.Event, Op.DomainError, R);
    const Returned = inference.result(callback, if (contextual) &.{ *const C, ContextInput, *const Op.Request, *const R } else &.{ *const C, *const Op.Request, *const R }, "makeRpcHandler.handler");
    if (Returned != Stream) @compileError("captured RPC streaming handler must return the definition's exact Stream(Event,Error,R)");
    inline for (.{ Op.Event, Op.DomainError }) |T| {
        ownership.validate(T);
        if (ownership.containsOwned(T) and !outcomes.ownsPayload(T)) @compileError("captured RPC owning channels require declared ownership before Stream composition");
    }
    const Storage = storage.Owner(C);
    return struct {
        const Self = @This();
        pub const effect_owned_payload = true;
        pub const Descriptor = Op;
        pub const Environment = R;
        pub const Error = Op.DomainError;
        pub const Reader = effects.StreamReader(Op.Event, Error, R);
        pub const Open = effects.Effect(Reader, Error, R);
        pub const captured_stream_binding = struct {
            pub const Operation = definition;
            pub const Configuration = C;
            pub const Required = R;
            pub const Handler = callback;
        };
        comptime operation: Op = definition,
        capture: Storage,
        pub fn create(allocator: std.mem.Allocator, configuration: C) error{OutOfMemory}!Self {
            return .{ .capture = try Storage.create(allocator, configuration) };
        }
        pub fn retain(self: Self) Self {
            return .{ .capture = self.capture.retain() };
        }
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return self.retain();
        }
        pub fn deinit(self: *Self) void {
            self.capture.deinit();
        }
        const Call = struct {
            pub const effect_owned_payload = true;
            configuration: Storage,
            context: protocol.Context,
            request: *const Op.Request,
            scope: *scopes.Scope(Error),
            pub fn deinit(self: *@This()) void {
                self.configuration.deinit();
            }
        };
        const CallOwner = storage.Owner(Call);
        const Unit = effects.Effect(void, Error, R);
        fn admitted(_: *const CallOwner, _: *const R) Unit {
            return Unit.succeed({});
        }
        fn released(_: *const CallOwner, _: void, _: *const R) Unit {
            return Unit.succeed({});
        }
        fn start(stored: *const CallOwner, _: void, current: *const R) Open {
            const call = stored.borrow();
            const stream = if (contextual)
                callback(call.configuration.borrow(), if (ContextInput == protocol.Context) call.context else &call.context, call.request, current)
            else
                callback(call.configuration.borrow(), call.request, current);
            var opening = stream.open(call.scope);
            return opening.flatMapCapturedAllocated(stored.allocator(), Reader, stored.retain(), struct {
                fn keep(owner: *const CallOwner, reader: Reader, supplied: *const R) Open {
                    const retained = Reader.retainCaptureAllocated(owner.allocator(), reader, owner.retain()) catch return Open.die(outcomes.Defect.out_of_memory);
                    return Open.fromOwnedOutcome(supplied, .{ .success = retained });
                }
            }.keep);
        }
        /// Request and context entries are native borrows through reader scope
        /// close. The context record and configuration have generated storage.
        /// Opening must execute before closing its supplied existing Scope(E).
        pub fn openWith(self: Self, scope: *scopes.Scope(Error), context: protocol.Context, request: *const Op.Request) Open {
            const allocator = self.capture.allocator();
            const call = CallOwner.create(allocator, .{ .configuration = self.capture.retain(), .context = context, .request = request, .scope = scope }) catch return Open.die(outcomes.Defect.out_of_memory);
            const continuation = call.retain();
            // This guard registers before the callback opens native readers.
            // Reverse Scope cleanup therefore drains those readers first.
            var guard = Unit.scopeAcquireFactoryCapturedAllocated(allocator, scope, call, admitted, released);
            return guard.flatMapCapturedAllocated(allocator, Reader, continuation, start);
        }
        pub fn open(self: Self, scope: *scopes.Scope(Error), request: *const Op.Request) Open {
            return self.openWith(scope, protocol.Context.empty, request);
        }
    };
}

pub fn isCanonical(comptime T: type) bool {
    if (@typeInfo(T) != .@"struct" or !@hasDecl(T, "captured_stream_binding")) return false;
    const D = T.captured_stream_binding;
    return T == Streaming(D.Operation, D.Configuration, D.Required, D.Handler);
}
