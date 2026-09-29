//! Typed request descriptions. The stable key is supplied by the caller and
//! remains distinct from the request codec/version fingerprint and any local
//! hash used by an implementation for indexing.
const std = @import("std");
const keys = @import("primary_key.zig");

/// Define a typed request from three codec types. This is a structural
/// comptime interface: each codec must expose `Decoded`, or compilation fails.
/// Parameters:
/// - `InputCodec: comptime type`: Codec whose `Decoded` type becomes `Input`.
/// - `SuccessCodec: comptime type`: Codec whose `Decoded` type becomes `Success`.
/// - `ErrorCodec: comptime type`: Codec whose `Decoded` type becomes `DomainError`.
/// Returns (`type`): A request definition type carrying the three exact codec
/// types and their decoded types; no runtime allocation or registration.
pub fn Request(comptime InputCodec: type, comptime SuccessCodec: type, comptime ErrorCodec: type) type {
    comptime {
        if (!@hasDecl(InputCodec, "Decoded") or !@hasDecl(SuccessCodec, "Decoded") or
            !@hasDecl(ErrorCodec, "Decoded"))
            @compileError("request codecs must expose decoded input, success and error types");
    }
    return struct {
        const Self = @This();
        /// Typed request or input value decoded by this definition.
        pub const Input = InputCodec.Decoded;
        /// Value type carried by the successful channel.
        pub const Success = SuccessCodec.Decoded;
        /// Domain-specific error type carried by the operation.
        pub const DomainError = ErrorCodec.Decoded;

        name: []const u8,
        version: u32,
        input: InputCodec,
        success: SuccessCodec,
        failure: ErrorCodec,

        /// Construct a statically checked definition.
        /// Parameters:
        /// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition
        ///   time.
        /// - `version: comptime u32`: Compile-time stable version used to reject incompatible encoded or
        ///   replayed data.
        /// - `input: comptime InputCodec`: Compile-time typed input value; move or clone owning payloads according to
        ///   its type contract.
        /// - `success: comptime SuccessCodec`: Codec or value for the successful result channel.
        /// - `failure: comptime ErrorCodec`: Codec or value for the typed failure channel.
        /// Returns (`Self`): A constructed instance; its provider and allocation ownership follow the containing type.
        pub fn define(comptime name: []const u8, comptime version: u32, comptime input: InputCodec, comptime success: SuccessCodec, comptime failure: ErrorCodec) Self {
            if (name.len == 0 or !std.unicode.utf8ValidateSlice(name))
                @compileError("request names must be stable nonempty UTF-8");
            if (version == 0) @compileError("request version zero is reserved for missing metadata");
            return .{ .name = name, .version = version, .input = input, .success = success, .failure = failure };
        }

        /// Construct or inspect the typed stable key for this definition.
        /// Parameters:
        /// - `self: comptime Self`: Compile-time current value passed by value; ownership follows this
        ///   operation’s contract.
        /// - `encoded: []const u8`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// Returns (`?Key(Self)`): stable borrowed request key, or `null` when this request has no key.
        pub fn key(comptime self: Self, encoded: []const u8) ?Key(Self) {
            if (keys.inspectEncoded(encoded) == .err) return null;
            return .{ .definition = self, .encoded = encoded };
        }
    };
}

/// Borrowed while the resolver Effect and every registered observer are live.
///
/// Parameters:
/// - `Definition: comptime type`: Compile-time typed definition used to derive this API.
/// Returns (`type`): borrowed request key type; its source must outlive the resolver run and observers.
pub fn Key(comptime Definition: type) type {
    return struct {
        /// Original request definition to which this key belongs.
        pub const RequestDefinition = Definition;
        definition: Definition,
        encoded: []const u8,
    };
}
