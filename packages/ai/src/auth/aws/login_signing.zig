//! AWS login ES256 proof using the standard P-256/SHA-256 implementation.
const std = @import("std");
const Scheme = std.crypto.sign.ecdsa.EcdsaP256Sha256;
fn publicSpki(a: std.mem.Allocator, point: []const u8, named: bool, explicit_seed: bool) ![]u8 {
    const asn1 = std.crypto.codecs.asn1;
    const ec_oid = asn1.Oid.fromDotComptime("1.2.840.10045.2.1");
    if (named) return asn1.der.encode(a, .{ .algorithm = .{ .oid = ec_oid, .parameters = asn1.Oid.fromDotComptime("1.2.840.10045.3.1.7") }, .key = asn1.BitString{ .bytes = point } });
    const Curve = Scheme.Curve;
    const Octets = asn1.Opaque(.universal(.octetstring, false));
    var coefficient_a: [32]u8 = undefined;
    std.mem.writeInt(u256, &coefficient_a, Curve.Fe.field_order - 3, .big);
    var coefficient_b: [32]u8 = undefined;
    std.mem.writeInt(u256, &coefficient_b, Curve.B.toInt(), .big);
    const generator = (Scheme.PublicKey{ .p = Curve.basePoint }).toUncompressedSec1();
    const seed = [_]u8{ 0xc4, 0x9d, 0x36, 0x08, 0x86, 0xe7, 0x04, 0x93, 0x6a, 0x66, 0x78, 0xe1, 0x13, 0x9d, 0x26, 0xb7, 0x81, 0x9f, 0x7e, 0x90 };
    return asn1.der.encode(a, .{ .algorithm = .{ .oid = ec_oid, .parameters = .{
        .version = @as(u8, 1),
        .field = .{ .kind = asn1.Oid.fromDotComptime("1.2.840.10045.1.1"), .prime = @as(u256, Curve.Fe.field_order) },
        .curve = .{ .a = Octets{ .bytes = &coefficient_a }, .b = Octets{ .bytes = &coefficient_b }, .seed = if (explicit_seed) @as(?asn1.BitString, .{ .bytes = &seed }) else null },
        .base = Octets{ .bytes = &generator },
        .order = @as(u256, Curve.scalar.field_order),
        .cofactor = @as(u8, 1),
    } }, .key = asn1.BitString{ .bytes = point } });
}
pub const Key = struct {
    pair: Scheme.KeyPair,
    coordinates: [64]u8,
    xLength: usize = 32,
    yLength: usize = 32,
    pub fn open(a: std.mem.Allocator, io: std.Io, pem: []const u8) !@This() {
        try io.checkCancel();
        var imported = try @import("../../crypto/keys.zig").p256PrivateImport(a, io, pem);
        defer std.crypto.secureZero(u8, std.mem.asBytes(&imported.pair.secret_key));
        var point = imported.pair.public_key.toUncompressedSec1();
        var result = @This(){ .pair = imported.pair, .coordinates = point[1..65].* };
        errdefer result.deinit();
        {
            const compressed = imported.pair.public_key.toCompressedSec1();
            const identity = if (imported.pair.public_key.p.rejectIdentity()) |_| false else |_| true;
            if (imported.pointForm == 6) point[0] = 6 | (point[64] & 1);
            const encoded_point: []const u8 = if (identity) &.{0} else if (imported.pointForm == 2) &compressed else &point;
            const public_der = try publicSpki(a, encoded_point, imported.namedParameters, imported.explicitSeed);
            defer a.free(public_der);
            const start = if (std.mem.findScalar(u8, public_der, 4)) |offset| offset + 1 else 0;
            const x_bytes = public_der[@min(start, public_der.len)..@min(start + 32, public_der.len)];
            const y_bytes = public_der[@min(start + 32, public_der.len)..@min(start + 64, public_der.len)];
            @memcpy(result.coordinates[0..x_bytes.len], x_bytes);
            @memcpy(result.coordinates[32..][0..y_bytes.len], y_bytes);
            result.xLength = x_bytes.len;
            result.yLength = y_bytes.len;
        }
        try io.checkCancel();
        return result;
    }
    pub fn deinit(self: *@This()) void {
        std.crypto.secureZero(u8, std.mem.asBytes(&self.pair.secret_key));
        self.* = undefined;
    }
    pub fn x(self: *const @This()) ![]const u8 {
        return self.coordinates[0..self.xLength];
    }
    pub fn y(self: *const @This()) ![]const u8 {
        return self.coordinates[32..][0..self.yLength];
    }
    pub fn sign(self: *const @This(), io: std.Io, message: []const u8) ![64]u8 {
        try io.checkCancel();
        var key = self.pair;
        defer std.crypto.secureZero(u8, std.mem.asBytes(&key.secret_key));
        var secret = key.secret_key.toBytes();
        defer std.crypto.secureZero(u8, &secret);
        var wide: [64]u8 = @splat(0);
        defer std.crypto.secureZero(u8, &wide);
        @memcpy(wide[wide.len - secret.len ..], &secret);
        // The source's EC signing multiplication reduces the imported integer;
        // std requires a canonical scalar. Keep normalization operation-owned.
        var reduced = Scheme.Curve.scalar.reduce64(wide, .big);
        defer std.crypto.secureZero(u8, &reduced);
        key.secret_key = try Scheme.SecretKey.fromBytes(reduced);
        var noise: [Scheme.noise_length]u8 = undefined;
        defer std.crypto.secureZero(u8, &noise);
        try io.randomSecure(&noise);
        var signer = try key.signer(noise);
        defer std.crypto.secureZero(u8, std.mem.asBytes(&signer));
        signer.update(message);
        const signature = try signer.finalize();
        try io.checkCancel();
        return signature.toBytes();
    }
};
