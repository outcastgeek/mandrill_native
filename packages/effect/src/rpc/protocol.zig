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

    /// Invocation context without correlation or metadata.
    pub const empty: Context = .{ .correlation = null, .metadata = &.{} };
};

/// Reserved transport-neutral metadata key for a validated PrimaryKey value.
pub const idempotency_metadata_key = "effect-idempotency-key";

/// Define a unary RPC operation from three codecs. Each codec must satisfy
/// `codec_contract.require` at comptime; the decoded types become the request,
/// success, and domain-error channels of the operation.
/// Parameters:
/// - `RequestCodec: comptime type`: Codec for the decoded request.
/// - `SuccessCodec: comptime type`: Codec for the decoded success payload.
/// - `ErrorCodec: comptime type`: Codec for the decoded domain error.
/// Returns (`type`): A unary operation descriptor with those exact codecs and
/// a statically known wire identity and handler signature.
pub fn Unary(comptime RequestCodec: type, comptime SuccessCodec: type, comptime ErrorCodec: type) type {
    inline for (.{ RequestCodec, SuccessCodec, ErrorCodec }) |C| codec_contract.require(C);
    return struct {
        const Self = @This();
        /// Typed request payload or request record for this operation.
        pub const Request = RequestCodec.Decoded;
        /// Value type carried by the successful channel.
        pub const Success = SuccessCodec.Decoded;
        /// Domain-specific error type carried by the operation.
        pub const DomainError = ErrorCodec.Decoded;
        /// `Mode` cases: `unary`.
        pub const Mode = enum { unary };

        name: []const u8,
        version: u32,
        request: RequestCodec,
        success: SuccessCodec,
        failure: ErrorCodec,
        idempotency_key: ?*const fn (*const Request) []const u8 = null,

        /// Bind a unary operation identity to its request, success, and error codecs.
        /// Parameters:
        /// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition
        ///   time.
        /// - `version: comptime u32`: Compile-time stable version used to reject incompatible encoded or
        ///   replayed data.
        /// - `request: comptime RequestCodec`: Compile-time typed input value; move or clone owning payloads according to
        ///   its type contract.
        /// - `success: comptime SuccessCodec`: Codec or value for the successful result channel.
        /// - `failure: comptime ErrorCodec`: Codec or value for the typed failure channel.
        /// Returns (`Self`): a typed unary operation definition with its stable name and version.
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

        /// Validate the value against its declared contract.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// Returns (`void`): no value.
        pub fn validate(comptime self: Self) void {
            if (comptime self.name.len == 0) @compileError("an RPC operation needs a stable name");
            if (comptime !std.unicode.utf8ValidateSlice(self.name)) @compileError("RPC operation names must be UTF-8");
        }

        /// Attach a typed idempotency-key selector to this operation.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `select_key: comptime fn (*const Request) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`Self`): unary operation definition carrying the request-derived idempotency key selector.
        pub fn withIdempotency(comptime self: Self, comptime select_key: fn (*const Request) []const u8) Self {
            var result = self;
            result.idempotency_key = select_key;
            return result;
        }

        /// Construct or inspect the typed stable key for this definition.
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `request_value: *const Request`: Decoded request value passed to the handler.
        /// Returns (`outcomes.Result(?[]const u8, ProtocolError)`): An ok/err Result; owned payloads must be released according to their type contract.
        pub fn key(self: Self, request_value: *const Request) outcomes.Result(?[]const u8, ProtocolError) {
            const selected = self.idempotency_key orelse return .{ .ok = null };
            const encoded = selected(request_value);
            if (keys.inspectEncoded(encoded) == .err) return .{ .err = .invalid_idempotency_key };
            return .{ .ok = encoded };
        }

        /// Attach a typed handler to this definition.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `R: comptime type`: Caller-defined environment type supplying the dependencies needed by this operation.
        /// - `handler: comptime fn (*const Request, *const R) effects.Effect(Success, DomainError, R)`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`UnaryHandler(Self, R)`): A completed typed handler binding its operation to the selected environment.
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

        /// Bind a handler that receives request context and environment.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `R: comptime type`: Caller-defined environment type supplying the dependencies needed by this operation.
        /// - `handler: comptime fn (Context, *const Request, *const R) effects.Effect(Success, DomainError, R)`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`UnaryHandler(Self, R)`): A completed typed handler binding its operation to the selected environment.
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

/// Specialize the typed handler signature for rpc operations.
/// Parameters:
/// - `Operation: comptime type`: Compile-time typed operation descriptor defining request, success, and error channels.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): handler binding for one unary operation with a matching `Effect` result.
pub fn UnaryHandler(comptime Operation: type, comptime R: type) type {
    return struct {
        const Self = @This();
        /// Typed operation descriptor attached to this handler.
        pub const Descriptor = Operation;
        /// Exact caller environment type required when the description runs.
        pub const Environment = R;
        operation: Operation,
        handler: *const fn (Context, *const Operation.Request, *const R) effects.Effect(Operation.Success, Operation.DomainError, R),

        /// Describe one typed RPC call.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `select_request: comptime fn (*const R) *const Operation.Request`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`effects.Effect(Operation.Success, Operation.DomainError, R)`): typed unary call result for this completed operation.
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
        ///
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `request_value: *const Operation.Request`: Decoded request value passed to the handler.
        /// Returns (`effects.Effect(Operation.Success, Operation.DomainError, R)`): handler result for the decoded unary request.
        pub fn invoke(comptime self: Self, request_value: *const Operation.Request) effects.Effect(Operation.Success, Operation.DomainError, R) {
            self.operation.validate();
            const Program = effects.Effect(Operation.Success, Operation.DomainError, R);
            return Program.deferBorrowed(Operation.Request, request_value, struct {
                fn start(input: *const Operation.Request, env: *const R) Program {
                    return self.handler(Context.empty, input, env);
                }
            }.start);
        }

        /// Invoke this RPC operation with caller-supplied context.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `context: *const Context`: Invocation metadata passed through without changing the typed payload.
        /// - `request_value: *const Operation.Request`: Decoded request value passed to the handler.
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// Returns (`effects.Effect(Operation.Success, Operation.DomainError, R)`): handler result with supplied RPC metadata and environment.
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
/// Each codec must satisfy `codec_contract.require` at comptime.
///
/// Parameters:
/// - `RequestCodec: comptime type`: Codec whose `Decoded` type is the request.
/// - `ChunkCodec: comptime type`: Codec whose `Decoded` type is each stream chunk.
/// - `TerminalCodec: comptime type`: Codec whose `Decoded` type ends the stream.
/// - `ErrorCodec: comptime type`: Codec whose `Decoded` type is the domain error.
/// Returns (`type`): A streaming descriptor with typed chunk and terminal
/// events and a handler shape distinct from unary operations.
pub fn Streaming(comptime RequestCodec: type, comptime ChunkCodec: type, comptime TerminalCodec: type, comptime ErrorCodec: type) type {
    inline for (.{ RequestCodec, ChunkCodec, TerminalCodec, ErrorCodec }) |C| codec_contract.require(C);
    return struct {
        const Self = @This();
        /// Typed request payload or request record for this operation.
        pub const Request = RequestCodec.Decoded;
        /// Typed stream chunk or chunk container.
        pub const Chunk = ChunkCodec.Decoded;
        /// Typed terminal event for a streaming operation.
        pub const Terminal = TerminalCodec.Decoded;
        /// Domain-specific error type carried by the operation.
        pub const DomainError = ErrorCodec.Decoded;
        /// `Mode` cases: `streaming`.
        pub const Mode = enum { streaming };
        /// Typed event or event variant emitted by this definition.
        pub const Event = union(enum) {
            /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements
            /// `deinit` and must be released after use.
            pub const effect_owned_payload = true;
            chunk: Chunk,
            terminal: Terminal,

            /// Release resources owned by this value.
            /// Parameters:
            /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
            /// Returns (`void`): no value.
            pub fn deinit(self: *@This()) void {
                switch (self.*) {
                    .chunk => outcomes.discardPayload(&self.chunk),
                    .terminal => outcomes.discardPayload(&self.terminal),
                }
                self.* = undefined;
            }

            /// Duplicate this value; marked owning payloads are cloned, while ordinary
            /// borrowed fields retain their original lifetime.
            /// Parameters:
            /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
            /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
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

        /// Bind a streaming operation identity to its request, chunk, terminal, and error codecs.
        /// Parameters:
        /// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition
        ///   time.
        /// - `version: comptime u32`: Compile-time stable version used to reject incompatible encoded or
        ///   replayed data.
        /// - `request: comptime RequestCodec`: Compile-time typed input value; move or clone owning payloads according to
        ///   its type contract.
        /// - `chunk: comptime ChunkCodec`: Compile-time typed input value; move or clone owning payloads according to
        ///   its type contract.
        /// - `terminal: comptime TerminalCodec`: Terminal-event codec whose `Decoded` type ends the stream.
        /// - `failure: comptime ErrorCodec`: Codec or value for the typed failure channel.
        /// Returns (`Self`): a typed streaming operation definition with its stable name and version.
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

        /// Validate the value against its declared contract.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// Returns (`void`): no value.
        pub fn validate(comptime self: Self) void {
            if (comptime self.name.len == 0) @compileError("a streaming RPC operation needs a stable name");
            if (comptime !std.unicode.utf8ValidateSlice(self.name)) @compileError("RPC operation names must be UTF-8");
        }

        /// Attach a typed idempotency-key selector to this operation.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `select_key: comptime fn (*const Request) []const u8`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`Self`): streaming operation definition carrying the request-derived idempotency key selector.
        pub fn withIdempotency(comptime self: Self, comptime select_key: fn (*const Request) []const u8) Self {
            var result = self;
            result.idempotency_key = select_key;
            return result;
        }

        /// Construct or inspect the typed stable key for this definition.
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `request_value: *const Request`: Decoded request value passed to the handler.
        /// Returns (`outcomes.Result(?[]const u8, ProtocolError)`): An ok/err Result; owned payloads must be released according to their type contract.
        pub fn key(self: Self, request_value: *const Request) outcomes.Result(?[]const u8, ProtocolError) {
            const selected = self.idempotency_key orelse return .{ .ok = null };
            const encoded = selected(request_value);
            if (keys.inspectEncoded(encoded) == .err) return .{ .err = .invalid_idempotency_key };
            return .{ .ok = encoded };
        }

        /// Attach a typed handler to this definition.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `R: comptime type`: Caller-defined environment type supplying the dependencies needed by this operation.
        /// - `handler: comptime fn (*const Request, *const R) effects.Stream(Event, DomainError, R)`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`StreamHandler(Self, R)`): A completed typed handler binding its operation to the selected environment.
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

        /// Bind a handler that receives request context and environment.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `R: comptime type`: Caller-defined environment type supplying the dependencies needed by this operation.
        /// - `handler: comptime fn (Context, *const Request, *const R) effects.Stream(Event, DomainError, R)`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`StreamHandler(Self, R)`): A completed typed handler binding its operation to the selected environment.
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

/// Specialize the typed handler signature for rpc operations.
/// Parameters:
/// - `Operation: comptime type`: Compile-time typed operation descriptor defining request, success, and error channels.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): handler binding for one streaming operation with a matching `Stream` result.
pub fn StreamHandler(comptime Operation: type, comptime R: type) type {
    return struct {
        const Self = @This();
        /// Typed operation descriptor attached to this handler.
        pub const Descriptor = Operation;
        /// Exact caller environment type required when the description runs.
        pub const Environment = R;
        operation: Operation,
        handler: *const fn (Context, *const Operation.Request, *const R) effects.Stream(Operation.Event, Operation.DomainError, R),

        /// Open the selected scoped resource or stream.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `request: *const Operation.Request`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// Returns (`effects.Stream(Operation.Event, Operation.DomainError, R)`): A lazy typed Stream; the caller consumes it under its scope and handles terminal Exit.
        pub fn open(comptime self: Self, request: *const Operation.Request, env: *const R) effects.Stream(Operation.Event, Operation.DomainError, R) {
            self.operation.validate();
            return self.handler(Context.empty, request, env);
        }

        /// Open this RPC stream with caller-supplied context.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `context: Context`: Invocation metadata passed through without changing the typed payload.
        /// - `request: *const Operation.Request`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// Returns (`effects.Stream(Operation.Event, Operation.DomainError, R)`): A lazy typed Stream; the caller consumes it under its scope and handles terminal Exit.
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
///
/// Parameters:
/// - `handlers: comptime anytype`: A nonempty tuple of completed unary or
///   streaming handlers. Every handler must expose `Descriptor`, `Environment`,
///   and `operation`; descriptors must match, environments must agree, and
///   `(name, version)` identities must be unique.
/// Returns (`type`): A closed protocol type with a statically known dispatch
/// set; invalid handler combinations fail compilation.
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
        /// Exact caller environment type required when the description runs.
        pub const Environment = R;

        /// Resolves a wire identity against the closed protocol. The inline
        /// registry gives the compiler a fixed dispatch set without a heap
        /// table or runtime registration step.
        ///
        /// Parameters:
        /// - `name: []const u8`: Stable, nonempty identity component validated at definition time.
        /// - `version: u32`: Stable version used to reject incompatible encoded or replayed data.
        /// Returns (`outcomes.Result(usize, ProtocolError)`): An ok/err Result; owned payloads must be released according to their type contract.
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
        ///
        /// Parameters:
        /// - `Response: comptime type`: Response payload type produced by the selected operation.
        /// - `name: []const u8`: Stable, nonempty identity component validated at definition time.
        /// - `version: u32`: Stable version used to reject incompatible encoded or replayed data.
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `payload: []const u8`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// - `visitor: comptime anytype`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`outcomes.Result(Response, ProtocolError)`): An ok/err Result; owned payloads must be released according to their type contract.
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

        /// Describe one typed RPC call.
        /// Parameters:
        /// - `operation: comptime anytype`: Compile-time statically inferred input checked when this operation is
        ///   instantiated.
        /// - `select_request: comptime fn (*const R) *const @TypeOf(operation).Request`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`effects.Effect(@TypeOf(operation).Success, @TypeOf(operation).DomainError, R)`): typed streaming call result for this completed operation.
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

        /// Invoke the selected typed operation.
        /// Parameters:
        /// - `operation: comptime anytype`: Compile-time statically inferred input checked when this operation is
        ///   instantiated.
        /// - `request_value: *const @TypeOf(operation).Request`: Decoded request value passed to the handler.
        /// Returns (`effects.Effect(@TypeOf(operation).Success, @TypeOf(operation).DomainError, R)`): handler stream result for the decoded streaming request.
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

        /// Invoke this RPC operation with caller-supplied context.
        /// Parameters:
        /// - `operation: comptime anytype`: Compile-time statically inferred input checked when this operation is
        ///   instantiated.
        /// - `context: *const Context`: Invocation metadata passed through without changing the typed payload.
        /// - `request_value: *const @TypeOf(operation).Request`: Decoded request value passed to the handler.
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// Returns (`effects.Effect(@TypeOf(operation).Success, @TypeOf(operation).DomainError, R)`): handler stream result with supplied RPC metadata and environment.
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

        /// Open the selected scoped resource or stream.
        /// Parameters:
        /// - `operation: comptime anytype`: Compile-time statically inferred input checked when this operation is
        ///   instantiated.
        /// - `request: *const @TypeOf(operation).Request`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// Returns (`effects.Stream(@TypeOf(operation).Event, @TypeOf(operation).DomainError, R)`): A lazy typed Stream; the caller consumes it under its scope and handles terminal Exit.
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
