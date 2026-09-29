//! Typed payload codecs for encoded RPC. Envelope framing is separate; this
//! adapter keeps each codec's decode and encode environments projected from R.
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const Issue = @import("../schema/schema_issue.zig").Issue;

pub const Bytes = outcomes.OwnedSlice(u8);

pub fn WireCodec(comptime A: type, comptime R: type) type {
    return struct {
        const Self = @This();
        pub const Decoded = A;
        pub const Environment = R;
        decoder: *const fn (*const Bytes) effects.Effect(A, Issue, R),
        encoder: *const fn (*const A) effects.Effect(Bytes, Issue, R),

        pub fn init(
            comptime decode_payload: fn (*const Bytes) effects.Effect(A, Issue, R),
            comptime encode_value: fn (*const A) effects.Effect(Bytes, Issue, R),
        ) Self {
            return .{ .decoder = decode_payload, .encoder = encode_value };
        }

        pub fn fromCodec(
            comptime codec: anytype,
            comptime project_decode: fn (*const R) *const @TypeOf(codec).DecodingEnvironment,
            comptime project_encode: fn (*const R) *const @TypeOf(codec).EncodingEnvironment,
        ) Self {
            const C = @TypeOf(codec);
            if (C.Decoded != A or C.Representation != Bytes or
                !@hasDecl(C, "decodeIn") or !@hasDecl(C, "encodeIn"))
                @compileError("RPC wire codec requires a typed owned-byte Codec");
            return init(struct {
                fn decodeValue(bytes: *const Bytes) effects.Effect(A, Issue, R) {
                    return codec.decodeIn(R, bytes, project_decode);
                }
            }.decodeValue, struct {
                fn encodeValue(value: *const A) effects.Effect(Bytes, Issue, R) {
                    return codec.encodeIn(R, value, project_encode);
                }
            }.encodeValue);
        }

        /// The borrowed input must outlive the returned Effect. Client/server
        /// helpers retain the owning frame until the effect has joined.
        pub fn decode(comptime self: Self, bytes: *const Bytes) effects.Effect(A, Issue, R) {
            return effects.Effect(A, Issue, R).deferBorrowed(Bytes, bytes, struct {
                fn start(input: *const Bytes, _: *const R) effects.Effect(A, Issue, R) {
                    return self.decoder(input);
                }
            }.start);
        }

        pub fn encode(comptime self: Self, value: *const A) effects.Effect(Bytes, Issue, R) {
            return effects.Effect(Bytes, Issue, R).deferBorrowed(A, value, struct {
                fn start(input: *const A, _: *const R) effects.Effect(Bytes, Issue, R) {
                    return self.encoder(input);
                }
            }.start);
        }
    };
}
