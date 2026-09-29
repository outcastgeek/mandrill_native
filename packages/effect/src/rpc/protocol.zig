//! Closed typed RPC definitions. A request is paired with its own result,
//! domain error and handler before a protocol can be completed.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const ProtocolError = @import("rpc_error.zig").Protocol;
const keys = @import("../primary_key.zig");
const envelope = @import("envelope.zig");
const codec_contract = @import("../schema/internal/codec_contract.zig");

/// Borrowed application metadata and correlation for one invocation. Encoded
/// handlers receive it from their request envelope; direct calls use `empty`.
pub const Context = struct {
    correlation: ?envelope.Correlation,
    metadata: []const envelope.View.Entry,

    pub const empty: Context = .{ .correlation = null, .metadata = &.{} };
};

/// Reserved transport-neutral metadata key for a validated PrimaryKey value.
pub const idempotency_metadata_key = "effect-idempotency-key";

pub fn Unary(comptime RequestCodec: type, comptime SuccessCodec: type, comptime ErrorCodec: type) type {
    inline for (.{ RequestCodec, SuccessCodec, ErrorCodec }) |C| codec_contract.require(C);
    return struct {
        const Self = @This();
        pub const Request = RequestCodec.Decoded;
        pub const Success = SuccessCodec.Decoded;
        pub const DomainError = ErrorCodec.Decoded;
        pub const Mode = enum { unary };

        name: []const u8,
        version: u32,
        request: RequestCodec,
        success: SuccessCodec,
        failure: ErrorCodec,
        idempotency_key: ?*const fn (*const Request) []const u8 = null,

        pub fn init(
            comptime name: []const u8,
            comptime version: u32,
            comptime request: RequestCodec,
            comptime success: SuccessCodec,
            comptime failure: ErrorCodec,
        ) Self {
            const operation: Self = .{ .name = name, .version = version, .request = request, .success = success, .failure = failure };
            operation.validate();
            return operation;
        }

        pub fn validate(comptime self: Self) void {
            if (comptime self.name.len == 0) @compileError("an RPC operation needs a stable name");
            if (comptime !std.unicode.utf8ValidateSlice(self.name)) @compileError("RPC operation names must be UTF-8");
        }

        pub fn withIdempotency(comptime self: Self, comptime select_key: fn (*const Request) []const u8) Self {
            var result = self;
            result.idempotency_key = select_key;
            return result;
        }

        pub fn key(self: Self, request_value: *const Request) outcomes.Result(?[]const u8, ProtocolError) {
            const selected = self.idempotency_key orelse return .{ .ok = null };
            const encoded = selected(request_value);
            if (keys.inspectEncoded(encoded) == .err) return .{ .err = .invalid_idempotency_key };
            return .{ .ok = encoded };
        }

        pub fn handledBy(
            comptime self: Self,
            comptime R: type,
            comptime handler: fn (*const Request, *const R) effects.Effect(Success, DomainError, R),
        ) UnaryHandler(Self, R) {
            return self.handledByContext(R, struct {
                fn invoke(_: Context, request_value: *const Request, env: *const R) effects.Effect(Success, DomainError, R) {
                    return handler(request_value, env);
                }
            }.invoke);
        }

        pub fn handledByContext(
            comptime self: Self,
            comptime R: type,
            comptime handler: fn (Context, *const Request, *const R) effects.Effect(Success, DomainError, R),
        ) UnaryHandler(Self, R) {
            self.validate();
            return .{ .operation = self, .handler = handler };
        }
    };
}

