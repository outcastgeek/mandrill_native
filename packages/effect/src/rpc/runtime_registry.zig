//! Runtime storage uses the same closed protocol admission and identity rules.
const std = @import("std");
const protocol = @import("protocol.zig");
const storage = @import("../internal/capture_owner.zig");
const captured = @import("captured_handler.zig");
const static = @import("static_binding.zig");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const rt = @import("../internal/runtime.zig");
const ownership = @import("../dsl/ownership.zig");

fn Binding(comptime T: type) type {
    const stream_held = @import("captured_stream_handler.zig").isCanonical(T);
    const held = captured.isCanonical(T) or stream_held;
    if (held or static.isCanonical(T)) return struct {
        pub const Prototype = T;
        pub const Program = if (stream_held) effects.Stream(T.Descriptor.Event, T.Descriptor.DomainError, T.Environment) else T.Result;
        pub const wrapped = held and !stream_held;
        pub const Configuration = void;
        pub const direct_static = !held;
        pub const declaration = false;
        pub fn launch(value: *const T, context: *const protocol.Context, request: *const T.Descriptor.Request, env: *const T.Environment) Program {
            if (comptime stream_held) {
                const D = T.captured_stream_binding;
                const signature = @import("../dsl/inference.zig").info(D.Handler, "runtime RPC stream handler");
                return if (signature.param_types.len == 4)
                    D.Handler(value.capture.borrow(), if (signature.param_types[1] == *const protocol.Context) context else context.*, request, env)
                else
                    D.Handler(value.capture.borrow(), request, env);
            }
            return if (held) value.invokeWith(context.*, request) else value.invokeWith(context, request, env);
        }
        pub fn launchStatic(context: *const protocol.Context, request: *const T.Descriptor.Request, env: *const T.Environment) Program {
            if (comptime @hasDecl(T.Descriptor, "Chunk")) return T.openStatic(context, request, env);
            return T.invokeStatic(context, request, env);
        }
    };
    const D = captured.Declared(T);
    return struct {
        pub const Prototype = D.Prototype;
        pub const Program = D.Program;
        pub const wrapped = false;
        pub const Configuration = D.Configuration;
        pub const direct_static = false;
        pub const declaration = true;
        pub const launch = D.launch;
        pub const launchConfiguration = D.launchConfiguration;
    };
}

