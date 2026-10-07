//! Domain failures retain their typed channel and every non-fail Exit case.
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const Issue = @import("schema_issue.zig").Issue;
/// Pair lazy representation-generic callbacks with domain failure E and independent D/W environments.
/// Inputs are borrowed through join; the codec does not add an allocator or executor outside R.
pub fn DomainCodec(comptime A: type, comptime Rep: type, comptime E: type, comptime D: type, comptime W: type) type {
    return struct {
        const Self = @This();
        /// Application value produced on success; ownership follows the codec’s declared value type.
        pub const Decoded = A;
        /// Wire or provider representation consumed and produced by this codec.
        pub const Representation = Rep;
        /// Application failure type; defects, interruption and finalization failures remain separate Exit channels.
        pub const Error = E;
        /// Services needed by decoding only; encoding services are not acquired by this direction.
        pub const DecodingEnvironment = D;
        /// Services needed by encoding only; decoding services are not acquired by this direction.
        pub const EncodingEnvironment = W;
        /// Optional explicit immutable representation witness; null means no shape may be inferred from callbacks.
        encoded_shape: ?*const @import("schema_ast.zig").Node = null,
        /// Selected decode callback; borrowed input remains alive through direct use or lazy join.
        decoder: *const fn (*const Rep) effects.Effect(A, E, D),
        /// Selected encode callback; returned representation follows its declared ownership contract.
        encoder: *const fn (*const A) effects.Effect(Rep, E, W),
        /// Select checked comptime callbacks for each direction; constructing a codec does not execute them.
        pub fn init(comptime decode_: fn (*const Rep) effects.Effect(A, E, D), comptime encode_: fn (*const A) effects.Effect(Rep, E, W)) Self {
            return .{ .decoder = decode_, .encoder = encode_ };
        }
        /// Provide only decoding’s environment through a borrowed R projection; keep both environments alive until join.
        pub fn decodeIn(comptime self: Self, comptime R: type, input: *const Rep, comptime project: fn (*const R) *const Self.DecodingEnvironment) effects.Effect(A, E, R) {
            return self.decode(input).provideSome(R, project);
        }
        /// Provide only encoding’s environment through a borrowed R projection; decoding services are not required.
        pub fn encodeIn(comptime self: Self, comptime R: type, input: *const A, comptime project: fn (*const R) *const Self.EncodingEnvironment) effects.Effect(Rep, E, R) {
            return self.encode(input).provideSome(R, project);
        }
        /// Construct lazy borrowed-input decoding; repeated runs use their own environment and preserve all Exit channels.
        pub fn decode(comptime self: Self, input: *const Rep) effects.Effect(A, E, D) {
            return effects.Effect(A, E, D).deferBorrowed(Rep, input, struct {
                fn start(value: *const Rep, _: *const D) effects.Effect(A, E, D) {
                    return self.decoder(value);
                }
            }.start);
        }
        /// Construct lazy borrowed-value encoding; the callback supplies the representation and its ownership contract.
        pub fn encode(comptime self: Self, input: *const A) effects.Effect(Rep, E, W) {
            return effects.Effect(Rep, E, W).deferBorrowed(A, input, struct {
                fn start(value: *const A, _: *const W) effects.Effect(Rep, E, W) {
                    return self.encoder(value);
                }
            }.start);
        }
        /// Run encoding once and discard a marked owning success; preserve domain failures and finalization failures.
        pub fn validate(comptime self: Self, input: *const A) effects.Effect(void, E, W) {
            return effects.Effect(void, E, W).deferBorrowed(A, input, struct {
                fn start(value: *const A, env: *const W) effects.Effect(void, E, W) {
                    var program = self.encode(value);
                    return program.mapOwned(env, struct {
                        fn discard(encoded: Rep) void {
                            var owned = encoded;
                            outcomes.discardPayload(&owned);
                        }
                    }.discard);
                }
            }.start);
        }
    };
}
fn directionPure(comptime child: anytype, comptime decoding: bool) bool {
    const C = @TypeOf(child);
    const name = if (decoding) "pure_decoding" else "pure_encoding";
    const R = if (decoding) C.DecodingEnvironment else C.EncodingEnvironment;
    return @hasDecl(C, name) and @field(C, name) and R == void;
}
/// Each pure direction has an independent typed direct callback and witness.
pub fn Directed(comptime A: type, comptime Rep: type, comptime E: type, comptime D: type, comptime W: type, comptime decoding_pure: bool, comptime encoding_pure: bool) type {
    if (decoding_pure and encoding_pure) return Pure(A, Rep, E);
    return struct {
        const Self = @This();
        /// Application value produced on success; ownership follows the codec’s declared value type.
        pub const Decoded = A;
        /// Wire or provider representation consumed and produced by this codec.
        pub const Representation = Rep;
        /// Application failure type; defects, interruption and finalization failures remain separate Exit channels.
        pub const Error = E;
        /// Services needed by decoding only; encoding services are not acquired by this direction.
        pub const DecodingEnvironment = D;
        /// Services needed by encoding only; decoding services are not acquired by this direction.
        pub const EncodingEnvironment = W;
        /// Public evidence that decoding uses a direct Result and requires no runtime services.
        pub const pure_decoding = decoding_pure;
        /// Public evidence that encoding uses a direct Result and requires no runtime services.
        pub const pure_encoding = encoding_pure;
        const Decode = if (decoding_pure) fn (*const Rep) outcomes.Result(A, E) else fn (*const Rep) effects.Effect(A, E, D);
        const Encode = if (encoding_pure) fn (*const A) outcomes.Result(Rep, E) else fn (*const A) effects.Effect(Rep, E, W);
        /// Optional explicit immutable representation witness; null means no shape may be inferred from callbacks.
        encoded_shape: ?*const @import("schema_ast.zig").Node = null,
        /// Selected decode callback; borrowed input remains alive through direct use or lazy join.
        decoder: *const Decode,
        /// Selected encode callback; returned representation follows its declared ownership contract.
        encoder: *const Encode,
        /// Select checked comptime callbacks for each direction; constructing a codec does not execute them.
        pub fn init(comptime decode_: Decode, comptime encode_: Encode) Self {
            if (decoding_pure and D != void or encoding_pure and W != void) @compileError("pure domain direction requires void environment");
            return .{ .decoder = decode_, .encoder = encode_ };
        }
        /// Run the checked pure decoder immediately. Only a direction with public pure evidence may use this method.
        pub fn decodeDirect(self: Self, input: *const Rep) outcomes.Result(A, E) {
            if (!decoding_pure) @compileError("domain decoding has no pure evidence");
            return self.decoder(input);
        }
        /// Run the checked pure encoder immediately without an Effect frame; preserve its typed Result.
        pub fn encodeDirect(self: Self, input: *const A) outcomes.Result(Rep, E) {
            if (!encoding_pure) @compileError("domain encoding has no pure evidence");
            return self.encoder(input);
        }
        /// Run pure encoding once and discard its owning representation; a service-requiring direction is rejected.
        pub fn validateDirect(self: Self, input: *const A) outcomes.Result(void, E) {
            if (!encoding_pure) @compileError("domain validation has no pure encoding evidence");
            var encoded = self.encodeDirect(input);
            return switch (encoded) {
                .ok => blk: {
                    outcomes.discardPayload(&encoded.ok);
                    break :blk .{ .ok = {} };
                },
                .err => |err| .{ .err = err },
            };
        }
        /// Provide only decoding’s environment through a borrowed R projection; keep both environments alive until join.
        pub fn decodeIn(comptime self: Self, comptime R: type, input: *const Rep, comptime project: fn (*const R) *const Self.DecodingEnvironment) effects.Effect(A, E, R) {
            return self.decode(input).provideSome(R, project);
        }
        /// Provide only encoding’s environment through a borrowed R projection; decoding services are not required.
        pub fn encodeIn(comptime self: Self, comptime R: type, input: *const A, comptime project: fn (*const R) *const Self.EncodingEnvironment) effects.Effect(Rep, E, R) {
            return self.encode(input).provideSome(R, project);
        }
        /// Construct lazy borrowed-input decoding; repeated runs use their own environment and preserve all Exit channels.
        pub fn decode(comptime self: Self, input: *const Rep) effects.Effect(A, E, D) {
            if (comptime decoding_pure) return effects.Effect(A, E, D).fromBorrowed(Rep, input, struct {
                fn run(value: *const Rep, _: *const D) outcomes.Exit(A, E) {
                    return switch (self.decodeDirect(value)) {
                        .ok => |a| .{ .success = a },
                        .err => |e| .{ .fail = e },
                    };
                }
            }.run);
            return effects.Effect(A, E, D).deferBorrowed(Rep, input, struct {
                fn run(value: *const Rep, _: *const D) effects.Effect(A, E, D) {
                    return self.decoder(value);
                }
            }.run);
        }
        /// Construct lazy borrowed-value encoding; the callback supplies the representation and its ownership contract.
        pub fn encode(comptime self: Self, input: *const A) effects.Effect(Rep, E, W) {
            if (comptime encoding_pure) return effects.Effect(Rep, E, W).fromBorrowed(A, input, struct {
                fn run(value: *const A, _: *const W) outcomes.Exit(Rep, E) {
                    return switch (self.encodeDirect(value)) {
                        .ok => |a| .{ .success = a },
                        .err => |e| .{ .fail = e },
                    };
                }
            }.run);
            return effects.Effect(Rep, E, W).deferBorrowed(A, input, struct {
                fn run(value: *const A, _: *const W) effects.Effect(Rep, E, W) {
                    return self.encoder(value);
                }
            }.run);
        }
        /// Run encoding once and discard a marked owning success; preserve domain failures and finalization failures.
        pub fn validate(comptime self: Self, input: *const A) effects.Effect(void, E, W) {
            if (comptime encoding_pure) return effects.Effect(void, E, W).fromBorrowed(A, input, struct {
                fn run(value: *const A, _: *const W) outcomes.Exit(void, E) {
                    return switch (self.validateDirect(value)) {
                        .ok => .{ .success = {} },
                        .err => |e| .{ .fail = e },
                    };
                }
            }.run);
            return effects.Effect(void, E, W).deferBorrowed(A, input, struct {
                fn run(value: *const A, env: *const W) effects.Effect(void, E, W) {
                    var encoded = self.encode(value);
                    return encoded.mapOwned(env, struct {
                        fn discard(rep: Rep) void {
                            var owned = rep;
                            outcomes.discardPayload(&owned);
                        }
                    }.discard);
                }
            }.run);
        }
    };
}
/// Map schema Issues into E with independent decode/encode callbacks.
/// Each mapping callback takes ownership of its Issue and must release or retain it.
/// Direction environments, pure evidence and immutable encoded metadata are preserved; other Exit channels remain distinct.
pub fn ofSchema(comptime E: type, comptime schema: anytype, comptime decode_issue: fn (Issue) E, comptime encode_issue: fn (Issue) E) MappedType(E, schema) {
    const C = @TypeOf(schema);
    const Out = MappedType(E, schema);
    const Decoder = if (comptime directionPure(schema, true)) struct {
        fn run(input: *const C.Representation) outcomes.Result(C.Decoded, E) {
            return switch (schema.decodeDirect(input)) {
                .ok => |value| .{ .ok = value },
                .err => |issue| .{ .err = decode_issue(issue) },
            };
        }
    } else struct {
        fn run(input: *const C.Representation) effects.Effect(C.Decoded, E, C.DecodingEnvironment) {
            return effects.Effect(C.Decoded, E, C.DecodingEnvironment).deferBorrowed(C.Representation, input, struct {
                fn start(value: *const C.Representation, env: *const C.DecodingEnvironment) effects.Effect(C.Decoded, E, C.DecodingEnvironment) {
                    var program = schema.decode(value);
                    return program.mapErrorOwned(env, decode_issue);
                }
            }.start);
        }
    };
    const Encoder = if (comptime directionPure(schema, false)) struct {
        fn run(input: *const C.Decoded) outcomes.Result(C.Representation, E) {
            return switch (schema.encodeDirect(input)) {
                .ok => |value| .{ .ok = value },
                .err => |issue| .{ .err = encode_issue(issue) },
            };
        }
    } else struct {
        fn run(input: *const C.Decoded) effects.Effect(C.Representation, E, C.EncodingEnvironment) {
            return effects.Effect(C.Representation, E, C.EncodingEnvironment).deferBorrowed(C.Decoded, input, struct {
                fn start(value: *const C.Decoded, env: *const C.EncodingEnvironment) effects.Effect(C.Representation, E, C.EncodingEnvironment) {
                    var program = schema.encode(value);
                    return program.mapErrorOwned(env, encode_issue);
                }
            }.start);
        }
    };
    var result = Out.init(Decoder.run, Encoder.run);
    if (@hasDecl(C, "encodedNode")) result.encoded_shape = C.encodedNode() else if (@hasDecl(C, "encoded_node")) result.encoded_shape = &C.encoded_node else if (@hasField(C, "encoded_shape")) result.encoded_shape = schema.encoded_shape;
    return result;
}
/// Compose ?A with explicit tagged .none/.some native representation, not JSON null.
/// Preserve independent pure/effectful directions and child ownership; absent values invoke no child callback.
pub fn optional(comptime child: anytype) OptionalType(child) {
    const C = @TypeOf(child);
    const Rep = @import("option_codec.zig").Representation(C.Representation);
    const Out = OptionalType(child);
    const Decoder = if (comptime directionPure(child, true)) struct {
        fn run(input: *const Rep) outcomes.Result(?C.Decoded, C.Error) {
            return switch (input.*) {
                .none => .{ .ok = null },
                .some => |*value| switch (child.decodeDirect(value)) {
                    .ok => |a| .{ .ok = a },
                    .err => |e| .{ .err = e },
                },
            };
        }
    } else struct {
        fn run(input: *const Rep) effects.Effect(?C.Decoded, C.Error, C.DecodingEnvironment) {
            return effects.Effect(?C.Decoded, C.Error, C.DecodingEnvironment).deferBorrowed(Rep, input, struct {
                fn start(value: *const Rep, env: *const C.DecodingEnvironment) effects.Effect(?C.Decoded, C.Error, C.DecodingEnvironment) {
                    if (value.* == .none) return effects.Effect(?C.Decoded, C.Error, C.DecodingEnvironment).succeed(null);
                    var program = child.decode(&value.some);
                    return program.mapOwned(env, struct {
                        fn present(a: C.Decoded) ?C.Decoded {
                            return a;
                        }
                    }.present);
                }
            }.start);
        }
    };
    const Encoder = if (comptime directionPure(child, false)) struct {
        fn run(input: *const ?C.Decoded) outcomes.Result(Rep, C.Error) {
            if (input.*) |*value| return switch (child.encodeDirect(value)) {
                .ok => |a| .{ .ok = .{ .some = a } },
                .err => |e| .{ .err = e },
            };
            return .{ .ok = .none };
        }
    } else struct {
        fn run(input: *const ?C.Decoded) effects.Effect(Rep, C.Error, C.EncodingEnvironment) {
            return effects.Effect(Rep, C.Error, C.EncodingEnvironment).deferBorrowed(?C.Decoded, input, struct {
                fn start(value: *const ?C.Decoded, env: *const C.EncodingEnvironment) effects.Effect(Rep, C.Error, C.EncodingEnvironment) {
                    if (value.* == null) return effects.Effect(Rep, C.Error, C.EncodingEnvironment).succeed(.none);
                    var program = child.encode(&value.*.?);
                    return program.mapOwned(env, struct {
                        fn present(a: C.Representation) Rep {
                            return .{ .some = a };
                        }
                    }.present);
                }
            }.start);
        }
    };
    return Out.init(Decoder.run, Encoder.run);
}

