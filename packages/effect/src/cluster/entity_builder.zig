//! A completed entity pairs request, result and error codecs with its Effect
//! handler before it can enter the compile-time registry.
const effects = @import("../effect.zig");
const entities = @import("entity.zig");

/// Staged logical-entity definition. A versioned group, codecs, and typed
/// handler must be paired before registry completion. Physical ownership and
/// message storage are independent capabilities supplied through `R`.
///
/// Parameters:
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): staged entity builder that pairs codecs and a typed handler before registration.
pub fn EntityBuilder(comptime E: type, comptime R: type) type {
    return struct {
        /// Handler bodies use the ordinary typed Effect sequencing kernel.
        pub const Body = effects.EffectBuilder(E, R);

        /// Begins a versioned logical entity group independent of physical placement.
        ///
        /// Parameters:
        /// - `group: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition
        ///   time.
        /// - `version: comptime u32`: Compile-time stable version used to reject incompatible encoded or
        ///   replayed data.
        /// Returns (`type`): versioned entity stage requiring request, success, and failure codecs.
        pub fn begin(comptime group: []const u8, comptime version: u32) type {
            return struct {
                /// Supplies the codec defining the entity request type.
                ///
                /// Parameters:
                /// - `request_codec: comptime anytype`: Compile-time typed codec/schema fixing the corresponding
                ///   decoded and encoded value types.
                /// Returns (`type`): entity stage carrying the request codec and requiring its success codec.
                pub fn request(comptime request_codec: anytype) type {
                    const RequestCodec = @TypeOf(request_codec);
                    return struct {
                        /// Supplies the codec defining the successful reply type.
                        ///
                        /// Parameters:
                        /// - `success_codec: comptime anytype`: Compile-time typed codec/schema fixing the
                        ///   corresponding decoded and encoded value types.
                        /// Returns (`type`): entity stage carrying request and success codecs and requiring its failure codec.
                        pub fn success(comptime success_codec: anytype) type {
                            const SuccessCodec = @TypeOf(success_codec);
                            return struct {
                                /// Supplies the codec defining the expected domain error type.
                                ///
                                /// Parameters:
                                /// - `error_codec: comptime anytype`: Compile-time typed codec/schema fixing the
                                ///   corresponding decoded and encoded value types.
                                /// Returns (`type`): completed entity definition stage ready for a matching typed handler.
                                pub fn failure(comptime error_codec: anytype) type {
                                    const ErrorCodec = @TypeOf(error_codec);
                                    if (!@hasDecl(ErrorCodec, "Decoded") or ErrorCodec.Decoded != E)
                                        @compileError("entity error codec must match the builder's typed error channel");
                                    const Definition = entities.Entity(RequestCodec, SuccessCodec, ErrorCodec);
                                    const defined = Definition.define(group, version, request_codec, success_codec, error_codec);
                                    return struct {
                                        /// Completed typed entity definition carried by this handler.
                                        pub const EntityDefinition = Definition;
                                        /// Completed definition carried by this builder or handler.
                                        pub const definition = defined;

                                        /// Completes the entity with a typed Effect handler using R.
                                        ///
                                        /// Parameters:
                                        /// - `handler: comptime fn (*const Definition.Request, *const R) effects.Effect(Definition.Success, E, R)`: Compile-time typed callback; its signature
                                        ///   is checked at comptime where static.
                                        /// Returns (`entities.Handler(Definition, R)`): A completed typed handler binding its operation to the selected environment.
                                        pub fn handledBy(
                                            comptime handler: fn (*const Definition.Request, *const R) effects.Effect(Definition.Success, E, R),
                                        ) entities.Handler(Definition, R) {
                                            return defined.handledBy(R, handler);
                                        }
                                    };
                                }
                            };
                        }
                    };
                }
            };
        }

        /// Closes the typed entity handler set and rejects duplicate group/version pairs.
        ///
        /// Parameters:
        /// - `handlers: comptime anytype`: Compile-time statically inferred input checked when this operation is
        ///   instantiated.
        /// Returns (`type`): registry type containing the completed entity definitions with checked identities.
        pub fn registry(comptime handlers: anytype) type {
            return entities.Registry(handlers);
        }
    };
}
