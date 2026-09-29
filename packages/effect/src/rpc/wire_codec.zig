//! Typed payload codecs for encoded RPC. Envelope framing is separate; this
//! adapter keeps each codec's decode and encode environments projected from R.
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const Issue = @import("../schema/schema_issue.zig").Issue;

/// Owned bytes buffer paired with its allocator; release it with `deinit`.
pub const Bytes = outcomes.OwnedSlice(u8);

/// Define a typed wire codec with its compile-time value and representation types.
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): typed RPC bytes codec whose encode/decode callbacks return Effects in `R`.
pub fn WireCodec(comptime A: type, comptime R: type) type {
    return struct {
        const Self = @This();
        /// Application value type produced by this codec.
        pub const Decoded = A;
        /// Exact caller environment type required when the description runs.
        pub const Environment = R;
        decoder: *const fn (*const Bytes) effects.Effect(A, Issue, R),
        encoder: *const fn (*const A) effects.Effect(Bytes, Issue, R),

        /// Pair typed RPC wire decoding and encoding Effects.
        /// Parameters:
        /// - `decode_payload: comptime fn (*const Bytes) effects.Effect(A, Issue, R)`: Compile-time typed function selected at comptime; no runtime
        ///   callback lookup is required.
        /// - `encode_value: comptime fn (*const A) effects.Effect(Bytes, Issue, R)`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Self`): a codec whose callbacks convert bytes and `A` through the caller environment.
        pub fn init(
            comptime decode_payload: fn (*const Bytes) effects.Effect(A, Issue, R),
            comptime encode_value: fn (*const A) effects.Effect(Bytes, Issue, R),
        ) Self {
            return .{ .decoder = decode_payload, .encoder = encode_value };
        }

        /// Adapt a schema codec to the RPC wire-codec contract.
        /// Parameters:
        /// - `codec: comptime anytype`: Compile-time typed codec/schema fixing the corresponding decoded and encoded
        ///   value types.
        /// - `project_decode: comptime fn (*const R) *const @TypeOf(codec).DecodingEnvironment`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `project_encode: comptime fn (*const R) *const @TypeOf(codec).EncodingEnvironment`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`Self`): A constructed instance; its provider and allocation ownership follow the containing type.
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
        ///
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `bytes: *const Bytes`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`effects.Effect(A, Issue, R)`): typed request or response value decoded from RPC bytes.
        pub fn decode(comptime self: Self, bytes: *const Bytes) effects.Effect(A, Issue, R) {
            return effects.Effect(A, Issue, R).deferBorrowed(Bytes, bytes, struct {
                fn start(input: *const Bytes, _: *const R) effects.Effect(A, Issue, R) {
                    return self.decoder(input);
                }
            }.start);
        }

        /// Encode a value into its wire representation.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `value: *const A`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`effects.Effect(Bytes, Issue, R)`): owned RPC bytes produced from the typed value.
        pub fn encode(comptime self: Self, value: *const A) effects.Effect(Bytes, Issue, R) {
            return effects.Effect(Bytes, Issue, R).deferBorrowed(A, value, struct {
                fn start(input: *const A, _: *const R) effects.Effect(Bytes, Issue, R) {
                    return self.encoder(input);
                }
            }.start);
        }
    };
}