pub fn Registry(comptime Inputs: type) type {
    if (@typeInfo(Inputs) != .@"struct" or !@hasField(Inputs, "bindings")) @compileError("withRpcRegistry inputs require named bindings plus invocation data");
    const Bindings = @FieldType(Inputs, "bindings");
    const info = @typeInfo(Bindings);
    if (info != .@"struct" or !info.@"struct".is_tuple or info.@"struct".field_names.len == 0) @compileError("runtime RPC bindings require a nonempty tuple");
    const PrototypeTuple = blk: {
        var types: [info.@"struct".field_types.len]type = undefined;
        for (info.@"struct".field_types, 0..) |H, i| types[i] = Binding(H).Prototype;
        break :blk @Tuple(&types);
    };
    // Only comptime operation fields reach admission/resolution. Invocation
    // always uses stored bindings; prototype data is never executed.
    const prototypes: PrototypeTuple = undefined;
    const Authority = protocol.Protocol(prototypes);
    const Storage = storage.Owner(Inputs);
    return struct {
        const Self = @This();
        pub const effect_owned_payload = true;
        pub const Environment = Authority.Environment;
        owner: Storage,
        pub fn create(allocator: std.mem.Allocator, inputs: Inputs) error{OutOfMemory}!Self {
            return .{ .owner = try Storage.create(allocator, inputs) };
        }
        pub fn retain(self: Self) Self {
            return .{ .owner = self.owner.retain() };
        }
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return self.retain();
        }
        pub fn deinit(self: *Self) void {
            self.owner.deinit();
        }
        pub fn inputValues(self: *const Self) *const Inputs {
            return self.owner.borrow();
        }
        pub fn resolve(_: *const Self, name: []const u8, version: u32) @TypeOf(Authority.resolve(name, version)) {
            return Authority.resolve(name, version);
        }
        fn Selected(comptime operation: anytype, comptime stream: bool) type {
            inline for (info.@"struct".field_types, 0..) |H, index| {
                const B = Binding(H);
                const defined = @field(@as(B.Prototype, undefined), "operation");
                if (comptime @TypeOf(defined) == @TypeOf(operation) and std.mem.eql(u8, defined.name, operation.name) and defined.version == operation.version) {
                    if (!std.meta.eql(defined, operation)) @compileError("RPC operation identity has incompatible codec definitions");
                    if (!stream and !@hasDecl(@TypeOf(operation), "Success")) @compileError("a streaming operation needs the streaming invocation path");
                    if (stream and !@hasDecl(@TypeOf(operation), "Chunk")) @compileError("a unary operation needs the unary invocation path");
                    return struct {
                        const Bind = B;
                        const slot = index;
                    };
                }
            }
            @compileError("the requested RPC operation has no paired handler");
        }
        fn CallContext(comptime Request: type, comptime Configuration: type) type {
            return struct {
                pub const effect_owned_payload = true;
                registry: Storage,
                // Immutable borrowed view. Registry storage owns its members.
                configuration: Configuration,
                context: protocol.Context,
                request: union(enum) { borrowed: *const Request, value: Request },
                fn pointer(self: *const @This()) *const Request {
                    return switch (self.request) {
                        .borrowed => |value| value,
                        .value => |*value| value,
                    };
                }
                pub fn deinit(self: *@This()) void {
                    switch (self.request) {
                        .value => |*value| ownership.discard(value),
                        .borrowed => {},
                    }
                    self.registry.deinit();
                }
            };
        }
        fn Invocation(comptime operation: anytype) type {
            const Op = @TypeOf(operation);
            const Selection = Selected(operation, false);
            const B = Selection.Bind;
            const P = B.Program;
            const Context = CallContext(Op.Request, B.Configuration);
            const CallStorage = storage.Owner(Context);
            const A = CallStorage.Held(Op.Success);
            const E = CallStorage.Held(Op.DomainError);
            const Out = effects.Effect(A, E, Environment);
            return struct {
                const Result = Out;
                const Frame = struct {
                    allocator: std.mem.Allocator,
                    parent: *Out.RuntimeAdapter.Job,
                    capture: *const CallStorage,
                    child: P.RuntimeAdapter.Job = undefined,
                    outcome: outcomes.Exit(P.Success, P.Error) = undefined,
                    fn success(owner: CallStorage, value: P.Success) A {
                        if (comptime B.wrapped) {
                            var inner = value;
                            const result = owner.keep(inner.value);
                            inner.owner.deinit();
                            return result;
                        }
                        return owner.keep(value);
                    }
                    fn failure(owner: CallStorage, value: P.Error) E {
                        if (comptime B.wrapped) {
                            var inner = value;
                            const result = owner.keep(inner.value);
                            inner.owner.deinit();
                            return result;
                        }
                        return owner.keep(value);
                    }
                    fn complete(_: *rt.State, data: ?*anyopaque) rt.Step {
                        const frame: *@This() = @ptrCast(@alignCast(data.?));
                        const mapped = outcomes.mapErrorCaptured(E, frame.outcome, frame.allocator, frame.capture.*, failure);
                        const result: outcomes.Exit(A, E) = switch (mapped) {
                            .success => |value| .{ .success = success(frame.capture.*, value) },
                            else => outcomes.castFailure(A, mapped),
                        };
                        const parent = frame.parent;
                        frame.allocator.destroy(frame);
                        return parent.adapterFinish(result);
                    }
                };
                fn start(state: *rt.State, parent: *Out.RuntimeAdapter.Job) rt.Step {
                    const cap: *const CallStorage = @ptrCast(@alignCast(parent.program.value.borrowed));
                    const allocator = state.services.allocatorValue() orelse return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
                    const frame = allocator.create(Frame) catch return parent.adapterFinish(.{ .die = outcomes.Defect.out_of_memory });
                    frame.* = .{ .allocator = allocator, .parent = parent, .capture = cap };
                    const context = cap.borrow();
                    const program = if (comptime B.direct_static)
                        B.launchStatic(&context.context, context.pointer(), parent.env)
                    else if (comptime B.declaration)
                        B.launchConfiguration(&context.configuration, &context.context, context.pointer(), parent.env)
                    else blk: {
                        const binding = &@field(context.registry.borrow().bindings, info.@"struct".field_names[Selection.slot]);
                        break :blk B.launch(binding, &context.context, context.pointer(), parent.env);
                    };
                    frame.child = .{ .program = program, .owned = program.ownership, .env = parent.env, .inherited = state.services, .sink = .{ .output = &frame.outcome, .next_step = .{ .data = frame, .advance = Frame.complete } } };
                    return frame.child.adapterStep();
                }
                fn make(owner: Storage, context: protocol.Context, request: anytype) Out {
                    const Given = @TypeOf(request);
                    const slot: @FieldType(Context, "request") = if (Given == *const Op.Request or Given == *Op.Request)
                        .{ .borrowed = request }
                    else
                        .{ .value = @as(Op.Request, request) };
                    const configuration: B.Configuration = if (comptime B.declaration) @field(owner.borrow().bindings, info.@"struct".field_names[Selection.slot]).inputs else {};
                    const call = CallStorage.create(owner.allocator(), .{ .registry = owner.retain(), .configuration = configuration, .context = context, .request = slot }) catch return Out.die(outcomes.Defect.out_of_memory);
                    var seed = effects.Effect(void, E, Environment).succeed({});
                    return seed.flatMapCapturedAllocated(owner.allocator(), A, call, struct {
                        fn next(cap: *const CallStorage, _: void, _: *const Environment) Out {
                            return .{ .value = .{ .borrowed = cap }, .start = start };
                        }
                    }.next);
                }
            };
        }
        fn StreamingInvocation(comptime operation: anytype) type {
            const Selection = Selected(operation, true);
            const B = Selection.Bind;
            const Op = @TypeOf(operation);
            const Context = CallContext(Op.Request, B.Configuration);
            const CallStorage = storage.Owner(Context);
            const Stream = effects.Stream(Op.Event, Op.DomainError, Environment);
            const Adapter = struct {
                fn build(stored: *const CallStorage, _: *const protocol.Context, _: *const Op.Request, env: *const Environment) Stream {
                    const context = stored.borrow();
                    if (comptime B.direct_static) return B.launchStatic(&context.context, context.pointer(), env);
                    if (comptime B.declaration) return B.launchConfiguration(&context.configuration, &context.context, context.pointer(), env);
                    const value = &@field(context.registry.borrow().bindings, info.@"struct".field_names[Selection.slot]);
                    return B.launch(value, &context.context, context.pointer(), env);
                }
            };
            const Handler = @import("captured_stream_handler.zig").Streaming(operation, CallStorage, Environment, Adapter.build);
            return struct {
                const Result = Handler.Open;
                fn make(owner: Storage, scope: *@import("../scope.zig").Scope(Op.DomainError), context: protocol.Context, request: anytype) Result {
                    const Given = @TypeOf(request);
                    const slot: @FieldType(Context, "request") = if (Given == *const Op.Request or Given == *Op.Request) .{ .borrowed = request } else .{ .value = @as(Op.Request, request) };
                    const configuration: B.Configuration = if (comptime B.declaration) @field(owner.borrow().bindings, info.@"struct".field_names[Selection.slot]).inputs else {};
                    const call = CallStorage.create(owner.allocator(), .{ .registry = owner.retain(), .configuration = configuration, .context = context, .request = slot }) catch return Result.die(outcomes.Defect.out_of_memory);
                    var handler = Handler.create(owner.allocator(), call) catch return Result.die(outcomes.Defect.out_of_memory);
                    const result = handler.openWith(scope, context, call.borrow().pointer());
                    handler.deinit();
                    return result;
                }
            };
        }
        /// Open a validated stream binding in its existing reader Scope(E).
        /// The scope retains registry/configuration/value-request storage until
        /// native reader finalizers have completed. Native chunk borrows keep
        /// the original Stream contract and must be consumed within that scope.
        pub fn open(self: *const Self, scope: anytype, comptime operation: anytype, request: anytype) StreamingInvocation(operation).Result {
            return self.openWith(scope, operation, protocol.Context.empty, request);
        }
        pub fn openWith(self: *const Self, scope: anytype, comptime operation: anytype, context: protocol.Context, request: anytype) StreamingInvocation(operation).Result {
            if (@TypeOf(scope) != *@import("../scope.zig").Scope(@TypeOf(operation).DomainError)) @compileError("runtime RPC stream opening requires the operation's existing Scope(Error)");
            return StreamingInvocation(operation).make(self.owner, scope, context, request);
        }
        /// Value requests move into owned call storage. Pointer requests remain
        /// caller borrows; returned views retain registry and call storage.
        pub fn invoke(self: *const Self, comptime operation: anytype, request: anytype) Invocation(operation).Result {
            return self.invokeWith(operation, protocol.Context.empty, request);
        }
        pub fn invokeWith(self: *const Self, comptime operation: anytype, context: protocol.Context, request: anytype) Invocation(operation).Result {
            return Invocation(operation).make(self.owner, context, request);
        }
    };
}
