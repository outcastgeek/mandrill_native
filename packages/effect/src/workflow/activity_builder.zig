//! Completed activity declarations pair codecs, body and stable identity.
const effects = @import("../effect.zig");
const activities = @import("activity.zig");

/// Pairs a stable activity identity and payload/outcome codecs with an
/// ordinary typed Effect body. The workflow engine records its completed Exit
/// so replay does not rerun a completed activity.
///
/// Parameters:
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): staged activity builder pairing stable identity, codecs, and a typed handler.
pub fn ActivityBuilder(comptime E: type, comptime R: type) type {
    return struct {
        /// Effect builder used to compose this operation body.
        pub const Body = effects.EffectBuilder(E, R);

        /// Lifts a typed success into an ordinary activity Effect body.
        ///
        /// Parameters:
        /// - `value: anytype`: success value returned by this activity description.
        /// Returns (`effects.Effect(@TypeOf(value), E, R)`): activity body that succeeds with the supplied typed value.
        pub fn succeed(value: anytype) effects.Effect(@TypeOf(value), E, R) {
            return Body.succeed(value);
        }

        /// Lifts an expected typed failure into an activity Effect body.
        ///
        /// Parameters:
        /// - `A: comptime type`: Value type produced by a successful computation.
        /// - `err: E`: Failure to propagate or transform.
        /// Returns (`effects.Effect(A, E, R)`): activity body that returns the supplied expected failure.
        pub fn fail(comptime A: type, err: E) effects.Effect(A, E, R) {
            return Body.fail(A, err);
        }

        /// Sequences ordinary Effects inside an activity, preserving E and R.
        ///
        /// Parameters:
        /// - `program: comptime anytype`: Compile-time statically inferred input checked when this operation is
        ///   instantiated.
        /// - `next: comptime anytype`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`@TypeOf(Body.bind(program, next))`): The value or description with the exact type inferred from the supplied input.
        pub fn bind(comptime program: anytype, comptime next: anytype) @TypeOf(Body.bind(program, next)) {
            return Body.bind(program, next);
        }

        /// Lift an existing typed description into this builder.
        ///
        /// Parameters:
        /// - `program: anytype`: Statically inferred input checked when this operation is instantiated.
        /// Returns (`@TypeOf(Body.returnFrom(program))`): The value or description with the exact type inferred from the supplied input.
        pub fn returnFrom(program: anytype) @TypeOf(Body.returnFrom(program)) {
            return Body.returnFrom(program);
        }

        /// Defers activity-body construction until the supplied environment is available.
        ///
        /// Parameters:
        /// - `factory: comptime anytype`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`@TypeOf(Body.delay(factory))`): The value or description with the exact type inferred from the supplied input.
        pub fn delay(comptime factory: anytype) @TypeOf(Body.delay(factory)) {
            return Body.delay(factory);
        }

        /// Sequences a void Effect before another typed activity Effect.
        ///
        /// Parameters:
        /// - `first: comptime effects.Effect(void, E, R)`: First typed description, executed or evaluated before the next.
        /// - `second: comptime anytype`: Compile-time statically inferred input checked when this operation is
        ///   instantiated.
        /// Returns (`@TypeOf(Body.combine(first, second))`): The value or description with the exact type inferred from the supplied input.
        pub fn combine(comptime first: effects.Effect(void, E, R), comptime second: anytype) @TypeOf(Body.combine(first, second)) {
            return Body.combine(first, second);
        }

        /// Begins a versioned activity; codecs and handler are still required.
        ///
        /// Parameters:
        /// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition
        ///   time.
        /// - `version: comptime u32`: Compile-time stable version used to reject incompatible encoded or
        ///   replayed data.
        /// Returns (`type`): versioned activity stage requiring payload, success, and error codecs.
        pub fn begin(comptime name: []const u8, comptime version: u32) type {
            return struct {
                /// Supplies the codec for the activity input payload.
                ///
                /// Parameters:
                /// - `payload_codec: comptime anytype`: Compile-time typed codec/schema fixing the corresponding
                ///   decoded and encoded value types.
                /// Returns (`type`): activity stage retaining its input codec and requiring a success codec.
                pub fn payload(comptime payload_codec: anytype) type {
                    const PayloadCodec = @TypeOf(payload_codec);
                    return struct {
                        /// Supplies the codec for the activity success payload.
                        ///
                        /// Parameters:
                        /// - `success_codec: comptime anytype`: Compile-time typed codec/schema fixing the
                        ///   corresponding decoded and encoded value types.
                        /// Returns (`type`): activity stage retaining input and success codecs and requiring an error codec.
                        pub fn success(comptime success_codec: anytype) type {
                            const SuccessCodec = @TypeOf(success_codec);
                            return struct {
                                /// Supplies the codec for the activity expected error payload.
                                ///
                                /// Parameters:
                                /// - `error_codec: comptime anytype`: Compile-time typed codec/schema fixing the
                                ///   corresponding decoded and encoded value types.
                                /// Returns (`type`): completed activity stage ready for a matching typed handler.
                                pub fn failure(comptime error_codec: anytype) type {
                                    const ErrorCodec = @TypeOf(error_codec);
                                    if (!@hasDecl(ErrorCodec, "Decoded") or ErrorCodec.Decoded != E)
                                        @compileError("activity error codec must match the builder's typed error channel");
                                    const Definition = activities.Activity(PayloadCodec, SuccessCodec, ErrorCodec);
                                    const defined = Definition.define(name, version, payload_codec, success_codec, error_codec);
                                    return struct {
                                        /// Completed typed activity definition.
                                        pub const ActivityDefinition = Definition;
                                        /// Completed definition carried by this builder or handler.
                                        pub const definition = defined;

                                        /// Completes the activity with an ordinary typed Effect handler.
                                        ///
                                        /// Parameters:
                                        /// - `handler: comptime fn (*const Definition.Payload, *const R) effects.Effect(Definition.Success, E, R)`: Compile-time typed callback; its signature
                                        ///   is checked at comptime where static.
                                        /// Returns (`activities.Handler(Definition, R)`): A completed typed handler binding its operation to the selected environment.
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

        /// Closes a nonempty set of completed activities and rejects duplicate identities.
        ///
        /// Parameters:
        /// - `handlers: comptime anytype`: Compile-time statically inferred input checked when this operation is
        ///   instantiated.
        /// Returns (`type`): closed registry of completed activities with duplicate identities rejected.
        pub fn registry(comptime handlers: anytype) type {
            return activities.Registry(handlers);
        }
    };
}