/// Compose a bounded native collection using the child’s separate environments and domain capacity failure callback.
pub fn array(comptime child: anytype, comptime S: type, comptime maximum: usize, comptime capacity_error: fn () @TypeOf(child).Error) type {
    const C = @TypeOf(child);
    return @import("sequence_codec.zig").Sequence(child, S, C.DecodingEnvironment, C.EncodingEnvironment, C.Error, maximum, capacity_error, capacity_error);
}
/// Collection alias; representation/key semantics are supplied by the caller’s sequence contract.
pub const map = array;

fn pure(comptime child: anytype) bool {
    const C = @TypeOf(child);
    return @hasDecl(C, "pure_decoding") and @hasDecl(C, "pure_encoding") and C.pure_decoding and C.pure_encoding and C.DecodingEnvironment == void and C.EncodingEnvironment == void;
}
fn MappedType(comptime E: type, comptime child: anytype) type {
    const C = @TypeOf(child);
    return Directed(C.Decoded, C.Representation, E, C.DecodingEnvironment, C.EncodingEnvironment, directionPure(child, true), directionPure(child, false));
}
fn OptionalType(comptime child: anytype) type {
    const C = @TypeOf(child);
    const Rep = @import("option_codec.zig").Representation(C.Representation);
    return Directed(?C.Decoded, Rep, C.Error, C.DecodingEnvironment, C.EncodingEnvironment, directionPure(child, true), directionPure(child, false));
}
/// Pair Result callbacks that require no runtime services. Lazy adapters still use the same typed Effect/Exit contract.
pub fn Pure(comptime A: type, comptime Rep: type, comptime E: type) type {
    return struct {
        const Self = @This();
        /// Application value produced on success; ownership follows the codec’s declared value type.
        pub const Decoded = A;
        /// Wire or provider representation consumed and produced by this codec.
        pub const Representation = Rep;
        /// Application failure type; defects, interruption and finalization failures remain separate Exit channels.
        pub const Error = E;
        /// Services needed by decoding only; encoding services are not acquired by this direction.
        pub const DecodingEnvironment = void;
        /// Services needed by encoding only; decoding services are not acquired by this direction.
        pub const EncodingEnvironment = void;
        /// Public evidence that decoding uses a direct Result and requires no runtime services.
        pub const pure_decoding = true;
        /// Public evidence that encoding uses a direct Result and requires no runtime services.
        pub const pure_encoding = true;
        /// Optional explicit immutable representation witness; null means no shape may be inferred from callbacks.
        encoded_shape: ?*const @import("schema_ast.zig").Node = null,
        /// Selected decode callback; borrowed input remains alive through direct use or lazy join.
        decoder: *const fn (*const Rep) outcomes.Result(A, E),
        /// Selected encode callback; returned representation follows its declared ownership contract.
        encoder: *const fn (*const A) outcomes.Result(Rep, E),
        /// Select checked comptime callbacks for each direction; constructing a codec does not execute them.
        pub fn init(comptime decode_: fn (*const Rep) outcomes.Result(A, E), comptime encode_: fn (*const A) outcomes.Result(Rep, E)) Self {
            return .{ .decoder = decode_, .encoder = encode_ };
        }
        /// Run the checked pure decoder immediately. Only a direction with public pure evidence may use this method.
        pub fn decodeDirect(self: Self, value: *const Rep) outcomes.Result(A, E) {
            return self.decoder(value);
        }
        /// Run the checked pure encoder immediately without an Effect frame; preserve its typed Result.
        pub fn encodeDirect(self: Self, value: *const A) outcomes.Result(Rep, E) {
            return self.encoder(value);
        }
        /// Run pure encoding once and discard its owning representation; a service-requiring direction is rejected.
        pub fn validateDirect(self: Self, value: *const A) outcomes.Result(void, E) {
            var encoded = self.encodeDirect(value);
            return switch (encoded) {
                .ok => blk: {
                    outcomes.discardPayload(&encoded.ok);
                    break :blk .{ .ok = {} };
                },
                .err => |err| .{ .err = err },
            };
        }
        /// Provide only decoding’s environment through a borrowed R projection; keep both environments alive until join.
        pub fn decodeIn(comptime self: Self, comptime R: type, input: *const Rep, comptime project: fn (*const R) *const Self.DecodingEnvironment) effects.Effect(A, E, R) {
            return self.decode(input).provideSome(R, project);
        }
        /// Provide only encoding’s environment through a borrowed R projection; decoding services are not required.
        pub fn encodeIn(comptime self: Self, comptime R: type, input: *const A, comptime project: fn (*const R) *const Self.EncodingEnvironment) effects.Effect(Rep, E, R) {
            return self.encode(input).provideSome(R, project);
        }
        /// Construct lazy borrowed-input decoding; repeated runs use their own environment and preserve all Exit channels.
        pub fn decode(comptime self: Self, value: *const Rep) effects.Effect(A, E, void) {
            return effects.Effect(A, E, void).fromBorrowed(Rep, value, struct {
                fn run(input: *const Rep, _: *const void) outcomes.Exit(A, E) {
                    return switch (self.decodeDirect(input)) {
                        .ok => |result| .{ .success = result },
                        .err => |err| .{ .fail = err },
                    };
                }
            }.run);
        }
        /// Construct lazy borrowed-value encoding; the callback supplies the representation and its ownership contract.
        pub fn encode(comptime self: Self, value: *const A) effects.Effect(Rep, E, void) {
            return effects.Effect(Rep, E, void).fromBorrowed(A, value, struct {
                fn run(input: *const A, _: *const void) outcomes.Exit(Rep, E) {
                    return switch (self.encodeDirect(input)) {
                        .ok => |result| .{ .success = result },
                        .err => |err| .{ .fail = err },
                    };
                }
            }.run);
        }
        /// Run encoding once and discard a marked owning success; preserve domain failures and finalization failures.
        pub fn validate(comptime self: Self, value: *const A) effects.Effect(void, E, void) {
            return effects.Effect(void, E, void).fromBorrowed(A, value, struct {
                fn run(input: *const A, _: *const void) outcomes.Exit(void, E) {
                    return switch (self.validateDirect(input)) {
                        .ok => .{ .success = {} },
                        .err => |err| .{ .fail = err },
                    };
                }
            }.run);
        }
    };
}