pub fn UnaryHandler(comptime Operation: type, comptime R: type) type {
    return struct {
        const Self = @This();
        pub const Descriptor = Operation;
        pub const Environment = R;
        operation: Operation,
        handler: *const fn (Context, *const Operation.Request, *const R) effects.Effect(Operation.Success, Operation.DomainError, R),

        pub fn call(comptime self: Self, comptime select_request: fn (*const R) *const Operation.Request) effects.Effect(Operation.Success, Operation.DomainError, R) {
            self.operation.validate();
            const Program = effects.Effect(Operation.Success, Operation.DomainError, R);
            return Program.deferEffect(struct {
                fn start(env: *const R) Program {
                    return self.handler(Context.empty, select_request(env), env);
                }
            }.start);
        }

        /// In-process invocation keeps the original request constructor and
        /// borrows its value through the complete handler Effect.
        pub fn invoke(comptime self: Self, request_value: *const Operation.Request) effects.Effect(Operation.Success, Operation.DomainError, R) {
            self.operation.validate();
            const Program = effects.Effect(Operation.Success, Operation.DomainError, R);
            return Program.deferBorrowed(Operation.Request, request_value, struct {
                fn start(input: *const Operation.Request, env: *const R) Program {
                    return self.handler(Context.empty, input, env);
                }
            }.start);
        }

        pub fn invokeWith(
            comptime self: Self,
            context: *const Context,
            request_value: *const Operation.Request,
            env: *const R,
        ) effects.Effect(Operation.Success, Operation.DomainError, R) {
            self.operation.validate();
            const Program = effects.Effect(Operation.Success, Operation.DomainError, R);
            const Input = struct { context: *const Context, request_value: *const Operation.Request };
            const InputEffect = effects.Effect(Input, Operation.DomainError, R);
            var selected = InputEffect.fromOwnedSuccess(env, .{ .context = context, .request_value = request_value });
            return selected.flatMapOwned(env, struct {
                fn start(input: Input, supplied: *const R) Program {
                    return self.handler(input.context.*, input.request_value, supplied);
                }
            }.start);
        }
    };
}

/// Streaming uses its own descriptor and typed handler shape. A stream chunk
/// cannot be registered as the unary success of an otherwise similar request.
pub fn Streaming(comptime RequestCodec: type, comptime ChunkCodec: type, comptime TerminalCodec: type, comptime ErrorCodec: type) type {
    inline for (.{ RequestCodec, ChunkCodec, TerminalCodec, ErrorCodec }) |C| codec_contract.require(C);
    return struct {
        const Self = @This();
        pub const Request = RequestCodec.Decoded;
        pub const Chunk = ChunkCodec.Decoded;
        pub const Terminal = TerminalCodec.Decoded;
        pub const DomainError = ErrorCodec.Decoded;
        pub const Mode = enum { streaming };
        pub const Event = union(enum) {
            pub const effect_owned_payload = true;
            chunk: Chunk,
            terminal: Terminal,

            pub fn deinit(self: *@This()) void {
                switch (self.*) {
                    .chunk => outcomes.discardPayload(&self.chunk),
                    .terminal => outcomes.discardPayload(&self.terminal),
                }
                self.* = undefined;
            }

            pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
                return switch (self.*) {
                    .chunk => |value| .{ .chunk = try outcomes.clonePayload(value) },
                    .terminal => |value| .{ .terminal = try outcomes.clonePayload(value) },
                };
            }
        };

        name: []const u8,
        version: u32,
        request: RequestCodec,
        chunk: ChunkCodec,
        terminal: TerminalCodec,
        failure: ErrorCodec,
        idempotency_key: ?*const fn (*const Request) []const u8 = null,

        pub fn init(
            comptime name: []const u8,
            comptime version: u32,
            comptime request: RequestCodec,
            comptime chunk: ChunkCodec,
            comptime terminal: TerminalCodec,
            comptime failure: ErrorCodec,
        ) Self {
            const operation: Self = .{ .name = name, .version = version, .request = request, .chunk = chunk, .terminal = terminal, .failure = failure };
            operation.validate();
            return operation;
        }

        pub fn validate(comptime self: Self) void {
            if (comptime self.name.len == 0) @compileError("a streaming RPC operation needs a stable name");
            if (comptime !std.unicode.utf8ValidateSlice(self.name)) @compileError("RPC operation names must be UTF-8");
        }

        pub fn withIdempotency(comptime self: Self, comptime select_key: fn (*const Request) []const u8) Self {
            var result = self;
            result.idempotency_key = select_key;
            return result;
        }

        pub fn key(self: Self, request_value: *const Request) outcomes.Result(?[]const u8, ProtocolError) {
            const selected = self.idempotency_key orelse return .{ .ok = null };
            const encoded = selected(request_value);
            if (keys.inspectEncoded(encoded) == .err) return .{ .err = .invalid_idempotency_key };
            return .{ .ok = encoded };
        }

        pub fn handledBy(
            comptime self: Self,
            comptime R: type,
            comptime handler: fn (*const Request, *const R) effects.Stream(Event, DomainError, R),
        ) StreamHandler(Self, R) {
            return self.handledByContext(R, struct {
                fn invoke(_: Context, request_value: *const Request, env: *const R) effects.Stream(Event, DomainError, R) {
                    return handler(request_value, env);
                }
            }.invoke);
        }

        pub fn handledByContext(
            comptime self: Self,
            comptime R: type,
            comptime handler: fn (Context, *const Request, *const R) effects.Stream(Event, DomainError, R),
        ) StreamHandler(Self, R) {
            self.validate();
            return .{ .operation = self, .handler = handler };
        }
    };
}

