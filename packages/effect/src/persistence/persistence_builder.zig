//! Compile-time construction for a persisted typed Exit. Every required
//! capability and codec projection must be supplied before build succeeds.
const deadlines = @import("../deadline.zig");
const outcome_wire = @import("../schema/outcome_codec.zig");
const typed = @import("../schema/typed_outcome_codec.zig");
const stores = @import("key_value_store.zig");
const persisted = @import("persistable.zig");

/// Staged persisted-Exit definition. A codec, schema version, wire limits,
/// key/expiry selectors, store, and codec environments are required by `build`.
/// The resulting operation uses the provider supplied through `R`.
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// - `DecodeR: comptime type`: Environment type passed to the decoding side of the codec.
/// - `EncodeR: comptime type`: Environment type passed to the encoding side of the codec.
/// Returns (`type`): staged persisted-outcome builder; `build` checks that codecs, store, key, expiry, limits, and environments were supplied.
pub fn Builder(
    comptime A: type,
    comptime E: type,
    comptime R: type,
    comptime ProviderError: type,
    comptime DecodeR: type,
    comptime EncodeR: type,
) type {
    return struct {
        const Self = @This();
        const Codec = typed.PayloadCodec(A, E, DecodeR, EncodeR);
        const Store = stores.KeyValueStore(R, ProviderError);
        const Persistable = persisted.Persistable(A, E, R, ProviderError, DecodeR, EncodeR);

        codec: ?Codec = null,
        schema_version: ?u32 = null,
        limits: ?outcome_wire.Limits = null,
        select_store: ?*const fn (*const R) *const Store = null,
        select_key: ?*const fn (*const R) []const u8 = null,
        select_expiry: ?*const fn (*const R) deadlines.Deadline = null,
        select_decode_environment: ?*const fn (*const R) *const DecodeR = null,
        select_encode_environment: ?*const fn (*const R) *const EncodeR = null,

        /// Starts an empty persisted-outcome builder; build rejects missing components.
        ///
        /// Returns (`Self`): empty builder stage; supply all required components before calling `build`.
        pub fn start() Self {
            return .{};
        }

        /// Supplies codecs for the typed success and error payloads of a stored Exit.
        ///
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `codec: comptime Codec`: Compile-time typed codec/schema fixing the corresponding decoded and encoded
        ///   value types.
        /// Returns (`Self`): builder stage retaining the typed success and error codecs.
        pub fn withCodec(comptime self: Self, comptime codec: Codec) Self {
            var next = self;
            next.codec = codec;
            return next;
        }

        /// Sets the stable stored schema version; zero is reserved for absent metadata.
        ///
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `version: comptime u32`: Compile-time stable version used to reject incompatible encoded or
        ///   replayed data.
        /// Returns (`Self`): builder stage retaining the nonzero wire-schema version.
        pub fn withSchemaVersion(comptime self: Self, comptime version: u32) Self {
            var next = self;
            next.schema_version = version;
            return next;
        }

        /// Sets caller-chosen bounds for stored wire payload and cleanup records.
        ///
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `limits: comptime outcome_wire.Limits`: Compile-time caller-selected, validated behavior or resource bounds; no
        ///   hidden default is installed.
        /// Returns (`Self`): builder stage retaining caller-chosen encoded size and cleanup limits.
        pub fn withLimits(comptime self: Self, comptime limits: outcome_wire.Limits) Self {
            var next = self;
            next.limits = limits;
            return next;
        }

        /// Projects the encoded key/value capability from R.
        ///
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `select: comptime fn (*const R) *const Store`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Self`): builder stage selecting the persistence provider from `R` at execution.
        pub fn withStore(comptime self: Self, comptime select: fn (*const R) *const Store) Self {
            var next = self;
            next.select_store = select;
            return next;
        }

        /// Projects the stable persisted key from R.
        ///
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `select: comptime fn (*const R) []const u8`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Self`): builder stage selecting a stable persistence key from `R`.
        pub fn withKey(comptime self: Self, comptime select: fn (*const R) []const u8) Self {
            var next = self;
            next.select_key = select;
            return next;
        }

        /// Projects the caller-selected expiry deadline from R.
        ///
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `select: comptime fn (*const R) deadlines.Deadline`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Self`): builder stage selecting an expiry deadline from `R`.
        pub fn withExpiry(comptime self: Self, comptime select: fn (*const R) deadlines.Deadline) Self {
            var next = self;
            next.select_expiry = select;
            return next;
        }

        /// Projects the codec decoding environment from R.
        ///
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `select: comptime fn (*const R) *const DecodeR`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Self`): builder stage projecting the decoder environment from `R`.
        pub fn withDecodeEnvironment(comptime self: Self, comptime select: fn (*const R) *const DecodeR) Self {
            var next = self;
            next.select_decode_environment = select;
            return next;
        }

        /// Projects the codec encoding environment from R.
        ///
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `select: comptime fn (*const R) *const EncodeR`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`Self`): builder stage projecting the encoder environment from `R`.
        pub fn withEncodeEnvironment(comptime self: Self, comptime select: fn (*const R) *const EncodeR) Self {
            var next = self;
            next.select_encode_environment = select;
            return next;
        }

        /// When both codec directions use R, derive the two identity
        /// projections at comptime instead of making consumers repeat them.
        ///
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// Returns (`Self`): builder stage using `R` for both codec directions.
        pub fn withSharedEnvironment(comptime self: Self) Self {
            if (DecodeR != R or EncodeR != R)
                @compileError("shared codec environment requires DecodeR and EncodeR to equal R");
            const select = struct {
                fn identity(env: *const R) *const R {
                    return env;
                }
            }.identity;
            var next = self;
            next.select_decode_environment = select;
            next.select_encode_environment = select;
            return next;
        }

        /// Completes a typed Persistable only after every codec, policy, and projection is present.
        ///
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// Returns (`Persistable`): Schema-backed persistence definition with stable identity.
        pub fn build(comptime self: Self) Persistable {
            return Persistable.define(
                self.codec orelse @compileError("persisted Exit requires a typed payload codec"),
                self.schema_version orelse @compileError("persisted Exit requires a schema version"),
                self.limits orelse @compileError("persisted Exit requires wire limits"),
                (self.select_store orelse @compileError("persisted Exit requires a store selector")).*,
                (self.select_key orelse @compileError("persisted Exit requires a key selector")).*,
                (self.select_expiry orelse @compileError("persisted Exit requires an expiry selector")).*,
                (self.select_decode_environment orelse @compileError("persisted Exit requires a decode environment selector")).*,
                (self.select_encode_environment orelse @compileError("persisted Exit requires an encode environment selector")).*,
            );
        }
    };
}
