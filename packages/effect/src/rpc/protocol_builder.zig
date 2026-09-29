//! Staged, compile-time RPC declaration. Operations enter a protocol only
//! after their schemas and typed handlers have been paired.
const effects = @import("../effect.zig");
const rpc = @import("protocol.zig");

/// Staged unary/streaming RPC builder. Each operation receives request,
/// success/chunk, and error codecs before a matching handler can join the
/// closed registry. Transport and execution remain supplied through `R`.
///
/// Parameters:
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): staged unary and streaming protocol builder with handlers fixed to error `E` and environment `R`.
pub fn ProtocolBuilder(comptime E: type, comptime R: type) type {
    return struct {
        /// Effect builder used to compose this operation body.
        pub const Body = effects.EffectBuilder(E, R);

        fn UnaryReady(comptime Operation: type, comptime defined: Operation) type {
            return struct {
                /// Completed definition carried by this builder or handler.
                pub const definition = defined;
                /// Typed RPC operation bound by this handler.
                pub const OperationType = Operation;

                /// Adds a typed idempotency-key selector without changing the request codec.
                ///
                /// Parameters:
                /// - `select_key: comptime fn (*const Operation.Request) []const u8`: Compile-time typed selector from the current environment; its
                ///   result must remain valid through use.
                /// Returns (`type`): the same completed operation with the request-derived idempotency key attached.
                pub fn idempotent(comptime select_key: fn (*const Operation.Request) []const u8) type {
                    return UnaryReady(Operation, defined.withIdempotency(select_key));
                }

                /// Completes an operation with a handler matching its request, result, error, and R.
                ///
                /// Parameters:
                /// - `handler: comptime fn (*const Operation.Request, *const R) effects.Effect(Operation.Success, E, R)`: Compile-time typed callback; its signature is checked at comptime
                ///   where static.
                /// Returns (`rpc.UnaryHandler(Operation, R)`): A completed typed handler binding its operation to the selected environment.
                pub fn handledBy(
                    comptime handler: fn (*const Operation.Request, *const R) effects.Effect(Operation.Success, E, R),
                ) rpc.UnaryHandler(Operation, R) {
                    return defined.handledBy(R, handler);
                }

                /// Completes an operation with a handler that also receives RPC metadata.
                ///
                /// Parameters:
                /// - `handler: comptime fn (rpc.Context, *const Operation.Request, *const R) effects.Effect(Operation.Success, E, R)`: Compile-time typed callback; its signature is checked at comptime
                ///   where static.
                /// Returns (`rpc.UnaryHandler(Operation, R)`): A completed typed handler binding its operation to the selected environment.
                pub fn handledByContext(
                    comptime handler: fn (rpc.Context, *const Operation.Request, *const R) effects.Effect(Operation.Success, E, R),
                ) rpc.UnaryHandler(Operation, R) {
                    return defined.handledByContext(R, handler);
                }
            };
        }

        fn StreamReady(comptime Operation: type, comptime defined: Operation) type {
            return struct {
                /// Completed definition carried by this builder or handler.
                pub const definition = defined;
                /// Typed RPC operation bound by this handler.
                pub const OperationType = Operation;

                /// Adds a typed idempotency-key selector without changing the request codec.
                ///
                /// Parameters:
                /// - `select_key: comptime fn (*const Operation.Request) []const u8`: Compile-time typed selector from the current environment; its
                ///   result must remain valid through use.
                /// Returns (`type`): the same completed streaming operation with its request-derived idempotency key attached.
                pub fn idempotent(comptime select_key: fn (*const Operation.Request) []const u8) type {
                    return StreamReady(Operation, defined.withIdempotency(select_key));
                }

                /// Completes an operation with a handler matching its request, result, error, and R.
                ///
                /// Parameters:
                /// - `handler: comptime fn (*const Operation.Request, *const R) effects.Stream(Operation.Event, E, R)`: Compile-time typed callback; its signature is checked at comptime
                ///   where static.
                /// Returns (`rpc.StreamHandler(Operation, R)`): A completed typed handler binding its operation to the selected environment.
                pub fn handledBy(
                    comptime handler: fn (*const Operation.Request, *const R) effects.Stream(Operation.Event, E, R),
                ) rpc.StreamHandler(Operation, R) {
                    return defined.handledBy(R, handler);
                }

                /// Completes an operation with a handler that also receives RPC metadata.
                ///
                /// Parameters:
                /// - `handler: comptime fn (rpc.Context, *const Operation.Request, *const R) effects.Stream(Operation.Event, E, R)`: Compile-time typed callback; its signature is checked at comptime
                ///   where static.
                /// Returns (`rpc.StreamHandler(Operation, R)`): A completed typed handler binding its operation to the selected environment.
                pub fn handledByContext(
                    comptime handler: fn (rpc.Context, *const Operation.Request, *const R) effects.Stream(Operation.Event, E, R),
                ) rpc.StreamHandler(Operation, R) {
                    return defined.handledByContext(R, handler);
                }
            };
        }

        /// Begins a versioned unary operation; request, result, error, and handler remain required.
        ///
        /// Parameters:
        /// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition
        ///   time.
        /// - `version: comptime u32`: Compile-time stable version used to reject incompatible encoded or
        ///   replayed data.
        /// Returns (`type`): unary declaration stage that requires request, success, and error codecs before a handler can be attached.
        pub fn unary(comptime name: []const u8, comptime version: u32) type {
            return struct {
                /// Supplies the request codec; its decoded type fixes the handler input.
                ///
                /// Parameters:
                /// - `request_codec: comptime anytype`: Compile-time typed codec/schema fixing the corresponding
                ///   decoded and encoded value types.
                /// Returns (`type`): unary stage retaining the request codec and requiring a success codec.
                pub fn request(comptime request_codec: anytype) type {
                    const RequestCodec = @TypeOf(request_codec);
                    return struct {
                        /// Supplies the unary success codec; its decoded type fixes the handler result.
                        ///
                        /// Parameters:
                        /// - `success_codec: comptime anytype`: Compile-time typed codec/schema fixing the
                        ///   corresponding decoded and encoded value types.
                        /// Returns (`type`): unary stage retaining request and success codecs and requiring an error codec.
                        pub fn success(comptime success_codec: anytype) type {
                            const SuccessCodec = @TypeOf(success_codec);
                            return struct {
                                /// Supplies an error codec whose decoded type must equal the builder error type.
                                ///
                                /// Parameters:
                                /// - `error_codec: comptime anytype`: Compile-time typed codec/schema fixing the
                                ///   corresponding decoded and encoded value types.
                                /// Returns (`type`): completed unary operation stage; `handledBy` or `handledByContext` attaches its typed handler.
                                pub fn failure(comptime error_codec: anytype) type {
                                    const ErrorCodec = @TypeOf(error_codec);
                                    if (!@hasDecl(ErrorCodec, "Decoded") or ErrorCodec.Decoded != E)
                                        @compileError("unary RPC error codec must match the builder's typed error channel");
                                    const Operation = rpc.Unary(RequestCodec, SuccessCodec, ErrorCodec);
                                    return UnaryReady(Operation, Operation.init(name, version, request_codec, success_codec, error_codec));
                                }
                            };
                        }
                    };
                }
            };
        }

        /// Begins a versioned stream operation with distinct chunk and terminal codecs.
        ///
        /// Parameters:
        /// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition
        ///   time.
        /// - `version: comptime u32`: Compile-time stable version used to reject incompatible encoded or
        ///   replayed data.
        /// Returns (`type`): streaming declaration stage requiring request, chunk, terminal, and error codecs before a handler can be attached.
        pub fn streaming(comptime name: []const u8, comptime version: u32) type {
            return struct {
                /// Supplies the request codec; its decoded type fixes the handler input.
                ///
                /// Parameters:
                /// - `request_codec: comptime anytype`: Compile-time typed codec/schema fixing the corresponding
                ///   decoded and encoded value types.
                /// Returns (`type`): streaming stage retaining the request codec and requiring a chunk codec.
                pub fn request(comptime request_codec: anytype) type {
                    const RequestCodec = @TypeOf(request_codec);
                    return struct {
                        /// Supplies the codec for each emitted streaming chunk.
                        ///
                        /// Parameters:
                        /// - `chunk_codec: comptime anytype`: Compile-time typed codec/schema fixing the
                        ///   corresponding decoded and encoded value types.
                        /// Returns (`type`): streaming stage retaining request and chunk codecs and requiring the terminal codec.
                        pub fn chunk(comptime chunk_codec: anytype) type {
                            const ChunkCodec = @TypeOf(chunk_codec);
                            return struct {
                                /// Supplies the codec for the stream terminal value.
                                ///
                                /// Parameters:
                                /// - `terminal_codec: comptime anytype`: Compile-time typed codec/schema fixing the
                                ///   corresponding decoded and encoded value types.
                                /// Returns (`type`): streaming stage retaining request, chunk, and terminal codecs and requiring the error codec.
                                pub fn terminal(comptime terminal_codec: anytype) type {
                                    const TerminalCodec = @TypeOf(terminal_codec);
                                    return struct {
                                        /// Supplies an error codec whose decoded type must equal the builder error type.
                                        ///
                                        /// Parameters:
                                        /// - `error_codec: comptime anytype`: Compile-time typed codec/schema fixing
                                        ///   the corresponding decoded and encoded value types.
                                        /// Returns (`type`): completed streaming operation stage; `handledBy` or `handledByContext` attaches its typed handler.
                                        pub fn failure(comptime error_codec: anytype) type {
                                            const ErrorCodec = @TypeOf(error_codec);
                                            if (!@hasDecl(ErrorCodec, "Decoded") or ErrorCodec.Decoded != E)
                                                @compileError("streaming RPC error codec must match the builder's typed error channel");
                                            const Operation = rpc.Streaming(RequestCodec, ChunkCodec, TerminalCodec, ErrorCodec);
                                            return StreamReady(Operation, Operation.init(name, version, request_codec, chunk_codec, terminal_codec, error_codec));
                                        }
                                    };
                                }
                            };
                        }
                    };
                }
            };
        }

        /// Closes a nonempty tuple of completed handlers, rejecting duplicate identities.
        ///
        /// Parameters:
        /// - `handlers: comptime anytype`: Compile-time statically inferred input checked when this operation is
        ///   instantiated.
        /// Returns (`type`): closed protocol type for the supplied completed handlers; duplicate identities fail at compilation.
        pub fn registry(comptime handlers: anytype) type {
            return rpc.Protocol(handlers);
        }
    };
}
