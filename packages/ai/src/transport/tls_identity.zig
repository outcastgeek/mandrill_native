//! Client certificate negotiation and CertificateVerify messages. TLS fields
//! use the standard binary decoder/writer; key material uses typed DER codecs.
const std = @import("std");
const tls = std.crypto.tls;
const keys = @import("../crypto/keys.zig");
const signing = @import("../crypto/tls_signing.zig");
pub const Identity = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    private_key: []const u8,
    key_type: keys.PrivateKeyType,
    certificates: []const []const u8,
    allow_pkcs1: bool = false,
    pss_allowed: [3]bool = .{ true, true, true },
    requested: bool = false,
    scheme: ?tls.SignatureScheme = null,
    context: []const u8 = "",
    /// All allocated certificate/request data uses a; private_key borrows its
    /// caller through the handshake. Callers normally supply a request arena.
    pub fn load(a: std.mem.Allocator, io: std.Io, certificate: []const u8, private_key: []const u8) !@This() {
        var key_type = try keys.privateKeyType(a, io, private_key);
        const private_type = key_type;
        var chain = try @import("../auth/signing.zig").CertificateChain.open(a, io, certificate);
        defer chain.deinit();
        var certificates: std.ArrayList([]const u8) = .empty;
        errdefer {
            for (certificates.items) |der| a.free(der);
            certificates.deinit(a);
        }
        while (try chain.next(io)) |value| {
            var leaf = value;
            defer leaf.deinit();
            const bytes = try a.dupe(u8, try leaf.signature.slice());
            errdefer a.free(bytes);
            try certificates.append(a, bytes);
        }
        if (certificates.items.len == 0) {
            var leaf = try @import("../auth/signing.zig").certificateLeaf(a, io, certificate);
            defer leaf.deinit();
            const bytes = try a.dupe(u8, try leaf.signature.slice());
            errdefer a.free(bytes);
            try certificates.append(a, bytes);
        }
        const parsed = try (std.crypto.Certificate{ .buffer = certificates.items[0], .index = 0 }).parse();
        if (!try keys.matchesCertificate(a, io, private_key, private_type, parsed)) return error.CertificatePrivateKeyMismatch;
        var allowed = [_]bool{ true, true, true };
        var allow_pkcs1 = false;
        if (private_type == .rsa or private_type == .rsa_pss) {
            key_type = switch (parsed.pub_key_algo) {
                .rsaEncryption => .rsa,
                .rsassa_pss => .rsa_pss,
                else => return error.InvalidPrivateKey,
            };
            const private_rules = try keys.rsaPssPrivateRules(a, io, private_key);
            const certificate_rules = try keys.rsaPssCertificateRules(a, parsed);
            inline for (.{ keys.RsaHash.sha256, keys.RsaHash.sha384, keys.RsaHash.sha512 }, .{ 32, 48, 64 }, 0..) |hash, length, index| {
                allowed[index] = private_rules.allows(hash, length) and certificate_rules.allows(hash, length);
            }
            allow_pkcs1 = private_type == .rsa and key_type == .rsa;
        }
        return .{ .allocator = a, .io = io, .private_key = private_key, .key_type = key_type, .certificates = try certificates.toOwnedSlice(a), .allow_pkcs1 = allow_pkcs1, .pss_allowed = allowed };
    }
    fn compatible(self: @This(), scheme: tls.SignatureScheme, version: tls.ProtocolVersion) bool {
        return switch (scheme) {
            .rsa_pss_rsae_sha256 => self.key_type == .rsa and self.pss_allowed[0],
            .rsa_pss_rsae_sha384 => self.key_type == .rsa and self.pss_allowed[1],
            .rsa_pss_rsae_sha512 => self.key_type == .rsa and self.pss_allowed[2],
            .rsa_pss_pss_sha256 => self.key_type == .rsa_pss and self.pss_allowed[0],
            .rsa_pss_pss_sha384 => self.key_type == .rsa_pss and self.pss_allowed[1],
            .rsa_pss_pss_sha512 => self.key_type == .rsa_pss and self.pss_allowed[2],
            .rsa_pkcs1_sha256, .rsa_pkcs1_sha384, .rsa_pkcs1_sha512 => version == .tls_1_2 and self.allow_pkcs1,
            .ecdsa_secp256r1_sha256 => self.key_type == .p256 or (version == .tls_1_2 and self.key_type == .p384),
            .ecdsa_secp384r1_sha384 => self.key_type == .p384 or (version == .tls_1_2 and self.key_type == .p256),
            .ecdsa_secp521r1_sha512 => version == .tls_1_2 and (self.key_type == .p256 or self.key_type == .p384),
            .ed25519 => self.key_type == .ed25519,
            else => false,
        };
    }
    fn algorithms(self: *@This(), decoder: *tls.Decoder, version: tls.ProtocolVersion, certificate_type: bool) !void {
        try decoder.ensure(2);
        const size = decoder.decode(u16);
        if (size == 0 or size % 2 != 0) return error.TlsDecodeError;
        var list = try decoder.sub(size);
        while (!list.eof()) {
            const scheme = list.decode(tls.SignatureScheme);
            if (certificate_type and self.scheme == null and self.compatible(scheme, version)) self.scheme = scheme;
        }
    }
    pub fn receiveRequest(self: *@This(), version: tls.ProtocolVersion, decoder: *tls.Decoder) !void {
        if (self.requested) return error.TlsUnexpectedMessage;
        self.requested = true;
        if (version == .tls_1_3) {
            try decoder.ensure(1);
            const size = decoder.decode(u8);
            // This client negotiates authentication during the initial
            // handshake; it does not advertise post-handshake authentication.
            if (size != 0) return error.TlsIllegalParameter;
            self.context = "";
            try decoder.ensure(2);
            const extension_size = decoder.decode(u16);
            var extensions = try decoder.sub(extension_size);
            var has_algorithms = false;
            var seen = std.StaticBitSet(65536).empty;
            while (!extensions.eof()) {
                try extensions.ensure(4);
                const kind = extensions.decode(tls.ExtensionType);
                const length = extensions.decode(u16);
                const extension_index: usize = @backingInt(kind);
                if (seen.isSet(extension_index)) return error.TlsIllegalParameter;
                seen.set(extension_index);
                var extension = try extensions.sub(length);
                if (kind == .signature_algorithms) {
                    has_algorithms = true;
                    try self.algorithms(&extension, version, true);
                    if (!extension.eof()) return error.TlsDecodeError;
                }
            }
            if (!has_algorithms) return error.TlsIllegalParameter;
        } else {
            try decoder.ensure(1);
            const size = decoder.decode(u8);
            if (size == 0) return error.TlsDecodeError;
            var types = try decoder.sub(size);
            var certificate_type = false;
            while (!types.eof()) {
                const kind = types.decode(u8);
                const rsa = self.key_type == .rsa or self.key_type == .rsa_pss;
                if ((kind == 1 and rsa) or (kind == 64 and !rsa)) certificate_type = true;
            }
            try self.algorithms(decoder, version, certificate_type);
            try decoder.ensure(2);
            const authorities_size = decoder.decode(u16);
            var authorities = try decoder.sub(authorities_size);
            while (!authorities.eof()) {
                try authorities.ensure(2);
                const length = authorities.decode(u16);
                if (length == 0) return error.TlsDecodeError;
                _ = try authorities.sub(length);
            }
        }
        if (!decoder.eof()) return error.TlsDecodeError;
    }
    pub fn certificateMessage(self: @This(), version: tls.ProtocolVersion) ![]const u8 {
        var entries = std.Io.Writer.Allocating.init(self.allocator);
        defer entries.deinit();
        if (self.scheme != null) for (self.certificates) |der| {
            const length = std.math.cast(u24, der.len) orelse return error.MessageTooLong;
            entries.writer.writeInt(u24, length, .big) catch return error.OutOfMemory;
            entries.writer.writeAll(der) catch return error.OutOfMemory;
            if (version == .tls_1_3) entries.writer.writeInt(u16, 0, .big) catch return error.OutOfMemory;
        };
        const entries_len = std.math.cast(u24, entries.written().len) orelse return error.MessageTooLong;
        const body_len = std.math.cast(u24, entries.written().len + 3 + (if (version == .tls_1_3) self.context.len + 1 else @as(usize, 0))) orelse return error.MessageTooLong;
        var message = std.Io.Writer.Allocating.init(self.allocator);
        defer message.deinit();
        message.writer.writeByte(@backingInt(tls.HandshakeType.certificate)) catch return error.OutOfMemory;
        message.writer.writeInt(u24, body_len, .big) catch return error.OutOfMemory;
        if (version == .tls_1_3) {
            message.writer.writeByte(@intCast(self.context.len)) catch return error.OutOfMemory;
            message.writer.writeAll(self.context) catch return error.OutOfMemory;
        }
        message.writer.writeInt(u24, entries_len, .big) catch return error.OutOfMemory;
        message.writer.writeAll(entries.written()) catch return error.OutOfMemory;
        return message.toOwnedSlice();
    }
    pub fn verifyMessage(self: @This(), message: []const u8) ![]const u8 {
        const scheme = self.scheme orelse return error.TlsBadSignatureScheme;
        const Sha = std.crypto.hash.sha2;
        const signature = try switch (scheme) {
            .rsa_pss_rsae_sha256, .rsa_pss_pss_sha256 => signing.rsa(Sha.Sha256, self.allocator, self.io, self.private_key, message, .pss),
            .rsa_pss_rsae_sha384, .rsa_pss_pss_sha384 => signing.rsa(Sha.Sha384, self.allocator, self.io, self.private_key, message, .pss),
            .rsa_pss_rsae_sha512, .rsa_pss_pss_sha512 => signing.rsa(Sha.Sha512, self.allocator, self.io, self.private_key, message, .pss),
            .rsa_pkcs1_sha256 => signing.rsa(Sha.Sha256, self.allocator, self.io, self.private_key, message, .pkcs1),
            .rsa_pkcs1_sha384 => signing.rsa(Sha.Sha384, self.allocator, self.io, self.private_key, message, .pkcs1),
            .rsa_pkcs1_sha512 => signing.rsa(Sha.Sha512, self.allocator, self.io, self.private_key, message, .pkcs1),
            .ecdsa_secp256r1_sha256 => if (self.key_type == .p256) signing.ecdsa(std.crypto.sign.ecdsa.EcdsaP256Sha256, self.allocator, self.io, self.private_key, message) else signing.ecdsa(std.crypto.sign.ecdsa.Ecdsa(std.crypto.ecc.P384, Sha.Sha256), self.allocator, self.io, self.private_key, message),
            .ecdsa_secp384r1_sha384 => if (self.key_type == .p384) signing.ecdsa(std.crypto.sign.ecdsa.EcdsaP384Sha384, self.allocator, self.io, self.private_key, message) else signing.ecdsa(std.crypto.sign.ecdsa.Ecdsa(std.crypto.ecc.P256, Sha.Sha384), self.allocator, self.io, self.private_key, message),
            .ecdsa_secp521r1_sha512 => if (self.key_type == .p256) signing.ecdsa(std.crypto.sign.ecdsa.Ecdsa(std.crypto.ecc.P256, Sha.Sha512), self.allocator, self.io, self.private_key, message) else signing.ecdsa(std.crypto.sign.ecdsa.Ecdsa(std.crypto.ecc.P384, Sha.Sha512), self.allocator, self.io, self.private_key, message),
            .ed25519 => signing.ed25519(self.allocator, self.io, self.private_key, message),
            else => return error.TlsBadSignatureScheme,
        };
        defer self.allocator.free(signature);
        const signature_len = std.math.cast(u16, signature.len) orelse return error.MessageTooLong;
        var output = std.Io.Writer.Allocating.init(self.allocator);
        defer output.deinit();
        output.writer.writeByte(@backingInt(tls.HandshakeType.certificate_verify)) catch return error.OutOfMemory;
        output.writer.writeInt(u24, @as(u24, signature_len) + 4, .big) catch return error.OutOfMemory;
        output.writer.writeInt(u16, @backingInt(scheme), .big) catch return error.OutOfMemory;
        output.writer.writeInt(u16, signature_len, .big) catch return error.OutOfMemory;
        output.writer.writeAll(signature) catch return error.OutOfMemory;
        return output.toOwnedSlice();
    }
};
pub fn writeClear(output: *std.Io.Writer, message: []const u8) !void {
    var offset: usize = 0;
    while (offset < message.len) {
        const count = @min(message.len - offset, 16384);
        try output.writeByte(@backingInt(tls.ContentType.handshake));
        try output.writeInt(u16, @backingInt(tls.ProtocolVersion.tls_1_2), .big);
        try output.writeInt(u16, @intCast(count), .big);
        try output.writeAll(message[offset..][0..count]);
        offset += count;
    }
}
pub fn writeEncrypted(comptime P: type, a: std.mem.Allocator, output: *std.Io.Writer, pv: anytype, sequence: *u64, message: []const u8) !void {
    const clear = try a.alloc(u8, @min(message.len, 16384) + 1);
    defer a.free(clear);
    const record = try a.alloc(u8, 5 + clear.len + P.AEAD.tag_length);
    defer a.free(record);
    var offset: usize = 0;
    while (offset < message.len) {
        if (sequence.* == std.math.maxInt(u64)) return error.TlsSequenceOverflow;
        const count = @min(message.len - offset, 16384);
        @memcpy(clear[0..count], message[offset..][0..count]);
        clear[count] = @backingInt(tls.ContentType.handshake);
        const size = count + 1 + P.AEAD.tag_length;
        var header = std.Io.Writer.fixed(record[0..5]);
        try header.writeByte(@backingInt(tls.ContentType.application_data));
        try header.writeInt(u16, @backingInt(tls.ProtocolVersion.tls_1_2), .big);
        try header.writeInt(u16, @intCast(size), .big);
        var nonce = pv.client_handshake_iv;
        var counter: [8]u8 = undefined;
        std.mem.writeInt(u64, &counter, sequence.*, .big);
        for (nonce[nonce.len - 8 ..], counter) |*byte, part| byte.* ^= part;
        P.AEAD.encrypt(record[5..][0 .. count + 1], record[5 + count + 1 ..][0..P.AEAD.tag_length], clear[0 .. count + 1], record[0..5], nonce, pv.client_handshake_key);
        try output.writeAll(record[0 .. 5 + size]);
        sequence.* += 1;
        offset += count;
    }
}
