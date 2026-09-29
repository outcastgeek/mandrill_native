//! A completed entity pairs request, result and error codecs with its Effect
//! handler before it can enter the compile-time registry.
const effects = @import("../effect.zig");
const entities = @import("entity.zig");

pub fn EntityBuilder(comptime E: type, comptime R: type) type {
    return struct {
        /// Handler bodies use the ordinary typed Effect sequencing kernel.
        pub const Body = effects.EffectBuilder(E, R);

        pub fn begin(comptime group: []const u8, comptime version: u32) type {
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
                                        @compileError("entity error codec must match the builder's typed error channel");
                                    const Definition = entities.Entity(RequestCodec, SuccessCodec, ErrorCodec);
                                    const defined = Definition.define(group, version, request_codec, success_codec, error_codec);
                                    return struct {
                                        pub const EntityDefinition = Definition;
                                        pub const definition = defined;

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

        pub fn registry(comptime handlers: anytype) type {
            return entities.Registry(handlers);
        }
    };
}
