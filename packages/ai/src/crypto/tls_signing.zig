//! Client CertificateVerify signing. ASN.1, hashes, scalar operations and
//! private modular exponentiation use Zig's standard crypto implementations.
const std = @import("std");
const keys = @import("keys.zig");
const asn1 = std.crypto.codecs.asn1;
const Modulus = std.crypto.ff.Modulus(16384);
pub const Padding = enum { pkcs1, pss };

fn transform(a: std.mem.Allocator, io: std.Io, key: *const keys.RsaPrivate, modulus: Modulus, encoded: []const u8) ![]u8 {
    const value = try Modulus.Fe.fromBytes(modulus, encoded, .big);
    var exponent: [2048]u8 = undefined;
    std.mem.writeInt(u16384, &exponent, key.privateExponent, .big);
    defer std.crypto.secureZero(u8, &exponent);
    const size = modulus.encodedLen();
    const exponent_size = @max(size, @divCeil(@as(usize, 16384 - @clz(key.privateExponent)), 8));
    const signature = try modulus.powWithEncodedExponent(value, exponent[exponent.len - exponent_size ..], .big);
    var public_exponent: [2048]u8 = undefined;
    std.mem.writeInt(u16384, &public_exponent, key.publicExponent, .big);
    const checked = try modulus.powWithEncodedPublicExponent(signature, std.mem.trimStart(u8, &public_exponent, &.{0}), .big);
    if (!checked.eql(value)) return error.InvalidPrivateKey;
    const result = try a.alloc(u8, size);
    errdefer a.free(result);
    try signature.toBytes(result, .big);
    try io.checkCancel();
    return result;
}
fn hashOid(comptime Hash: type) asn1.Oid {
    return if (Hash == std.crypto.hash.sha2.Sha256) asn1.Oid.fromDotComptime("2.16.840.1.101.3.4.2.1") else if (Hash == std.crypto.hash.sha2.Sha384) asn1.Oid.fromDotComptime("2.16.840.1.101.3.4.2.2") else if (Hash == std.crypto.hash.sha2.Sha512) asn1.Oid.fromDotComptime("2.16.840.1.101.3.4.2.3") else @compileError("Unsupported TLS RSA digest");
}
pub fn rsa(comptime Hash: type, a: std.mem.Allocator, io: std.Io, pem: []const u8, message: []const u8, padding: Padding) ![]u8 {
    try io.checkCancel();
    if (padding == .pss) {
        const rules = try keys.rsaPssPrivateRules(a, io, pem);
        const kind: keys.RsaHash = if (Hash == std.crypto.hash.sha2.Sha256) .sha256 else if (Hash == std.crypto.hash.sha2.Sha384) .sha384 else if (Hash == std.crypto.hash.sha2.Sha512) .sha512 else @compileError("Unsupported TLS RSA digest");
        if (!rules.allows(kind, Hash.digest_length)) return error.TlsBadSignatureScheme;
    }
    var key = if (padding == .pss) try keys.rsaPrivatePss(a, io, pem) else try keys.rsaPrivate(a, io, pem);
    defer std.crypto.secureZero(u8, std.mem.asBytes(&key));
    var modulus_bytes: [2048]u8 = undefined;
    std.mem.writeInt(u16384, &modulus_bytes, key.modulus, .big);
    const modulus = try Modulus.fromBytes(&modulus_bytes, .big);
    var digest: [Hash.digest_length]u8 = undefined;
    Hash.hash(message, &digest, .{});
    const encoded = if (padding == .pkcs1) try pkcs1(Hash, a, modulus.encodedLen(), &digest) else try pss(Hash, a, io, modulus.bits() - 1, &digest);
    defer a.free(encoded);
    return transform(a, io, &key, modulus, encoded);
}
fn pkcs1(comptime Hash: type, a: std.mem.Allocator, size: usize, digest: *const [Hash.digest_length]u8) ![]u8 {
    const Octets = asn1.Opaque(.universal(.octetstring, false));
    const info = try asn1.der.encode(a, .{ .algorithm = .{ .oid = hashOid(Hash), .parameters = asn1.Any{ .tag = .universal(.null, false), .bytes = "" } }, .digest = Octets{ .bytes = digest } });
    defer a.free(info);
    if (size < info.len + 11) return error.InvalidPrivateKey;
    const encoded = try a.alloc(u8, size);
    encoded[0] = 0;
    encoded[1] = 1;
    const boundary = size - info.len - 1;
    @memset(encoded[2..boundary], 0xff);
    encoded[boundary] = 0;
    @memcpy(encoded[boundary + 1 ..], info);
    return encoded;
}
fn pss(comptime Hash: type, a: std.mem.Allocator, io: std.Io, bits: usize, digest: *const [Hash.digest_length]u8) ![]u8 {
    return pssWithSalt(Hash, Hash, a, io, bits, digest, Hash.digest_length);
}
fn pssWithSalt(comptime Hash: type, comptime MaskHash: type, a: std.mem.Allocator, io: std.Io, bits: usize, digest: *const [Hash.digest_length]u8, salt_length: usize) ![]u8 {
    const size = std.math.divCeil(usize, bits, 8) catch return error.InvalidPrivateKey;
    if (size < Hash.digest_length + 2 or salt_length > size - Hash.digest_length - 2) return error.InvalidPrivateKey;
    const salt = try a.alloc(u8, salt_length);
    defer {
        std.crypto.secureZero(u8, salt);
        a.free(salt);
    }
    try io.randomSecure(salt);
    var hash = Hash.init(.{});
    hash.update(&@as([8]u8, @splat(0)));
    hash.update(digest);
    hash.update(salt);
    var h: [Hash.digest_length]u8 = undefined;
    hash.final(&h);
    const encoded = try a.alloc(u8, size);
    errdefer a.free(encoded);
    const db = encoded[0 .. size - Hash.digest_length - 1];
    @memset(db, 0);
    db[db.len - salt.len - 1] = 1;
    @memcpy(db[db.len - salt.len ..], salt);
    var offset: usize = 0;
    var counter: u32 = 0;
    while (offset < db.len) : (counter += 1) {
        var counter_bytes: [4]u8 = undefined;
        std.mem.writeInt(u32, &counter_bytes, counter, .big);
        var block_hash = MaskHash.init(.{});
        block_hash.update(&h);
        block_hash.update(&counter_bytes);
        var mask: [MaskHash.digest_length]u8 = undefined;
        block_hash.final(&mask);
        const count = @min(mask.len, db.len - offset);
        for (db[offset..][0..count], mask[0..count]) |*byte, mask_byte| byte.* ^= mask_byte;
        offset += count;
    }
    const unused: u3 = @intCast(8 * size - bits);
    db[0] &= @as(u8, 0xff) >> unused;
    @memcpy(encoded[db.len..][0..h.len], &h);
    encoded[encoded.len - 1] = 0xbc;
    return encoded;
}
/// Explicit maximum-salt SHA256 signing, retaining key MGF1 restrictions.
pub fn rsaPssMaximum(a: std.mem.Allocator, io: std.Io, pem: []const u8, message: []const u8) ![]u8 {
    return rsaPssGoogle(a, io, pem, message, -3);
}
/// Google/Node SHA256 PSS permits a concrete salt length, digest length (-1),
/// or maximal signing length (-2/-3). Key restrictions remain in force.
pub fn rsaPssGoogle(a: std.mem.Allocator, io: std.Io, pem: []const u8, message: []const u8, requested_salt: ?i32) ![]u8 {
    try io.checkCancel();
    const Hash = std.crypto.hash.sha2.Sha256;
    const rules = try keys.rsaPssPrivateRules(a, io, pem);
    if ((rules.hash != null and rules.hash.? != .sha256) or rules.trailer != 1) return error.UnsupportedGoogleSigningKey;
    if (rules.min_salt < 0) return error.InvalidPrivateKey;
    const minimum: usize = @intCast(rules.min_salt);
    var key = try keys.rsaPrivatePss(a, io, pem);
    defer std.crypto.secureZero(u8, std.mem.asBytes(&key));
    var modulus_bytes: [2048]u8 = undefined;
    std.mem.writeInt(u16384, &modulus_bytes, key.modulus, .big);
    const modulus = try Modulus.fromBytes(&modulus_bytes, .big);
    const bits = modulus.bits() - 1;
    const size = std.math.divCeil(usize, bits, 8) catch return error.InvalidPrivateKey;
    if (size < Hash.digest_length + 2) return error.InvalidPrivateKey;
    const maximum = size - Hash.digest_length - 2;
    const salt_length: usize = if (requested_salt) |length| switch (length) {
        -1 => Hash.digest_length,
        -2, -3 => maximum,
        else => if (length < 0) return error.UnsupportedGoogleSigningSaltLength else @intCast(length),
    } else if (rules.hash != null) minimum else maximum;
    if (salt_length > maximum or salt_length < minimum) return error.InvalidPrivateKey;
    var digest: [Hash.digest_length]u8 = undefined;
    Hash.hash(message, &digest, .{});
    const encoded = switch (rules.mask_hash orelse .sha256) {
        .sha1 => try pssWithSalt(Hash, std.crypto.hash.Sha1, a, io, bits, &digest, salt_length),
        .sha224 => try pssWithSalt(Hash, std.crypto.hash.sha2.Sha224, a, io, bits, &digest, salt_length),
        .sha256 => try pssWithSalt(Hash, Hash, a, io, bits, &digest, salt_length),
        .sha384 => try pssWithSalt(Hash, std.crypto.hash.sha2.Sha384, a, io, bits, &digest, salt_length),
        .sha512 => try pssWithSalt(Hash, std.crypto.hash.sha2.Sha512, a, io, bits, &digest, salt_length),
        .sha512_224 => try pssWithSalt(Hash, std.crypto.hash.sha2.Sha512_224, a, io, bits, &digest, salt_length),
        .sha512_256 => try pssWithSalt(Hash, std.crypto.hash.sha2.Sha512_256, a, io, bits, &digest, salt_length),
        .unsupported => return error.UnsupportedGoogleSigningKey,
    };
    defer a.free(encoded);
    return transform(a, io, &key, modulus, encoded);
}
pub fn rsaX931Google(a: std.mem.Allocator, io: std.Io, pem: []const u8, message: []const u8) ![]u8 {
    try io.checkCancel();
    var key = try keys.rsaPrivate(a, io, pem);
    defer std.crypto.secureZero(u8, std.mem.asBytes(&key));
    const modulus = try Modulus.fromPrimitive(u16384, key.modulus);
    var digest: [32]u8 = undefined;
    std.crypto.hash.sha2.Sha256.hash(message, &digest, .{});
    const size = modulus.encodedLen();
    if (size < digest.len + 3) return error.InvalidPrivateKey;
    const encoded = try a.alloc(u8, size);
    defer a.free(encoded);
    var writer = std.Io.Writer.fixed(encoded);
    const padding = size - digest.len - 3;
    if (padding == 0) try writer.writeByte(0x6a) else {
        try writer.writeByte(0x6b);
        try writer.splatByteAll(0xbb, padding - 1);
        try writer.writeByte(0xba);
    }
    try writer.writeAll(&digest);
    try writer.writeByte(0x34);
    try writer.writeByte(0xcc);
    const result = try transform(a, io, &key, modulus, encoded);
    errdefer a.free(result);
    const value = try (try std.crypto.ff.Uint(16384).fromBytes(result, .big)).toPrimitive(u16384);
    const complement = key.modulus - value;
    if (value > complement) try (try std.crypto.ff.Uint(16384).fromPrimitive(u16384, complement)).toBytes(result, .big);
    try io.checkCancel();
    return result;
}
pub fn ecdsa(comptime Scheme: type, a: std.mem.Allocator, io: std.Io, pem: []const u8, message: []const u8) ![]u8 {
    try io.checkCancel();
    var key = try keys.ecPrivateFor(Scheme, a, io, pem);
    defer std.crypto.secureZero(u8, std.mem.asBytes(&key.secret_key));
    var scalar_wide: [64]u8 = @splat(0);
    defer std.crypto.secureZero(u8, &scalar_wide);
    @memcpy(scalar_wide[scalar_wide.len - Scheme.SecretKey.encoded_length ..], &key.secret_key.toBytes());
    key.secret_key = try Scheme.SecretKey.fromBytes(Scheme.Curve.scalar.reduce64(scalar_wide, .big));
    var noise: [Scheme.noise_length]u8 = undefined;
    defer std.crypto.secureZero(u8, &noise);
    try io.randomSecure(&noise);
    var signer = try key.signer(noise);
    defer std.crypto.secureZero(u8, std.mem.asBytes(&signer));
    signer.update(message);
    const signature = try signer.finalize();
    var encoded: [Scheme.Signature.der_encoded_length_max]u8 = undefined;
    const result = try a.dupe(u8, signature.toDer(&encoded));
    errdefer a.free(result);
    try io.checkCancel();
    return result;
}
pub fn ed25519(a: std.mem.Allocator, io: std.Io, pem: []const u8, message: []const u8) ![]u8 {
    try io.checkCancel();
    var key = try keys.ed25519Private(a, io, pem);
    defer std.crypto.secureZero(u8, std.mem.asBytes(&key.secret_key));
    var noise: [std.crypto.sign.Ed25519.noise_length]u8 = undefined;
    defer std.crypto.secureZero(u8, &noise);
    try io.randomSecure(&noise);
    const signature = try key.sign(message, noise);
    const result = try a.dupe(u8, &signature.toBytes());
    errdefer a.free(result);
    try io.checkCancel();
    return result;
}