pub fn StreamHandler(comptime Operation: type, comptime R: type) type {
    return struct {
        const Self = @This();
        pub const Descriptor = Operation;
        pub const Environment = R;
        operation: Operation,
        handler: *const fn (Context, *const Operation.Request, *const R) effects.Stream(Operation.Event, Operation.DomainError, R),

        pub fn open(comptime self: Self, request: *const Operation.Request, env: *const R) effects.Stream(Operation.Event, Operation.DomainError, R) {
            self.operation.validate();
            return self.handler(Context.empty, request, env);
        }

        pub fn openWith(
            comptime self: Self,
            context: Context,
            request: *const Operation.Request,
            env: *const R,
        ) effects.Stream(Operation.Event, Operation.DomainError, R) {
            self.operation.validate();
            return self.handler(context, request, env);
        }
    };
}

/// Only completed operation/handler pairs enter this static registry. It
/// rejects duplicate names at declaration, before any request is dispatched.
pub fn Protocol(comptime handlers: anytype) type {
    const Info = @typeInfo(@TypeOf(handlers));
    if (Info != .@"struct" or !Info.@"struct".is_tuple or handlers.len == 0)
        @compileError("a protocol needs a nonempty tuple of completed handlers");
    inline for (handlers, 0..) |handler, index| {
        const H = @TypeOf(handler);
        if (@typeInfo(H) != .@"struct" or !@hasDecl(H, "Descriptor") or
            !@hasDecl(H, "Environment") or !@hasField(H, "operation"))
            @compileError("protocol members must pair operations with handlers");
        const Operation = H.Descriptor;
        if (Operation != @TypeOf(handler.operation))
            @compileError("RPC handler descriptor must match its operation");
        if (@hasDecl(Operation, "Success")) {
            if (H != UnaryHandler(Operation, H.Environment))
                @compileError("unary RPC operation needs a completed unary handler");
        } else if (@hasDecl(Operation, "Chunk")) {
            if (H != StreamHandler(Operation, H.Environment))
                @compileError("streaming RPC operation needs a completed stream handler");
        } else {
            @compileError("RPC operation needs a typed unary or streaming descriptor");
        }
        handler.operation.validate();
        if (H.Environment != @TypeOf(handlers[0]).Environment)
            @compileError("protocol handlers need one projected environment");
        inline for (handlers, 0..) |other, earlier| {
            if (earlier < index and std.mem.eql(u8, handler.operation.name, other.operation.name) and
                handler.operation.version == other.operation.version)
                @compileError("duplicate RPC operation identity");
        }
    }
    const R = @TypeOf(handlers[0]).Environment;
    return struct {
        pub const Environment = R;

        /// Resolves a wire identity against the closed protocol. The inline
        /// registry gives the compiler a fixed dispatch set without a heap
        /// table or runtime registration step.
        pub fn resolve(name: []const u8, version: u32) outcomes.Result(usize, ProtocolError) {
            var supported: ?u32 = null;
            inline for (handlers, 0..) |handler, index| {
                if (std.mem.eql(u8, name, handler.operation.name)) {
                    if (version == handler.operation.version) return .{ .ok = index };
                    if (supported == null) supported = handler.operation.version;
                }
            }
            if (supported) |current| return .{ .err = .{ .incompatible_version = .{
                .requested = version,
                .supported = current,
            } } };
            return .{ .err = .unknown_operation };
        }

        /// A transport adapter supplies one visitor with a common encoded
        /// response type. Each branch still receives its concrete typed
        /// handler at comptime, so request/result codec pairing is preserved.
        pub fn dispatch(
            comptime Response: type,
            name: []const u8,
            version: u32,
            env: *const R,
            payload: []const u8,
            comptime visitor: anytype,
        ) outcomes.Result(Response, ProtocolError) {
            const selected = switch (resolve(name, version)) {
                .ok => |index| index,
                .err => |problem| return .{ .err = problem },
            };
            inline for (handlers, 0..) |handler, index| {
                if (selected == index) {
                    const response: Response = visitor(handler, env, payload);
                    return .{ .ok = response };
                }
            }
            unreachable;
        }

        pub fn call(
            comptime operation: anytype,
            comptime select_request: fn (*const R) *const @TypeOf(operation).Request,
        ) effects.Effect(@TypeOf(operation).Success, @TypeOf(operation).DomainError, R) {
            inline for (handlers) |handler| {
                if (@TypeOf(handler.operation) == @TypeOf(operation) and
                    std.mem.eql(u8, handler.operation.name, operation.name) and
                    handler.operation.version == operation.version)
                {
                    if (!std.meta.eql(handler.operation, operation))
                        @compileError("RPC operation identity has incompatible codec definitions");
                    if (!@hasDecl(@TypeOf(handler.operation), "Success"))
                        @compileError("a streaming operation needs the streaming call path");
                    return handler.call(select_request);
                }
            }
            @compileError("the requested RPC operation has no paired handler");
        }

        pub fn invoke(
            comptime operation: anytype,
            request_value: *const @TypeOf(operation).Request,
        ) effects.Effect(@TypeOf(operation).Success, @TypeOf(operation).DomainError, R) {
            inline for (handlers) |handler| {
                if (@TypeOf(handler.operation) == @TypeOf(operation) and
                    std.mem.eql(u8, handler.operation.name, operation.name) and
                    handler.operation.version == operation.version)
                {
                    if (!std.meta.eql(handler.operation, operation))
                        @compileError("RPC operation identity has incompatible codec definitions");
                    if (!@hasDecl(@TypeOf(handler.operation), "Success"))
                        @compileError("streaming operations need the streaming invocation path");
                    return handler.invoke(request_value);
                }
            }
            @compileError("the requested RPC operation has no paired handler");
        }

        pub fn invokeWith(
            comptime operation: anytype,
            context: *const Context,
            request_value: *const @TypeOf(operation).Request,
            env: *const R,
        ) effects.Effect(@TypeOf(operation).Success, @TypeOf(operation).DomainError, R) {
            inline for (handlers) |handler| {
                if (@TypeOf(handler.operation) == @TypeOf(operation) and
                    std.mem.eql(u8, handler.operation.name, operation.name) and
                    handler.operation.version == operation.version)
                {
                    if (!std.meta.eql(handler.operation, operation))
                        @compileError("RPC operation identity has incompatible codec definitions");
                    if (!@hasDecl(@TypeOf(handler.operation), "Success"))
                        @compileError("streaming operations need the streaming invocation path");
                    return handler.invokeWith(context, request_value, env);
                }
            }
            @compileError("the requested RPC operation has no paired handler");
        }

        pub fn open(
            comptime operation: anytype,
            request: *const @TypeOf(operation).Request,
            env: *const R,
        ) effects.Stream(@TypeOf(operation).Event, @TypeOf(operation).DomainError, R) {
            inline for (handlers) |handler| {
                if (@TypeOf(handler.operation) == @TypeOf(operation) and
                    std.mem.eql(u8, handler.operation.name, operation.name) and
                    handler.operation.version == operation.version)
                {
                    if (!std.meta.eql(handler.operation, operation))
                        @compileError("RPC operation identity has incompatible codec definitions");
                    if (!@hasDecl(@TypeOf(handler.operation), "Chunk"))
                        @compileError("a unary operation needs the unary call path");
                    return handler.open(request, env);
                }
            }
            @compileError("the requested streaming RPC operation has no paired handler");
        }
    };
}
