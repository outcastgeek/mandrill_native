//! Compile-time construction for a persisted typed Exit. Every required
//! capability and codec projection must be supplied before build succeeds.
const deadlines = @import("../deadline.zig");
const outcome_wire = @import("../schema/outcome_codec.zig");
const typed = @import("../schema/typed_outcome_codec.zig");
const stores = @import("key_value_store.zig");
const persisted = @import("persistable.zig");

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

        pub fn start() Self {
            return .{};
        }

        pub fn withCodec(comptime self: Self, comptime codec: Codec) Self {
            var next = self;
            next.codec = codec;
            return next;
        }

        pub fn withSchemaVersion(comptime self: Self, comptime version: u32) Self {
            var next = self;
            next.schema_version = version;
            return next;
        }

        pub fn withLimits(comptime self: Self, comptime limits: outcome_wire.Limits) Self {
            var next = self;
            next.limits = limits;
            return next;
        }

        pub fn withStore(comptime self: Self, comptime select: fn (*const R) *const Store) Self {
            var next = self;
            next.select_store = select;
            return next;
        }

        pub fn withKey(comptime self: Self, comptime select: fn (*const R) []const u8) Self {
            var next = self;
            next.select_key = select;
            return next;
        }

        pub fn withExpiry(comptime self: Self, comptime select: fn (*const R) deadlines.Deadline) Self {
            var next = self;
            next.select_expiry = select;
            return next;
        }

        pub fn withDecodeEnvironment(comptime self: Self, comptime select: fn (*const R) *const DecodeR) Self {
            var next = self;
            next.select_decode_environment = select;
            return next;
        }

        pub fn withEncodeEnvironment(comptime self: Self, comptime select: fn (*const R) *const EncodeR) Self {
            var next = self;
            next.select_encode_environment = select;
            return next;
        }

        /// When both codec directions use R, derive the two identity
        /// projections at comptime instead of making consumers repeat them.
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
