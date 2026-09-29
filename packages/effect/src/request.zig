//! Typed request descriptions. The stable key is supplied by the caller and
//! remains distinct from the request codec/version fingerprint and any local
//! hash used by an implementation for indexing.
const std = @import("std");
const keys = @import("primary_key.zig");

pub fn Request(comptime InputCodec: type, comptime SuccessCodec: type, comptime ErrorCodec: type) type {
    comptime {
        if (!@hasDecl(InputCodec, "Decoded") or !@hasDecl(SuccessCodec, "Decoded") or
            !@hasDecl(ErrorCodec, "Decoded"))
            @compileError("request codecs must expose decoded input, success and error types");
    }
    return struct {
        const Self = @This();
        pub const Input = InputCodec.Decoded;
        pub const Success = SuccessCodec.Decoded;
        pub const DomainError = ErrorCodec.Decoded;

        name: []const u8,
        version: u32,
        input: InputCodec,
        success: SuccessCodec,
        failure: ErrorCodec,

        pub fn define(comptime name: []const u8, comptime version: u32, comptime input: InputCodec, comptime success: SuccessCodec, comptime failure: ErrorCodec) Self {
            if (name.len == 0 or !std.unicode.utf8ValidateSlice(name))
                @compileError("request names must be stable nonempty UTF-8");
            if (version == 0) @compileError("request version zero is reserved for missing metadata");
            return .{ .name = name, .version = version, .input = input, .success = success, .failure = failure };
        }

        pub fn key(comptime self: Self, encoded: []const u8) ?Key(Self) {
            if (keys.inspectEncoded(encoded) == .err) return null;
            return .{ .definition = self, .encoded = encoded };
        }
    };
}

/// Borrowed while the resolver Effect and every registered observer are live.
pub fn Key(comptime Definition: type) type {
    return struct {
        pub const RequestDefinition = Definition;
        definition: Definition,
        encoded: []const u8,
    };
}
