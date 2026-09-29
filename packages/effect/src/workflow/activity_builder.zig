//! Completed activity declarations pair codecs, body and stable identity.
const effects = @import("../effect.zig");
const activities = @import("activity.zig");

pub fn ActivityBuilder(comptime E: type, comptime R: type) type {
    return struct {
        pub const Body = effects.EffectBuilder(E, R);

        pub fn succeed(value: anytype) effects.Effect(@TypeOf(value), E, R) {
            return Body.succeed(value);
        }

        pub fn fail(comptime A: type, err: E) effects.Effect(A, E, R) {
            return Body.fail(A, err);
        }

        pub fn bind(comptime program: anytype, comptime next: anytype) @TypeOf(Body.bind(program, next)) {
            return Body.bind(program, next);
        }

        pub fn returnFrom(program: anytype) @TypeOf(Body.returnFrom(program)) {
            return Body.returnFrom(program);
        }

        pub fn delay(comptime factory: anytype) @TypeOf(Body.delay(factory)) {
            return Body.delay(factory);
        }

        pub fn combine(comptime first: effects.Effect(void, E, R), comptime second: anytype) @TypeOf(Body.combine(first, second)) {
            return Body.combine(first, second);
        }

        pub fn begin(comptime name: []const u8, comptime version: u32) type {
            return struct {
                pub fn payload(comptime payload_codec: anytype) type {
                    const PayloadCodec = @TypeOf(payload_codec);
                    return struct {
                        pub fn success(comptime success_codec: anytype) type {
                            const SuccessCodec = @TypeOf(success_codec);
                            return struct {
                                pub fn failure(comptime error_codec: anytype) type {
                                    const ErrorCodec = @TypeOf(error_codec);
                                    if (!@hasDecl(ErrorCodec, "Decoded") or ErrorCodec.Decoded != E)
                                        @compileError("activity error codec must match the builder's typed error channel");
                                    const Definition = activities.Activity(PayloadCodec, SuccessCodec, ErrorCodec);
                                    const defined = Definition.define(name, version, payload_codec, success_codec, error_codec);
                                    return struct {
                                        pub const ActivityDefinition = Definition;
                                        pub const definition = defined;

                                        pub fn handledBy(
                                            comptime handler: fn (*const Definition.Payload, *const R) effects.Effect(Definition.Success, E, R),
                                        ) activities.Handler(Definition, R) {
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
            return activities.Registry(handlers);
        }
    };
}
