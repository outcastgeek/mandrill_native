//! Bidirectional typed codecs. Decoded and encoded values, and the two
//! dependency environments, remain independent type parameters.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const legacy = @import("../schema.zig");
const Issue = @import("schema_issue.zig").Issue;

/// Bidirectional codec interface specialized at comptime. The decoder and
/// encoder are statically typed functions; each constructs a lazy Effect, and
/// decoding and encoding may require different caller environments.
/// Parameters:
/// - `Value: comptime type`: Decoded application value type.
/// - `Encoded: comptime type`: Encoded representation type.
/// - `DecodeEnv: comptime type`: Environment required by the decoder Effect.
/// - `EncodeEnv: comptime type`: Environment required by the encoder Effect.
/// Returns (`type`): A codec type whose callbacks have signatures
/// `fn (*const Encoded) Effect(Value, Issue, DecodeEnv)` and
/// `fn (*const Value) Effect(Encoded, Issue, EncodeEnv)`.
pub fn Codec(comptime Value: type, comptime Encoded: type, comptime DecodeEnv: type, comptime EncodeEnv: type) type {
    return struct {
        const Self = @This();
        /// Application value type produced by this codec.
        pub const Decoded = Value;
        /// Encoded representation type consumed and produced by this codec.
        pub const Representation = Encoded;
        /// Environment required to run decoding effects.
        pub const DecodingEnvironment = DecodeEnv;
        /// Environment required to run encoding effects.
        pub const EncodingEnvironment = EncodeEnv;

        decoder: *const fn (*const Encoded) effects.Effect(Value, Issue, DecodeEnv),
        encoder: *const fn (*const Value) effects.Effect(Encoded, Issue, EncodeEnv),

        /// Pair effectful decoding and encoding callbacks for this representation.
        /// Parameters:
        /// - `decode_value: comptime fn (*const Encoded) effects.Effect(Value, Issue, DecodeEnv)`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// - `encode_value: comptime fn (*const Value) effects.Effect(Encoded, Issue, EncodeEnv)`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Self`): a codec whose callbacks produce typed Effects in their respective environments.
        pub fn init(
            comptime decode_value: fn (*const Encoded) effects.Effect(Value, Issue, DecodeEnv),
            comptime encode_value: fn (*const Value) effects.Effect(Encoded, Issue, EncodeEnv),
        ) Self {
            return .{ .decoder = decode_value, .encoder = encode_value };
        }

        /// Decoding can suspend, fail, be interrupted, or run an effectful
        /// default/refinement. Inputs remain borrowed until the Effect joins.
        ///
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `encoded: *const Encoded`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`effects.Effect(Value, Issue, DecodeEnv)`): decoded application value, preserving typed validation failure and runtime defects.
        pub fn decode(comptime self: Self, encoded: *const Encoded) effects.Effect(Value, Issue, DecodeEnv) {
            return effects.Effect(Value, Issue, DecodeEnv).deferBorrowed(Encoded, encoded, struct {
                fn start(input: *const Encoded, _: *const DecodeEnv) effects.Effect(Value, Issue, DecodeEnv) {
                    return self.decoder(input);
                }
            }.start);
        }

        /// Encode a value into its wire representation.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `value: *const Value`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`effects.Effect(Encoded, Issue, EncodeEnv)`): encoded representation produced by the selected codec.
        pub fn encode(comptime self: Self, value: *const Value) effects.Effect(Encoded, Issue, EncodeEnv) {
            return effects.Effect(Encoded, Issue, EncodeEnv).deferBorrowed(Value, value, struct {
                fn start(input: *const Value, _: *const EncodeEnv) effects.Effect(Encoded, Issue, EncodeEnv) {
                    return self.encoder(input);
                }
            }.start);
        }

        /// A larger R can provide each direction through a narrow projection.
        /// Allocator and I/O, when needed, remain fields of that environment.
        ///
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `R: comptime type`: Caller-defined environment type supplying the dependencies needed by this operation.
        /// - `encoded: *const Encoded`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// - `project: comptime fn (*const R) *const DecodeEnv`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`effects.Effect(Value, Issue, R)`): decoded value using a decoding environment projected from the caller’s `R`.
        pub fn decodeIn(
            comptime self: Self,
            comptime R: type,
            encoded: *const Encoded,
            comptime project: fn (*const R) *const DecodeEnv,
        ) effects.Effect(Value, Issue, R) {
            return self.decode(encoded).provideSome(R, project);
        }

        /// Encode a value using a caller-selected environment.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `R: comptime type`: Caller-defined environment type supplying the dependencies needed by this operation.
        /// - `value: *const Value`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// - `project: comptime fn (*const R) *const EncodeEnv`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`effects.Effect(Encoded, Issue, R)`): encoded representation using an encoding environment projected from `R`.
        pub fn encodeIn(
            comptime self: Self,
            comptime R: type,
            value: *const Value,
            comptime project: fn (*const R) *const EncodeEnv,
        ) effects.Effect(Encoded, Issue, R) {
            return self.encode(value).provideSome(R, project);
        }
    };
}

