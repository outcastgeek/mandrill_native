//! Bidirectional typed codecs. Decoded and encoded values, and the two
//! dependency environments, remain independent type parameters.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const legacy = @import("../schema.zig");
const Issue = @import("schema_issue.zig").Issue;

pub fn Codec(comptime Value: type, comptime Encoded: type, comptime DecodeEnv: type, comptime EncodeEnv: type) type {
    return struct {
        const Self = @This();
        pub const Decoded = Value;
        pub const Representation = Encoded;
        pub const DecodingEnvironment = DecodeEnv;
        pub const EncodingEnvironment = EncodeEnv;

        decoder: *const fn (*const Encoded) effects.Effect(Value, Issue, DecodeEnv),
        encoder: *const fn (*const Value) effects.Effect(Encoded, Issue, EncodeEnv),

        pub fn init(
            comptime decode_value: fn (*const Encoded) effects.Effect(Value, Issue, DecodeEnv),
            comptime encode_value: fn (*const Value) effects.Effect(Encoded, Issue, EncodeEnv),
        ) Self {
            return .{ .decoder = decode_value, .encoder = encode_value };
        }

        /// Decoding can suspend, fail, be interrupted, or run an effectful
        /// default/refinement. Inputs remain borrowed until the Effect joins.
        pub fn decode(comptime self: Self, encoded: *const Encoded) effects.Effect(Value, Issue, DecodeEnv) {
            return effects.Effect(Value, Issue, DecodeEnv).deferBorrowed(Encoded, encoded, struct {
                fn start(input: *const Encoded, _: *const DecodeEnv) effects.Effect(Value, Issue, DecodeEnv) {
                    return self.decoder(input);
                }
            }.start);
        }

        pub fn encode(comptime self: Self, value: *const Value) effects.Effect(Encoded, Issue, EncodeEnv) {
            return effects.Effect(Encoded, Issue, EncodeEnv).deferBorrowed(Value, value, struct {
                fn start(input: *const Value, _: *const EncodeEnv) effects.Effect(Encoded, Issue, EncodeEnv) {
                    return self.encoder(input);
                }
            }.start);
        }

        /// A larger R can provide each direction through a narrow projection.
        /// Allocator and I/O, when needed, remain fields of that environment.
        pub fn decodeIn(
            comptime self: Self,
            comptime R: type,
            encoded: *const Encoded,
            comptime project: fn (*const R) *const DecodeEnv,
        ) effects.Effect(Value, Issue, R) {
            return self.decode(encoded).provideSome(R, project);
        }

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
pub fn PureCodec(comptime Value: type, comptime Encoded: type) type {
    return struct {
        const Self = @This();
        pub const Decoded = Value;
        pub const Representation = Encoded;
        pub const DecodingEnvironment = void;
        pub const EncodingEnvironment = void;

        decoder: *const fn (*const Encoded) outcomes.Result(Value, Issue),
        encoder: *const fn (*const Value) outcomes.Result(Encoded, Issue),

        pub fn init(
            comptime decode_value: fn (*const Encoded) outcomes.Result(Value, Issue),
            comptime encode_value: fn (*const Value) outcomes.Result(Encoded, Issue),
        ) Self {
            return .{ .decoder = decode_value, .encoder = encode_value };
        }

        pub fn decodeDirect(self: Self, encoded: *const Encoded) outcomes.Result(Value, Issue) {
            return self.decoder(encoded);
        }

        pub fn encodeDirect(self: Self, value: *const Value) outcomes.Result(Encoded, Issue) {
            return self.encoder(value);
        }

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
