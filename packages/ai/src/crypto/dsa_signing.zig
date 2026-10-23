const std = @import("std");
const keys = @import("keys.zig");
const Modulus = std.crypto.ff.Modulus(16384);
const Uint = std.crypto.ff.Uint(16384);

fn nonce(io: std.Io, q: Modulus, private: *const u16384, message: []const u8) !Modulus.Fe {
    var scalar = std.math.cast(u768, private.*) orelse return error.GoogleSigningPrivateKeyTooLarge;
    defer std.crypto.secureZero(u8, std.mem.asBytes(&scalar));
    var private_bytes: [96]u8 = undefined;
    std.mem.writeInt(u768, &private_bytes, scalar, .big);
    defer std.crypto.secureZero(u8, &private_bytes);
    var random: [64]u8 = undefined;
    defer std.crypto.secureZero(u8, &random);
    var digest: [64]u8 = undefined;
    defer std.crypto.secureZero(u8, &digest);
    var candidate: [2048]u8 = undefined;
    defer std.crypto.secureZero(u8, &candidate);
    const size = q.encodedLen();
    const excess: u3 = @intCast(size * 8 - q.bits());
    for (0..64) |_| {
        var offset: usize = 0;
        var counter: u8 = 0;
        while (offset < size) : (counter += 1) {
            try io.randomSecure(&random);
            var hash = std.crypto.hash.sha2.Sha512.init(.{});
            defer std.crypto.secureZero(u8, std.mem.asBytes(&hash));
            hash.update(&.{counter});
            hash.update(&private_bytes);
            hash.update(message);
            hash.update(&random);
            hash.final(&digest);
            const count = @min(digest.len, size - offset);
            @memcpy(candidate[offset..][0..count], digest[0..count]);
            offset += count;
        }
        candidate[0] &= @as(u8, 0xff) >> excess;
        var value = Modulus.Fe.fromBytes(q, candidate[0..size], .big) catch |err| switch (err) {
            error.NonCanonical => continue,
            else => return err,
        };
        defer std.crypto.secureZero(u8, std.mem.asBytes(&value));
        if (!value.isZero()) return value;
    }
    return error.GoogleSigningNonceGenerationFailed;
}
fn blinding(io: std.Io, q: Modulus) !Modulus.Fe {
    const bits = q.bits() - 1;
    const size = (bits + 7) / 8;
    const excess: u3 = @intCast(size * 8 - bits);
    var candidate: [2048]u8 = undefined;
    defer std.crypto.secureZero(u8, &candidate);
    for (0..100) |_| {
        try io.randomSecure(candidate[0..size]);
        candidate[0] &= @as(u8, 0xff) >> excess;
        var value = try Modulus.Fe.fromBytes(q, candidate[0..size], .big);
        defer std.crypto.secureZero(u8, std.mem.asBytes(&value));
        if (!value.isZero()) return value;
    }
    return error.GoogleSigningNonceGenerationFailed;
}
pub fn sign(a: std.mem.Allocator, io: std.Io, pem: []const u8, message: []const u8) ![]u8 {
    return signWithEncoding(a, io, pem, message, false);
}
pub fn signP1363(a: std.mem.Allocator, io: std.Io, pem: []const u8, message: []const u8) ![]u8 {
    return signWithEncoding(a, io, pem, message, true);
}
fn signWithEncoding(a: std.mem.Allocator, io: std.Io, pem: []const u8, message: []const u8, p1363: bool) ![]u8 {
    try io.checkCancel();
    var key = try keys.dsaPrivate(a, io, pem);
    defer std.crypto.secureZero(u8, std.mem.asBytes(&key));
    if (key.g == 0) return error.InvalidPrivateKey;
    const p = try Modulus.fromPrimitive(u16384, key.p);
    const q = try Modulus.fromPrimitive(u16384, key.q);
    if (q.bits() < 128) return error.InvalidPrivateKey;
    const generator = p.reduce(try Uint.fromPrimitive(u16384, key.g));
    var private = q.reduce(try Uint.fromPrimitive(u16384, key.x));
    defer std.crypto.secureZero(u8, std.mem.asBytes(&private));
    var digest: [32]u8 = undefined;
    std.crypto.hash.sha2.Sha256.hash(message, &digest, .{});
    defer std.crypto.secureZero(u8, &digest);
    var digest_length: usize = digest.len;
    var inverse_exponent: [2048]u8 = undefined;
    std.mem.writeInt(u16384, &inverse_exponent, key.q - 2, .big);
    const inverse_bytes = inverse_exponent[inverse_exponent.len - q.encodedLen() ..];
    for (0..10) |_| {
        try io.checkCancel();
        var k = try nonce(io, q, &key.x, digest[0..digest_length]);
        defer std.crypto.secureZero(u8, std.mem.asBytes(&k));
        var encoded_k: [2048]u8 = undefined;
        defer std.crypto.secureZero(u8, &encoded_k);
        const k_bytes = encoded_k[encoded_k.len - q.encodedLen() ..];
        try k.toBytes(k_bytes, .big);
        var power = try p.powWithEncodedExponent(generator, k_bytes, .big);
        defer std.crypto.secureZero(u8, std.mem.asBytes(&power));
        const r = q.reduce(try Uint.fromPrimitive(u16384, try power.toPrimitive(u16384)));
        var inverse = try q.powWithEncodedPublicExponent(k, inverse_bytes, .big);
        defer std.crypto.secureZero(u8, std.mem.asBytes(&inverse));
        digest_length = @min(digest_length, q.encodedLen());
        const m = q.reduce(try Uint.fromBytes(digest[0..digest_length], .big));
        var blind = try blinding(io, q);
        defer std.crypto.secureZero(u8, std.mem.asBytes(&blind));
        var blind_inverse = try q.powWithEncodedPublicExponent(blind, inverse_bytes, .big);
        defer std.crypto.secureZero(u8, std.mem.asBytes(&blind_inverse));
        if (!q.mul(blind, blind_inverse).eql(q.one())) return error.InvalidPrivateKey;
        var product = q.mul(q.mul(blind, private), r);
        defer std.crypto.secureZero(u8, std.mem.asBytes(&product));
        var blinded_message = q.mul(blind, m);
        defer std.crypto.secureZero(u8, std.mem.asBytes(&blinded_message));
        const s = q.mul(q.mul(q.add(product, blinded_message), inverse), blind_inverse);
        if (r.isZero() or s.isZero()) continue;
        const encoded = if (p1363) raw: {
            const size = q.encodedLen();
            const bytes = try a.alloc(u8, size * 2);
            errdefer a.free(bytes);
            try r.toBytes(bytes[0..size], .big);
            try s.toBytes(bytes[size..], .big);
            break :raw bytes;
        } else try std.crypto.codecs.asn1.der.encode(a, .{ .r = try r.toPrimitive(u16384), .s = try s.toPrimitive(u16384) });
        errdefer a.free(encoded);
        try io.checkCancel();
        return encoded;
    }
    return error.InvalidPrivateKey;
}