/// Pure callbacks never acquire a runtime service or introduce an implicit
/// native error channel. They can still report structured root issues.
///
/// Parameters:
/// - `Value: comptime type`: Value type represented or stored by this component.
/// - `Encoded: comptime type`: Compile-time encoded representation type.
/// Returns (`type`): paired pure decoder and encoder returning typed `Result` values.
pub fn PureCodec(comptime Value: type, comptime Encoded: type) type {
    return struct {
        const Self = @This();
        /// Application value type produced by this codec.
        pub const Decoded = Value;
        /// Encoded representation type consumed and produced by this codec.
        pub const Representation = Encoded;
        /// Environment required to run decoding effects.
        pub const DecodingEnvironment = void;
        /// Environment required to run encoding effects.
        pub const EncodingEnvironment = void;

        decoder: *const fn (*const Encoded) outcomes.Result(Value, Issue),
        encoder: *const fn (*const Value) outcomes.Result(Encoded, Issue),

        /// Pair pure decoding and encoding callbacks for this representation.
        /// Parameters:
        /// - `decode_value: comptime fn (*const Encoded) outcomes.Result(Value, Issue)`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// - `encode_value: comptime fn (*const Value) outcomes.Result(Encoded, Issue)`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Self`): a pure codec returning typed `Result` values without an Effect runtime.
        pub fn init(
            comptime decode_value: fn (*const Encoded) outcomes.Result(Value, Issue),
            comptime encode_value: fn (*const Value) outcomes.Result(Encoded, Issue),
        ) Self {
            return .{ .decoder = decode_value, .encoder = encode_value };
        }

        /// Decode immediately and return the typed validation outcome.
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `encoded: *const Encoded`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`outcomes.Result(Value, Issue)`): An ok/err Result; owned payloads must be released according to their type contract.
        pub fn decodeDirect(self: Self, encoded: *const Encoded) outcomes.Result(Value, Issue) {
            return self.decoder(encoded);
        }

        /// Encode immediately and return a typed outcome.
        /// Parameters:
        /// - `self: Self`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
        /// - `value: *const Value`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`outcomes.Result(Encoded, Issue)`): An ok/err Result; owned payloads must be released according to their type contract.
        pub fn encodeDirect(self: Self, value: *const Value) outcomes.Result(Encoded, Issue) {
            return self.encoder(value);
        }

        /// Expose this schema through the Codec interface.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// Returns (`Codec(Value, Encoded, void, void)`): A typed codec or schema preserving the declared decoded value, representation, and environments.
        pub fn asCodec(comptime self: Self) Codec(Value, Encoded, void, void) {
            return Codec(Value, Encoded, void, void).init(struct {
                fn decodeValue(encoded: *const Encoded) effects.Effect(Value, Issue, void) {
                    return effects.Effect(Value, Issue, void).fromResult(self.decodeDirect(encoded));
                }
            }.decodeValue, struct {
                fn encodeValue(value: *const Value) effects.Effect(Encoded, Issue, void) {
                    return effects.Effect(Encoded, Issue, void).fromResult(self.encodeDirect(value));
                }
            }.encodeValue);
        }

        /// An old decoder can read the new JSON codec. The caller explicitly
        /// maps issues into its old E; the reverse direction is not inferred.
        ///
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
        /// - `map_issue: comptime fn (Issue) E`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`legacy.Schema(Value, E)`): A typed codec or schema preserving the declared decoded value, representation, and environments.
        pub fn toLegacy(comptime self: Self, comptime E: type, comptime map_issue: fn (Issue) E) legacy.Schema(Value, E) {
            if (Encoded != std.json.Value) @compileError("toLegacy requires a JSON representation");
            return legacy.Schema(Value, E).fromDecoder(struct {
                fn decodeValue(encoded: std.json.Value) outcomes.Result(Value, E) {
                    return switch (self.decodeDirect(&encoded)) {
                        .ok => |value| .{ .ok = value },
                        .err => |issue| .{ .err = map_issue(issue) },
                    };
                }
            }.decodeValue);
        }
    };
}
