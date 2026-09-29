//! Staged, compile-time RPC declaration. Operations enter a protocol only
//! after their schemas and typed handlers have been paired.
const effects = @import("../effect.zig");
const rpc = @import("protocol.zig");

pub fn ProtocolBuilder(comptime E: type, comptime R: type) type {
    return struct {
        pub const Body = effects.EffectBuilder(E, R);

        fn UnaryReady(comptime Operation: type, comptime defined: Operation) type {
            return struct {
                pub const definition = defined;
                pub const OperationType = Operation;

                pub fn idempotent(comptime select_key: fn (*const Operation.Request) []const u8) type {
                    return UnaryReady(Operation, defined.withIdempotency(select_key));
                }

                pub fn handledBy(
                    comptime handler: fn (*const Operation.Request, *const R) effects.Effect(Operation.Success, E, R),
                ) rpc.UnaryHandler(Operation, R) {
                    return defined.handledBy(R, handler);
                }

                pub fn handledByContext(
                    comptime handler: fn (rpc.Context, *const Operation.Request, *const R) effects.Effect(Operation.Success, E, R),
                ) rpc.UnaryHandler(Operation, R) {
                    return defined.handledByContext(R, handler);
                }
            };
        }

        fn StreamReady(comptime Operation: type, comptime defined: Operation) type {
            return struct {
                pub const definition = defined;
                pub const OperationType = Operation;

                pub fn idempotent(comptime select_key: fn (*const Operation.Request) []const u8) type {
                    return StreamReady(Operation, defined.withIdempotency(select_key));
                }

                pub fn handledBy(
                    comptime handler: fn (*const Operation.Request, *const R) effects.Stream(Operation.Event, E, R),
                ) rpc.StreamHandler(Operation, R) {
                    return defined.handledBy(R, handler);
                }

                pub fn handledByContext(
                    comptime handler: fn (rpc.Context, *const Operation.Request, *const R) effects.Stream(Operation.Event, E, R),
                ) rpc.StreamHandler(Operation, R) {
                    return defined.handledByContext(R, handler);
                }
            };
        }

        pub fn unary(comptime name: []const u8, comptime version: u32) type {
            return struct {
                pub fn request(comptime request_codec: anytype) type {
                    const RequestCodec = @TypeOf(request_codec);
                    return struct {
                        pub fn success(comptime success_codec: anytype) type {
                            const SuccessCodec = @TypeOf(success_codec);
                            return struct {
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

        pub fn streaming(comptime name: []const u8, comptime version: u32) type {
            return struct {
                pub fn request(comptime request_codec: anytype) type {
                    const RequestCodec = @TypeOf(request_codec);
                    return struct {
                        pub fn chunk(comptime chunk_codec: anytype) type {
                            const ChunkCodec = @TypeOf(chunk_codec);
                            return struct {
                                pub fn terminal(comptime terminal_codec: anytype) type {
                                    const TerminalCodec = @TypeOf(terminal_codec);
                                    return struct {
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

        pub fn registry(comptime handlers: anytype) type {
            return rpc.Protocol(handlers);
        }
    };
}
