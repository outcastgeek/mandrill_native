//! Key armor selection uses the standalone regex codec; ASN.1 is decoded by
//! Zig's typed DER codec. Private-key arithmetic stays in std.crypto.
const std = @import("std");
const asn1 = std.crypto.codecs.asn1;
const der = asn1.der;
fn oidEqual(a: asn1.Oid, b: asn1.Oid) bool {
    return std.mem.eql(u8, a.encoded, b.encoded);
}
const Octets = asn1.Opaque(.universal(.octetstring, false));
pub const LegacyPemCipher = enum { aes128_cbc, aes256_cbc };
pub const LegacyPemEncryption = struct { cipher: LegacyPemCipher, iv: [16]u8 };
fn legacyPemCipher(name: []const u8) !LegacyPemCipher {
    if (std.ascii.eqlIgnoreCase(name, "AES-128-CBC") or std.ascii.eqlIgnoreCase(name, "AES128")) return .aes128_cbc;
    if (std.ascii.eqlIgnoreCase(name, "AES-256-CBC") or std.ascii.eqlIgnoreCase(name, "AES256")) return .aes256_cbc;
    return error.UnsupportedPrivateKeyEncryption;
}
pub const Pem = struct { label: []const u8, bytes: []u8, encryption: ?LegacyPemEncryption = null, consumed: usize = 0 };
pub fn decodePem(a: std.mem.Allocator, io: std.Io, source: []const u8) !Pem {
    return decodePemRecord(a, io, source, false);
}
pub fn decodePrivatePem(a: std.mem.Allocator, io: std.Io, source: []const u8) !Pem {
    var remaining = source;
    while (true) {
        const material = try decodePrivatePemArmor(a, io, remaining);
        var retained = false;
        defer {
            if (!retained) {
                a.free(material.label);
                std.crypto.secureZero(u8, material.bytes);
                a.free(material.bytes);
            }
        }
        remaining = remaining[material.consumed..];
        if (material.encryption != null or std.mem.eql(u8, material.label, "ENCRYPTED PRIVATE KEY")) return error.PrivateKeyPassphraseRequired;
        if (!isPrivatePemLabel(material.label)) continue;
        retained = true;
        return material;
    }
}
pub fn isPrivatePemLabel(label: []const u8) bool {
    for ([_][]const u8{ "PRIVATE KEY", "RSA PRIVATE KEY", "EC PRIVATE KEY", "DSA PRIVATE KEY", "ENCRYPTED PRIVATE KEY" }) |candidate| {
        if (std.mem.eql(u8, label, candidate)) return true;
    }
    return false;
}
/// One raw armor record, with a byte count for advancing through a bundle.
/// Encryption must be handled before label selection; ciphertext is never
/// private DER. The caller owns label/bytes and borrows no regex storage.
pub fn decodePrivatePemArmor(a: std.mem.Allocator, io: std.Io, source: []const u8) !Pem {
    return decodePemRecord(a, io, source, true);
}
fn privatePemBody(a: std.mem.Allocator, io: std.Io, encoded: []const u8) !std.Io.Writer.Allocating {
    var trailing = try @import("../regex/regexp.zig").Regex.init(a, io, "[\\x00-\\x09\\x0b-\\x20]+(?=\\n|$)", .{});
    defer trailing.deinit();
    // Removing trailing bytes cannot exceed the captured input bound.
    var result = try std.Io.Writer.Allocating.initCapacity(a, encoded.len);
    errdefer {
        std.crypto.secureZero(u8, result.written());
        result.deinit();
    }
    var offset: usize = 0;
    while (try trailing.execBytes(a, encoded, offset)) |found| {
        var match = found;
        defer match.deinit();
        const span = match.captures[0].?;
        try result.writer.writeAll(encoded[offset..span.start]);
        offset = span.end;
    }
    try result.writer.writeAll(encoded[offset..]);
    return result;
}
fn decodePemRecord(a: std.mem.Allocator, io: std.Io, source: []const u8, comptime private_only: bool) !Pem {
    const pattern = if (private_only)
        \\(?:^|(?<=\n))-----BEGIN ([^\r\n]*)-----[\x00-\x09\x0b-\x20]*\n([\s\S]*?)(?<=\n)-----END ([^\r\n]*)-----[\x00-\x09\x0b-\x20]*(?:\n|$)
    else
        \\-----BEGIN ([A-Z0-9 ]+)-----([\s\S]*?)-----END \1-----
    ;
    var regex = try @import("../regex/regexp.zig").Regex.init(a, io, pattern, .{});
    defer regex.deinit();
    const input = if (private_only and source.len > 3 and std.mem.startsWith(u8, source, "\xef\xbb\xbf")) source[3..] else source;
    if (try (if (private_only) regex.execBytes(a, input, 0) else regex.exec(a, input, 0))) |found| {
        var match = found;
        defer match.deinit();
        const captured_label = (try match.text(a, 1)).?;
        const label = if (private_only) label: {
            defer a.free(captured_label);
            // The PEM reader compares its allocated name as a C string.
            // Keep the full capture's allocation separate from that view.
            break :label try a.dupe(u8, std.mem.sliceTo(captured_label, 0));
        } else captured_label;
        var retained = false;
        defer if (!retained) a.free(label);
        if (private_only) {
            const end_label = (try match.text(a, 3)).?;
            defer a.free(end_label);
            if (!std.mem.eql(u8, label, end_label)) return error.InvalidPem;
        }
        const encoded = (try match.text(a, 2)).?;
        defer {
            if (private_only) std.crypto.secureZero(u8, @constCast(encoded));
            a.free(encoded);
        }
        var normalized = if (private_only) try privatePemBody(a, io, encoded) else std.Io.Writer.Allocating.init(a);
        defer {
            if (private_only) std.crypto.secureZero(u8, normalized.written());
            normalized.deinit();
        }
        var body = if (private_only) normalized.written() else encoded;
        var encryption: ?LegacyPemEncryption = null;
        if (private_only) {
            var header = try @import("../regex/regexp.zig").Regex.init(a, io,
                \\^Proc-Type:[ \t]*4,[ \t]*ENCRYPTED[ \t\r]*\nDEK-Info:[ \t]*([^ \t,\r\n]+)[ \t]*,([0-9a-fA-F]{32})[^\n]*\n(?:[^\n]+\n)*\n([\s\S]*)(?![\s\S])
            , .{});
            defer header.deinit();
            if (try header.execBytes(a, body, 0)) |found_header| {
                var matched = found_header;
                defer matched.deinit();
                const cipher_span = matched.captures[1].?;
                const cipher = body[cipher_span.start..cipher_span.end];
                var info = LegacyPemEncryption{ .cipher = try legacyPemCipher(cipher), .iv = undefined };
                const iv_span = matched.captures[2].?;
                _ = try std.fmt.hexToBytes(&info.iv, body[iv_span.start..iv_span.end]);
                const payload = matched.captures[3].?;
                body = body[payload.start..payload.end];
                // Post-header data uses 64-byte lines; a short line must be
                // final and a second blank line is not accepted by the source.
                var lines = try @import("../regex/regexp.zig").Regex.init(a, io, "^(?:[^\\n]{64}\\n)*[^\\n]{1,64}\\n(?![\\s\\S])", .{});
                defer lines.deinit();
                var valid = (try lines.execBytes(a, body, 0)) orelse return error.InvalidPem;
                defer valid.deinit();
                encryption = info;
            }
        }
        const codec = std.base64.standard.decoderWithIgnore(" \t\r\n");
        const output = try a.alloc(u8, codec.calcSizeUpperBound(body.len));
        defer {
            std.crypto.secureZero(u8, output);
            a.free(output);
        }
        const count = try codec.decode(output, body);
        if (private_only and count == 0) return error.InvalidPem;
        const bytes = try a.dupe(u8, output[0..count]);
        retained = true;
        return .{ .label = label, .bytes = bytes, .encryption = encryption, .consumed = if (private_only) match.captures[0].?.end + source.len - input.len else 0 };
    }
    return error.InvalidPem;
}
const Algorithm = struct {
    oid: asn1.Oid,
    parameters: ?asn1.Any = null,
    pub fn decodeDer(decoder: *der.Decoder) !@This() {
        const sequence = try decoder.sequence();
        defer decoder.index = sequence.slice.end;
        var fields = der.Decoder{ .bytes = decoder.view(sequence) };
        const oid = try fields.any(asn1.Oid);
        const parameters: ?asn1.Any = if (fields.index < fields.bytes.len) parameters: {
            const element = try fields.element(.{});
            fields.index = element.slice.end;
            break :parameters .{ .tag = element.tag, .bytes = fields.view(element) };
        } else null;
        if (fields.index != fields.bytes.len) return error.InvalidPrivateKey;
        return .{ .oid = oid, .parameters = parameters };
    }
};
const PrivateKeyInfo = struct {
    version: i16384,
    algorithm: Algorithm,
    key: Octets,
    pub fn decodeDer(decoder: *der.Decoder) !@This() {
        const sequence = try decoder.sequence();
        defer decoder.index = sequence.slice.end;
        var fields = der.Decoder{ .bytes = decoder.view(sequence) };
        const result = @This(){ .version = try fields.any(i16384), .algorithm = try fields.any(Algorithm), .key = try fields.any(Octets) };
        if (fields.index < fields.bytes.len) {
            const attributes = try fields.element(.{ .number = @fromBackingInt(0), .constructed = true, .class = .context_specific });
            var records = der.Decoder{ .bytes = fields.view(attributes) };
            while (records.index < records.bytes.len) {
                const record = try records.sequence();
                var attribute = der.Decoder{ .bytes = records.view(record) };
                _ = try attribute.any(asn1.Oid);
                const set = try attribute.element(asn1.Tag.universal(.set, true).toExpected());
                var values = der.Decoder{ .bytes = attribute.view(set) };
                while (values.index < values.bytes.len) {
                    const value = try values.element(.{});
                    values.index = value.slice.end;
                }
                attribute.index = set.slice.end;
                if (attribute.index != attribute.bytes.len) return error.InvalidPrivateKey;
                records.index = record.slice.end;
            }
            fields.index = attributes.slice.end;
        }
        if (fields.index != fields.bytes.len) return error.InvalidPrivateKey;
        return result;
    }
};
fn optionalExplicit(comptime T: type, decoder: *der.Decoder, number: u16) !?T {
    if (decoder.index == decoder.bytes.len) return null;
    const next = try asn1.Element.decode(decoder.bytes, decoder.index);
    if (next.tag.class != .context_specific or @backingInt(next.tag.number) != number) return null;
    const element = try decoder.element(.{ .number = @fromBackingInt(number), .constructed = true, .class = .context_specific });
    var contents = der.Decoder{ .bytes = decoder.view(element) };
    const result = try contents.any(T);
    if (contents.index != contents.bytes.len) return error.InvalidPrivateKey;
    decoder.index = element.slice.end;
    return result;
}
fn decode(comptime T: type, bytes: []const u8) !T {
    var decoder = der.Decoder{ .bytes = bytes };
    return decoder.any(T);
}
pub fn validatePrivateKeyInfo(bytes: []const u8) !void {
    const container = try decode(PrivateKeyInfo, bytes);
    if (container.key.bytes.len == 0) return error.InvalidPrivateKey;
}
fn completeSequence(comptime T: type, decoder: *der.Decoder) !T {
    const sequence = try decoder.sequence();
    defer decoder.index = sequence.slice.end;
    var fields = der.Decoder{ .bytes = decoder.view(sequence) };
    var result: T = undefined;
    errdefer std.crypto.secureZero(u8, std.mem.asBytes(&result));
    const info = @typeInfo(T).@"struct";
    inline for (info.field_names, info.field_types) |name, Field| @field(result, name) = try fields.any(Field);
    if (fields.index != fields.bytes.len) return error.InvalidPrivateKey;
    return result;
}
const DsaParameters = struct {
    p: u16384,
    q: u16384,
    g: u16384,
    pub fn decodeDer(decoder: *der.Decoder) !@This() {
        return completeSequence(@This(), decoder);
    }
};
pub const DsaPrivate = struct {
    version: i32,
    p: u16384,
    q: u16384,
    g: u16384,
    y: u16384,
    x: u16384,
    pub fn decodeDer(decoder: *der.Decoder) !@This() {
        return completeSequence(@This(), decoder);
    }
};
pub fn isPrivateDsa(a: std.mem.Allocator, io: std.Io, source: []const u8) !bool {
    const material = try decodePrivatePem(a, io, source);
    defer a.free(material.label);
    defer {
        std.crypto.secureZero(u8, material.bytes);
        a.free(material.bytes);
    }
    if (std.mem.eql(u8, material.label, "DSA PRIVATE KEY")) return true;
    if (!std.mem.eql(u8, material.label, "PRIVATE KEY")) return false;
    return oidEqual((try decode(PrivateKeyInfo, material.bytes)).algorithm.oid, asn1.Oid.fromDotComptime("1.2.840.10040.4.1"));
}
pub fn dsaPrivate(a: std.mem.Allocator, io: std.Io, source: []const u8) !DsaPrivate {
    try io.checkCancel();
    const material = try decodePrivatePem(a, io, source);
    defer a.free(material.label);
    defer {
        std.crypto.secureZero(u8, material.bytes);
        a.free(material.bytes);
    }
    if (std.mem.eql(u8, material.label, "DSA PRIVATE KEY")) {
        var key = try decode(DsaPrivate, material.bytes);
        errdefer std.crypto.secureZero(u8, std.mem.asBytes(&key));
        try io.checkCancel();
        return key;
    }
    if (!std.mem.eql(u8, material.label, "PRIVATE KEY")) return error.InvalidPrivateKey;
    const info = try decode(PrivateKeyInfo, material.bytes);
    if (!oidEqual(info.algorithm.oid, asn1.Oid.fromDotComptime("1.2.840.10040.4.1"))) return error.InvalidPrivateKey;
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const parameters = try decodeParameter(DsaParameters, temporary.allocator(), info.algorithm.parameters orelse return error.InvalidPrivateKey);
    var key = DsaPrivate{ .version = 0, .p = parameters.p, .q = parameters.q, .g = parameters.g, .y = 0, .x = try decode(u16384, info.key.bytes) };
    errdefer std.crypto.secureZero(u8, std.mem.asBytes(&key));
    const Modulus = std.crypto.ff.Modulus(16384);
    const Uint = std.crypto.ff.Uint(16384);
    const modulus = try Modulus.fromPrimitive(u16384, key.p);
    const generator = modulus.reduce(try Uint.fromPrimitive(u16384, key.g));
    var exponent: [2048]u8 = undefined;
    std.mem.writeInt(u16384, &exponent, key.x, .big);
    defer std.crypto.secureZero(u8, &exponent);
    const public = if (key.x == 0) modulus.one() else try modulus.powWithEncodedExponent(generator, &exponent, .big);
    key.y = try public.toPrimitive(u16384);
    try io.checkCancel();
    return key;
}
pub const RsaPrivate = struct {
    version: i32,
    modulus: u16384,
    publicExponent: u16384,
    privateExponent: u16384,
    prime1: u16384,
    prime2: u16384,
    exponent1: u16384,
    exponent2: u16384,
    coefficient: u16384,
    pub fn decodeDer(decoder: *der.Decoder) !@This() {
        const sequence = try decoder.sequence();
        defer decoder.index = sequence.slice.end;
        var fields = der.Decoder{ .bytes = decoder.view(sequence) };
        var result: @This() = undefined;
        errdefer std.crypto.secureZero(u8, std.mem.asBytes(&result));
        const info = @typeInfo(@This()).@"struct";
        inline for (info.field_names, info.field_types) |name, Field| @field(result, name) = try fields.any(Field);
        var extra_count: usize = 0;
        if (fields.index < fields.bytes.len) {
            const extra = try fields.sequence();
            var primes = der.Decoder{ .bytes = fields.view(extra) };
            while (primes.index < primes.bytes.len) {
                const prime_info = try primes.sequence();
                var parts = der.Decoder{ .bytes = primes.view(prime_info) };
                inline for (0..3) |_| {
                    var component = try parts.any(u16384);
                    std.crypto.secureZero(u8, std.mem.asBytes(&component));
                }
                if (parts.index != parts.bytes.len) return error.InvalidPrivateKey;
                primes.index = prime_info.slice.end;
                extra_count += 1;
            }
            fields.index = extra.slice.end;
        }
        if (fields.index != fields.bytes.len or (result.version == 1 and extra_count == 0)) return error.InvalidPrivateKey;
        return result;
    }
};
pub fn rsaPrivate(a: std.mem.Allocator, io: std.Io, pem: []const u8) !RsaPrivate {
    return rsaPrivateImpl(a, io, pem, false);
}
pub fn rsaPrivatePss(a: std.mem.Allocator, io: std.Io, pem: []const u8) !RsaPrivate {
    return rsaPrivateImpl(a, io, pem, true);
}
fn rsaPrivateImpl(a: std.mem.Allocator, io: std.Io, pem: []const u8, allow_pss: bool) !RsaPrivate {
    const material = try decodePrivatePem(a, io, pem);
    defer a.free(material.label);
    defer {
        std.crypto.secureZero(u8, material.bytes);
        a.free(material.bytes);
    }
    const bytes = if (std.mem.eql(u8, material.label, "RSA PRIVATE KEY")) material.bytes else blk: {
        if (!std.mem.eql(u8, material.label, "PRIVATE KEY")) return error.InvalidPrivateKey;
        const container = try decode(PrivateKeyInfo, material.bytes);
        const pss = oidEqual(container.algorithm.oid, asn1.Oid.fromDotComptime("1.2.840.113549.1.1.10"));
        if (!oidEqual(container.algorithm.oid, asn1.Oid.fromDotComptime("1.2.840.113549.1.1.1")) and
            !(allow_pss and pss)) return error.InvalidPrivateKey;
        if (pss) _ = try pssRules(a, container.algorithm.parameters);
        break :blk container.key.bytes;
    };
    // ASN.1 import does not run RSA_check_key or impose signing arithmetic
    // ranges. The standard modulus/exponent operations validate their own
    // preconditions when a signing operation actually needs them.
    return decode(RsaPrivate, bytes);
}
pub fn signRs256(a: std.mem.Allocator, io: std.Io, pem: []const u8, message: []const u8) ![]u8 {
    try io.checkCancel();
    var key = try rsaPrivate(a, io, pem);
    defer std.crypto.secureZero(u8, std.mem.asBytes(&key));
    var modulus_bytes: [2048]u8 = undefined;
    std.mem.writeInt(u16384, &modulus_bytes, key.modulus, .big);
    const modulus = try std.crypto.ff.Modulus(16384).fromBytes(&modulus_bytes, .big);
    const n = modulus.encodedLen();
    var hash: [32]u8 = undefined;
    std.crypto.hash.sha2.Sha256.hash(message, &hash, .{});
    const digest = try der.encode(a, .{ .algorithm = .{ .oid = asn1.Oid.fromDotComptime("2.16.840.1.101.3.4.2.1"), .parameters = asn1.Any{ .tag = .universal(.null, false), .bytes = "" } }, .digest = Octets{ .bytes = &hash } });
    defer a.free(digest);
    if (n < digest.len + 11) return error.InvalidPrivateKey;
    const encoded = try a.alloc(u8, n);
    defer a.free(encoded);
    encoded[0] = 0;
    encoded[1] = 1;
    const boundary = n - digest.len - 1;
    @memset(encoded[2..boundary], 0xff);
    encoded[boundary] = 0;
    @memcpy(encoded[boundary + 1 ..], digest);
    const value = try std.crypto.ff.Modulus(16384).Fe.fromBytes(modulus, encoded, .big);
    var exponent: [2048]u8 = undefined;
    std.mem.writeInt(u16384, &exponent, key.privateExponent, .big);
    defer std.crypto.secureZero(u8, &exponent);
    const exponent_size = @max(n, @divCeil(@as(usize, 16384 - @clz(key.privateExponent)), 8));
    const signed = try modulus.powWithEncodedExponent(value, exponent[exponent.len - exponent_size ..], .big);
    // Verify with the public exponent before publishing a private-key result.
    var public_exponent: [2048]u8 = undefined;
    std.mem.writeInt(u16384, &public_exponent, key.publicExponent, .big);
    const checked = try modulus.powWithEncodedPublicExponent(signed, std.mem.trimStart(u8, &public_exponent, &.{0}), .big);
    if (!checked.eql(value)) return error.InvalidPrivateKey;
    const result = try a.alloc(u8, n);
    errdefer a.free(result);
    try signed.toBytes(result, .big);
    try io.checkCancel();
    return result;
}
const EcField = struct {
    kind: asn1.Oid,
    prime: u16384,
    pub fn decodeDer(decoder: *der.Decoder) !@This() {
        return completeSequence(@This(), decoder);
    }
};
const EcCurve = struct {
    a: Octets,
    b: Octets,
    seed: ?asn1.BitString,
    pub fn decodeDer(decoder: *der.Decoder) !@This() {
        const sequence = try decoder.sequence();
        defer decoder.index = sequence.slice.end;
        var fields = der.Decoder{ .bytes = decoder.view(sequence) };
        const result = @This(){ .a = try fields.any(Octets), .b = try fields.any(Octets), .seed = if (fields.index < fields.bytes.len) try fields.any(asn1.BitString) else null };
        if (fields.index != fields.bytes.len) return error.InvalidPrivateKey;
        if (result.seed) |seed| if (seed.bytes.len == 0) return error.InvalidPrivateKey;
        return result;
    }
};
const EcSpecified = struct {
    version: i32,
    field: EcField,
    curve: EcCurve,
    base: Octets,
    order: u16384,
    cofactor: ?u16384,
    pub fn decodeDer(decoder: *der.Decoder) !@This() {
        const sequence = try decoder.sequence();
        defer decoder.index = sequence.slice.end;
        return fromContents(decoder.view(sequence));
    }
    fn fromContents(bytes: []const u8) !@This() {
        var fields = der.Decoder{ .bytes = bytes };
        const result = @This(){ .version = try fields.any(i32), .field = try fields.any(EcField), .curve = try fields.any(EcCurve), .base = try fields.any(Octets), .order = try fields.any(u16384), .cofactor = if (fields.index < fields.bytes.len) try fields.any(u16384) else null };
        if (fields.index != fields.bytes.len) return error.InvalidPrivateKey;
        return result;
    }
};
const EcParameters = union(enum) {
    named: asn1.Oid,
    specified: EcSpecified,
    pub fn decodeDer(decoder: *der.Decoder) !@This() {
        const element = try decoder.element(.{});
        defer decoder.index = element.slice.end;
        return ecParameters(.{ .tag = element.tag, .bytes = decoder.view(element) });
    }
};
fn ecParameters(parameter: asn1.Any) !EcParameters {
    if (std.meta.eql(parameter.tag, asn1.Tag.universal(.oid, false))) return .{ .named = .{ .encoded = parameter.bytes } };
    if (!std.meta.eql(parameter.tag, asn1.Tag.universal(.sequence, true))) return error.InvalidPrivateKey;
    // The standard element already identifies the sequence contents. Reuse
    // its typed fields without building another ASN.1 or integer codec.
    return .{ .specified = try EcSpecified.fromContents(parameter.bytes) };
}
fn specifiedCurveMatches(comptime Scheme: type, value: EcSpecified) !bool {
    const Curve = Scheme.Curve;
    if (!oidEqual(value.field.kind, asn1.Oid.fromDotComptime("1.2.840.10045.1.1")) or value.field.prime != Curve.Fe.field_order or value.order != Curve.scalar.field_order) return false;
    const Modulus = std.crypto.ff.Modulus(16384);
    const Uint = std.crypto.ff.Uint(16384);
    const modulus = try Modulus.fromPrimitive(u16384, value.field.prime);
    const coefficient_a = modulus.reduce(try Uint.fromBytes(value.curve.a.bytes, .big));
    const coefficient_b = modulus.reduce(try Uint.fromBytes(value.curve.b.bytes, .big));
    const expected_a: u16384 = if (Curve == std.crypto.ecc.Secp256k1) 0 else Curve.Fe.field_order - 3;
    if (try coefficient_a.toPrimitive(u16384) != expected_a or try coefficient_b.toPrimitive(u16384) != Curve.B.toInt()) return false;
    const base = try ecPublicFor(Scheme, value.base.bytes);
    const expected = Scheme.PublicKey{ .p = Curve.basePoint };
    return std.mem.eql(u8, &base.toUncompressedSec1(), &expected.toUncompressedSec1());
}
fn ecParameterCurve(parameters: EcParameters) !asn1.Oid {
    return switch (parameters) {
        .named => |oid| oid,
        .specified => |value| curve: {
            inline for (.{
                .{ std.crypto.sign.ecdsa.EcdsaP256Sha256, asn1.Oid.fromDotComptime("1.2.840.10045.3.1.7") },
                .{ std.crypto.sign.ecdsa.EcdsaP384Sha384, asn1.Oid.fromDotComptime("1.3.132.0.34") },
                .{ std.crypto.sign.ecdsa.EcdsaSecp256k1Sha256, asn1.Oid.fromDotComptime("1.3.132.0.10") },
            }) |entry| if (try specifiedCurveMatches(entry[0], value)) break :curve entry[1];
            return error.UnsupportedPrivateKey;
        },
    };
}
const EcPrivate = struct {
    version: i32,
    key: Octets,
    parameters: ?EcParameters = null,
    publicKey: ?asn1.BitString = null,
    pub const asn1_tags = .{ .parameters = asn1.FieldTag.initExplicit(0, .context_specific), .publicKey = asn1.FieldTag.initExplicit(1, .context_specific) };
    pub fn decodeDer(decoder: *der.Decoder) !@This() {
        const sequence = try decoder.sequence();
        defer decoder.index = sequence.slice.end;
        var fields = der.Decoder{ .bytes = decoder.view(sequence) };
        const result = @This(){
            .version = try fields.any(i32),
            .key = try fields.any(Octets),
            .parameters = try optionalExplicit(EcParameters, &fields, 0),
            .publicKey = try optionalExplicit(asn1.BitString, &fields, 1),
        };
        if (fields.index != fields.bytes.len) return error.InvalidPrivateKey;
        return result;
    }
};
/// Node routes both DER pkcs1 and sec1 inputs through d2i_AutoPrivateKey.
/// Select only among existing standard-DER schemas; do not infer a key family
/// from the caller's type spelling or add a separate ASN.1 byte parser.
pub fn autoPrivateDerLabel(bytes: []const u8) ![]const u8 {
    if (decode(PrivateKeyInfo, bytes)) |_| return "PRIVATE KEY" else |_| {}
    if (decode(RsaPrivate, bytes)) |decoded| {
        var key = decoded;
        defer std.crypto.secureZero(u8, std.mem.asBytes(&key));
        return "RSA PRIVATE KEY";
    } else |_| {}
    if (decode(EcPrivate, bytes)) |_| return "EC PRIVATE KEY" else |_| {}
    if (decode(DsaPrivate, bytes)) |decoded| {
        var key = decoded;
        defer std.crypto.secureZero(u8, std.mem.asBytes(&key));
        return "DSA PRIVATE KEY";
    } else |_| {}
    return error.InvalidPrivateKey;
}
fn ecContainerCurve(container: PrivateKeyInfo) !asn1.Oid {
    const outer = try ecParameterCurve(try ecParameters(container.algorithm.parameters orelse return error.InvalidPrivateKey));
    inline for (.{
        asn1.Oid.fromDotComptime("1.2.840.10045.3.1.7"),
        asn1.Oid.fromDotComptime("1.3.132.0.34"),
        asn1.Oid.fromDotComptime("1.3.132.0.10"),
    }) |supported| if (oidEqual(outer, supported)) {
        const key = try decode(EcPrivate, container.key.bytes);
        return if (key.parameters) |parameters| try ecParameterCurve(parameters) else outer;
    };
    return outer;
}
/// Import metadata needed when an EC consumer re-exports a public SPKI.
/// Other callers continue to receive the identical standard key pair.
fn EcPrivateImport(comptime Scheme: type) type {
    return struct { pair: Scheme.KeyPair, namedParameters: bool, explicitSeed: bool, pointForm: u8 };
}
pub fn p256PrivateImport(a: std.mem.Allocator, io: std.Io, pem: []const u8) !EcPrivateImport(std.crypto.sign.ecdsa.EcdsaP256Sha256) {
    return ecPrivateImport(std.crypto.sign.ecdsa.EcdsaP256Sha256, asn1.Oid.fromDotComptime("1.2.840.10045.3.1.7"), a, io, pem);
}
pub fn p256Private(a: std.mem.Allocator, io: std.Io, pem: []const u8) !std.crypto.sign.ecdsa.EcdsaP256Sha256.KeyPair {
    return ecPrivate(std.crypto.sign.ecdsa.EcdsaP256Sha256, asn1.Oid.fromDotComptime("1.2.840.10045.3.1.7"), a, io, pem);
}
pub fn p384Private(a: std.mem.Allocator, io: std.Io, pem: []const u8) !std.crypto.sign.ecdsa.EcdsaP384Sha384.KeyPair {
    return ecPrivate(std.crypto.sign.ecdsa.EcdsaP384Sha384, asn1.Oid.fromDotComptime("1.3.132.0.34"), a, io, pem);
}
pub fn ecPrivateFor(comptime Scheme: type, a: std.mem.Allocator, io: std.Io, pem: []const u8) !Scheme.KeyPair {
    const curve = if (Scheme.Curve == std.crypto.ecc.P256) asn1.Oid.fromDotComptime("1.2.840.10045.3.1.7") else if (Scheme.Curve == std.crypto.ecc.P384) asn1.Oid.fromDotComptime("1.3.132.0.34") else if (Scheme.Curve == std.crypto.ecc.Secp256k1) asn1.Oid.fromDotComptime("1.3.132.0.10") else @compileError("Unsupported TLS EC curve");
    return ecPrivate(Scheme, curve, a, io, pem);
}
fn ecPublicFor(comptime Scheme: type, bytes: []const u8) !Scheme.PublicKey {
    if (bytes.len == 0 or (bytes[0] != 6 and bytes[0] != 7)) return Scheme.PublicKey.fromSec1(bytes);
    if (bytes.len != Scheme.PublicKey.uncompressed_sec1_encoded_length) return error.InvalidEncoding;
    var reader = std.Io.Reader.fixed(bytes);
    const form = try reader.takeByte();
    const x = try reader.takeArray(Scheme.Curve.Fe.encoded_length);
    const y = try reader.takeArray(Scheme.Curve.Fe.encoded_length);
    if ((form & 1) != (y[y.len - 1] & 1)) return error.InvalidEncoding;
    return .{ .p = try Scheme.Curve.fromSerializedAffineCoordinates(x.*, y.*, .big) };
}
fn ecPrivate(comptime Scheme: type, curve: asn1.Oid, a: std.mem.Allocator, io: std.Io, pem: []const u8) !Scheme.KeyPair {
    var imported = try ecPrivateImport(Scheme, curve, a, io, pem);
    defer std.crypto.secureZero(u8, std.mem.asBytes(&imported.pair.secret_key));
    return imported.pair;
}
fn ecPrivateImport(comptime Scheme: type, curve: asn1.Oid, a: std.mem.Allocator, io: std.Io, pem: []const u8) !EcPrivateImport(Scheme) {
    const material = try decodePrivatePem(a, io, pem);
    defer a.free(material.label);
    defer {
        std.crypto.secureZero(u8, material.bytes);
        a.free(material.bytes);
    }
    var named_parameters = true;
    var explicit_seed = false;
    const bytes = if (std.mem.eql(u8, material.label, "EC PRIVATE KEY")) material.bytes else blk: {
        if (!std.mem.eql(u8, material.label, "PRIVATE KEY")) return error.InvalidPrivateKey;
        const container = try decode(PrivateKeyInfo, material.bytes);
        if (!oidEqual(container.algorithm.oid, asn1.Oid.fromDotComptime("1.2.840.10045.2.1"))) return error.InvalidPrivateKey;
        if (!oidEqual(try ecContainerCurve(container), curve)) return error.InvalidPrivateKey;
        const parameters = try ecParameters(container.algorithm.parameters orelse return error.InvalidPrivateKey);
        named_parameters = parameters == .named;
        explicit_seed = parameters == .specified and parameters.specified.curve.seed != null;
        break :blk container.key.bytes;
    };
    const size = Scheme.SecretKey.encoded_length;
    const key = try decode(EcPrivate, bytes);
    if (key.parameters) |parameters| {
        if (!oidEqual(try ecParameterCurve(parameters), curve)) return error.InvalidPrivateKey;
        named_parameters = parameters == .named;
        explicit_seed = parameters == .specified and parameters.specified.curve.seed != null;
    }
    const significant = std.mem.trimStart(u8, key.key.bytes, &.{0});
    var scalar: [size]u8 = @splat(0);
    defer std.crypto.secureZero(u8, &scalar);
    if (significant.len <= size) @memcpy(scalar[size - significant.len ..], significant) else {
        // The fixed-width curve API needs an equivalent subgroup scalar.
        // Compose standard integer decoding and modular operations over
        // borrowed chunks without imposing a second private-integer limit.
        const Uint = std.crypto.ff.Uint(1024);
        const order = try std.crypto.ff.Modulus(512).fromPrimitive(u512, Scheme.Curve.scalar.field_order);
        var reduced = order.zero;
        defer std.crypto.secureZero(u8, std.mem.asBytes(&reduced));
        var reader = std.Io.Reader.fixed(significant);
        while (reader.bufferedLen() > 0) {
            try io.checkCancel();
            const count = @min(@as(usize, 64), reader.bufferedLen());
            var integer = try Uint.fromBytes(try reader.take(count), .big);
            defer std.crypto.secureZero(u8, std.mem.asBytes(&integer));
            const shift = try Uint.fromPrimitive(u1024, @as(u1024, 1) << @as(u10, @intCast(count * 8)));
            var product = order.mul(reduced, order.reduce(shift));
            defer std.crypto.secureZero(u8, std.mem.asBytes(&product));
            var part = order.reduce(integer);
            defer std.crypto.secureZero(u8, std.mem.asBytes(&part));
            reduced = order.add(product, part);
        }
        try reduced.toBytes(&scalar, .big);
    }
    var secret = try Scheme.SecretKey.fromBytes(scalar);
    defer std.crypto.secureZero(u8, std.mem.asBytes(&secret));
    if (key.publicKey) |public| return .{ .pair = .{ .secret_key = secret, .public_key = try ecPublicFor(Scheme, public.bytes) }, .namedParameters = named_parameters, .explicitSeed = explicit_seed, .pointForm = if (public.bytes.len > 0) public.bytes[0] & 0xfe else 4 };
    return .{ .pair = Scheme.KeyPair.fromSecretKey(secret) catch .{ .secret_key = secret, .public_key = .{ .p = Scheme.Curve.identityElement } }, .namedParameters = named_parameters, .explicitSeed = explicit_seed, .pointForm = 4 };
}
pub fn ed25519Private(a: std.mem.Allocator, io: std.Io, pem: []const u8) !std.crypto.sign.Ed25519.KeyPair {
    const material = try decodePrivatePem(a, io, pem);
    defer a.free(material.label);
    defer {
        std.crypto.secureZero(u8, material.bytes);
        a.free(material.bytes);
    }
    if (!std.mem.eql(u8, material.label, "PRIVATE KEY")) return error.InvalidPrivateKey;
    const container = try decode(PrivateKeyInfo, material.bytes);
    if (!oidEqual(container.algorithm.oid, asn1.Oid.fromDotComptime("1.3.101.112")) or container.algorithm.parameters != null) return error.InvalidPrivateKey;
    const seed = try decode(Octets, container.key.bytes);
    if (seed.bytes.len != std.crypto.sign.Ed25519.KeyPair.seed_length) return error.InvalidPrivateKey;
    return std.crypto.sign.Ed25519.KeyPair.generateDeterministic(seed.bytes[0..std.crypto.sign.Ed25519.KeyPair.seed_length].*);
}
/// Structural import for Google's streaming-signer rejection. No Ed448
/// signature/public-key implementation is introduced by this boundary.
pub fn isPrivateEd448(a: std.mem.Allocator, io: std.Io, pem: []const u8) !bool {
    try io.checkCancel();
    const material = try decodePrivatePem(a, io, pem);
    defer a.free(material.label);
    defer {
        std.crypto.secureZero(u8, material.bytes);
        a.free(material.bytes);
    }
    if (!std.mem.eql(u8, material.label, "PRIVATE KEY")) return false;
    const container = try decode(PrivateKeyInfo, material.bytes);
    if (!oidEqual(container.algorithm.oid, asn1.Oid.fromDotComptime("1.3.101.113"))) return false;
    if (container.algorithm.parameters != null) return error.InvalidPrivateKey;
    const seed = try decode(Octets, container.key.bytes);
    if (seed.bytes.len != 57) return error.InvalidPrivateKey;
    try io.checkCancel();
    return true;
}
pub const PrivateKeyType = enum { rsa, rsa_pss, p256, p384, ed25519 };
/// A Google signer may accept a named EC curve outside the TLS key set.
/// Compare within this owner's lifetime; returned OIDs never borrow freed DER.
pub fn isPrivateEcCurve(a: std.mem.Allocator, io: std.Io, pem: []const u8, curve: asn1.Oid) !bool {
    const material = try decodePrivatePem(a, io, pem);
    defer a.free(material.label);
    defer {
        std.crypto.secureZero(u8, material.bytes);
        a.free(material.bytes);
    }
    if (std.mem.eql(u8, material.label, "EC PRIVATE KEY")) {
        const parameters = (try decode(EcPrivate, material.bytes)).parameters orelse return false;
        return oidEqual(try ecParameterCurve(parameters), curve);
    }
    if (!std.mem.eql(u8, material.label, "PRIVATE KEY")) return false;
    const info = try decode(PrivateKeyInfo, material.bytes);
    if (!oidEqual(info.algorithm.oid, asn1.Oid.fromDotComptime("1.2.840.10045.2.1"))) return false;
    if (info.algorithm.parameters == null) return false;
    return oidEqual(try ecContainerCurve(info), curve);
}
pub fn privateKeyType(a: std.mem.Allocator, io: std.Io, pem: []const u8) !PrivateKeyType {
    const material = try decodePrivatePem(a, io, pem);
    defer a.free(material.label);
    defer {
        std.crypto.secureZero(u8, material.bytes);
        a.free(material.bytes);
    }
    if (std.mem.eql(u8, material.label, "RSA PRIVATE KEY")) return .rsa;
    if (std.mem.eql(u8, material.label, "DSA PRIVATE KEY")) return error.UnsupportedPrivateKey;
    const curve = if (std.mem.eql(u8, material.label, "EC PRIVATE KEY")) try ecParameterCurve((try decode(EcPrivate, material.bytes)).parameters orelse return error.InvalidPrivateKey) else curve: {
        if (!std.mem.eql(u8, material.label, "PRIVATE KEY")) return error.InvalidPrivateKey;
        const info = try decode(PrivateKeyInfo, material.bytes);
        if (oidEqual(info.algorithm.oid, asn1.Oid.fromDotComptime("1.2.840.113549.1.1.1"))) return .rsa;
        if (oidEqual(info.algorithm.oid, asn1.Oid.fromDotComptime("1.2.840.113549.1.1.10"))) return .rsa_pss;
        if (oidEqual(info.algorithm.oid, asn1.Oid.fromDotComptime("1.3.101.112"))) return .ed25519;
        if (!oidEqual(info.algorithm.oid, asn1.Oid.fromDotComptime("1.2.840.10045.2.1"))) return error.UnsupportedPrivateKey;
        break :curve try ecContainerCurve(info);
    };
    if (oidEqual(curve, asn1.Oid.fromDotComptime("1.2.840.10045.3.1.7"))) return .p256;
    if (oidEqual(curve, asn1.Oid.fromDotComptime("1.3.132.0.34"))) return .p384;
    return error.UnsupportedPrivateKey;
}
pub const RsaHash = enum { sha1, sha224, sha256, sha384, sha512, sha512_224, sha512_256, unsupported };
pub const RsaPssRules = struct {
    hash: ?RsaHash = null,
    mask_hash: ?RsaHash = null,
    min_salt: i32 = 0,
    trailer: i32 = 1,
    pub fn allows(self: @This(), hash: RsaHash, length: usize) bool {
        return (self.hash == null or self.hash.? == hash) and (self.mask_hash == null or self.mask_hash.? == hash) and self.min_salt >= 0 and @as(usize, @intCast(self.min_salt)) <= length and self.trailer == 1;
    }
};
const PssParameters = struct {
    hash: ?Algorithm = null,
    mask: ?Algorithm = null,
    salt: ?i16384 = null,
    trailer: ?i16384 = null,
    pub const asn1_tags = .{
        .hash = asn1.FieldTag.initExplicit(0, .context_specific),
        .mask = asn1.FieldTag.initExplicit(1, .context_specific),
        .salt = asn1.FieldTag.initExplicit(2, .context_specific),
        .trailer = asn1.FieldTag.initExplicit(3, .context_specific),
    };
    pub fn decodeDer(decoder: *der.Decoder) !@This() {
        const sequence = try decoder.sequence();
        defer decoder.index = sequence.slice.end;
        var fields = der.Decoder{ .bytes = decoder.view(sequence) };
        const result = @This(){
            .hash = try optionalExplicit(Algorithm, &fields, 0),
            .mask = try optionalExplicit(Algorithm, &fields, 1),
            .salt = try optionalExplicit(i16384, &fields, 2),
            .trailer = try optionalExplicit(i16384, &fields, 3),
        };
        if (fields.index != fields.bytes.len) return error.InvalidPrivateKey;
        return result;
    }
};
fn digestKind(algorithm: Algorithm) !RsaHash {
    const algorithms = .{
        .{ RsaHash.sha1, asn1.Oid.fromDotComptime("1.3.14.3.2.26") },
        .{ RsaHash.sha224, asn1.Oid.fromDotComptime("2.16.840.1.101.3.4.2.4") },
        .{ RsaHash.sha256, asn1.Oid.fromDotComptime("2.16.840.1.101.3.4.2.1") },
        .{ RsaHash.sha384, asn1.Oid.fromDotComptime("2.16.840.1.101.3.4.2.2") },
        .{ RsaHash.sha512, asn1.Oid.fromDotComptime("2.16.840.1.101.3.4.2.3") },
        .{ RsaHash.sha512_224, asn1.Oid.fromDotComptime("2.16.840.1.101.3.4.2.5") },
        .{ RsaHash.sha512_256, asn1.Oid.fromDotComptime("2.16.840.1.101.3.4.2.6") },
    };
    inline for (algorithms) |entry| if (oidEqual(algorithm.oid, entry[1])) return entry[0];
    return .unsupported;
}
fn decodeParameter(comptime T: type, a: std.mem.Allocator, parameter: asn1.Any) !T {
    if (!std.meta.eql(parameter.tag, asn1.Tag.universal(.sequence, true))) return error.InvalidPrivateKey;
    // Decoded Algorithm/Oid fields borrow this encoding in the caller's
    // temporary region until all restrictions have become scalar enum values.
    const encoded = try der.encode(a, parameter);
    return decode(T, encoded);
}
const EncryptedPrivateKeyInfo = struct {
    algorithm: Algorithm,
    data: Octets,
    pub fn decodeDer(decoder: *der.Decoder) !@This() {
        return completeSequence(@This(), decoder);
    }
};
const Pbes2Parameters = struct {
    derivation: Algorithm,
    encryption: Algorithm,
    pub fn decodeDer(decoder: *der.Decoder) !@This() {
        return completeSequence(@This(), decoder);
    }
};
const Pbkdf2Parameters = struct {
    salt: Octets,
    iterations: i16384,
    key_length: ?i16384 = null,
    prf: ?Algorithm = null,
    pub fn decodeDer(decoder: *der.Decoder) !@This() {
        const sequence = try decoder.sequence();
        defer decoder.index = sequence.slice.end;
        var fields = der.Decoder{ .bytes = decoder.view(sequence) };
        var result = @This(){ .salt = try fields.any(Octets), .iterations = try fields.any(i16384) };
        if (fields.index < fields.bytes.len) {
            const next = try asn1.Element.decode(fields.bytes, fields.index);
            if (std.meta.eql(next.tag, asn1.Tag.universal(.integer, false))) result.key_length = try fields.any(i16384);
        }
        if (fields.index < fields.bytes.len) result.prf = try fields.any(Algorithm);
        if (fields.index != fields.bytes.len) return error.InvalidPrivateKey;
        return result;
    }
};
const ScryptParameters = struct {
    salt: Octets,
    cost: i16384,
    block_size: i16384,
    parallelization: i16384,
    key_length: ?i16384,
    pub fn decodeDer(decoder: *der.Decoder) !@This() {
        const sequence = try decoder.sequence();
        defer decoder.index = sequence.slice.end;
        var fields = der.Decoder{ .bytes = decoder.view(sequence) };
        const result = @This(){ .salt = try fields.any(Octets), .cost = try fields.any(i16384), .block_size = try fields.any(i16384), .parallelization = try fields.any(i16384), .key_length = if (fields.index < fields.bytes.len) try fields.any(i16384) else null };
        if (fields.index != fields.bytes.len) return error.InvalidPrivateKey;
        return result;
    }
};
const Pbes2Derivation = union(enum) { pbkdf2: Pbkdf2Parameters, scrypt: ScryptParameters };
/// Both slices share one allocation. Release the full storage, including
/// padding, after the plaintext has been consumed by the key-import owner.
pub const DecryptedPrivateKey = struct {
    storage: []u8,
    plaintext: []const u8,
    pub fn deinit(self: *@This(), a: std.mem.Allocator) void {
        std.crypto.secureZero(u8, self.storage);
        a.free(self.storage);
        self.* = undefined;
    }
};
fn pbkdf2Derive(key: []u8, password: []const u8, parameters: Pbkdf2Parameters) !void {
    if (parameters.key_length) |length| if ((std.math.cast(c_long, length) orelse -1) != @as(c_long, @intCast(key.len))) return error.UnsupportedPrivateKeyEncryption;
    const rounds = legacyAsn1Integer(parameters.iterations);
    if (rounds < 1) return error.InvalidPrivateKey;
    const prf = if (parameters.prf) |algorithm| algorithm.oid else asn1.Oid.fromDotComptime("1.2.840.113549.2.7");
    const hmac = std.crypto.auth.hmac;
    inline for (.{
        .{ "1.3.6.1.5.5.8.1.1", hmac.HmacMd5 },                                 .{ "1.3.6.1.5.5.8.1.2", hmac.HmacSha1 },
        .{ "1.2.840.113549.2.6", hmac.HmacMd5 },                                .{ "1.2.840.113549.2.7", hmac.HmacSha1 },
        .{ "1.2.840.113549.2.8", hmac.sha2.HmacSha224 },                        .{ "1.2.840.113549.2.9", hmac.sha2.HmacSha256 },
        .{ "1.2.840.113549.2.10", hmac.sha2.HmacSha384 },                       .{ "1.2.840.113549.2.11", hmac.sha2.HmacSha512 },
        .{ "1.2.840.113549.2.12", hmac.Hmac(std.crypto.hash.sha2.Sha512_224) }, .{ "1.2.840.113549.2.13", hmac.Hmac(std.crypto.hash.sha2.Sha512_256) },
    }) |entry| if (oidEqual(prf, asn1.Oid.fromDotComptime(entry[0]))) {
        try std.crypto.pwhash.pbkdf2(key, password, parameters.salt.bytes, @intCast(rounds), entry[1]);
        return;
    };
    return error.UnsupportedPrivateKeyEncryption;
}
fn scryptDerive(a: std.mem.Allocator, key: []u8, password: []const u8, parameters: ScryptParameters) !void {
    if (parameters.key_length) |length| if ((std.math.cast(u64, length) orelse return error.InvalidPrivateKey) != key.len) return error.UnsupportedPrivateKeyEncryption;
    const n = std.math.cast(u64, parameters.cost) orelse return error.InvalidPrivateKey;
    const r = std.math.cast(u32, parameters.block_size) orelse return error.InvalidPrivateKey;
    const p = std.math.cast(u32, parameters.parallelization) orelse return error.InvalidPrivateKey;
    if (n < 2 or n & (n - 1) != 0 or r == 0 or p == 0) return error.InvalidPrivateKey;
    if (@as(u64, r) * p >= 1 << 30 or (r <= 3 and @ctz(n) >= @as(u64, r) * 16)) return error.InvalidPrivateKey;
    const memory = @as(u128, 128) * r * (@as(u128, n) + p + 2);
    if (memory > 32 * 1024 * 1024) return error.PrivateKeyKdfMemoryLimit;
    try std.crypto.pwhash.scrypt.kdf(a, key, password, parameters.salt.bytes, .{ .ln = @intCast(@ctz(n)), .r = @intCast(r), .p = @intCast(p) });
}
fn pbes2AesCbc(comptime Aes: type, a: std.mem.Allocator, io: std.Io, password: []const u8, parameters: Pbes2Derivation, iv: *const [16]u8, ciphertext: []const u8) !DecryptedPrivateKey {
    var key: [Aes.key_bits / 8]u8 = undefined;
    defer std.crypto.secureZero(u8, &key);
    try io.checkCancel();
    switch (parameters) {
        .pbkdf2 => |value| try pbkdf2Derive(&key, password, value),
        .scrypt => |value| try scryptDerive(a, &key, password, value),
    }
    try io.checkCancel();
    return aesCbcDecrypt(Aes, a, io, &key, iv, ciphertext);
}
fn aesCbcDecrypt(comptime Aes: type, a: std.mem.Allocator, io: std.Io, key: *const [Aes.key_bits / 8]u8, iv: *const [16]u8, ciphertext: []const u8) !DecryptedPrivateKey {
    if (ciphertext.len == 0 or ciphertext.len % 16 != 0) return error.InvalidPrivateKey;
    var context = Aes.initDec(key.*);
    defer std.crypto.secureZero(u8, std.mem.asBytes(&context));
    const storage = try a.alloc(u8, ciphertext.len);
    errdefer {
        std.crypto.secureZero(u8, storage);
        a.free(storage);
    }
    var previous = iv.*;
    var reader = std.Io.Reader.fixed(ciphertext);
    var offset: usize = 0;
    while (reader.bufferedLen() > 0) : (offset += 16) {
        try io.checkCancel();
        const block = try reader.takeArray(16);
        const output = storage[offset..][0..16];
        context.decrypt(output, block);
        for (output, previous) |*byte, prior| byte.* ^= prior;
        previous = block.*;
    }
    // Check every final-block byte before interpreting the padding length.
    const padding = storage[storage.len - 1];
    const tail: @Vector(16, u8) = storage[storage.len - 16 ..][0..16].*;
    const backwards: @Vector(16, u8) = .{ 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0 };
    const included: @Vector(16, bool) = backwards < @as(@Vector(16, u8), @splat(padding));
    const difference = @select(u8, included, tail ^ @as(@Vector(16, u8), @splat(padding)), @as(@Vector(16, u8), @splat(0)));
    if (padding == 0 or padding > 16 or @reduce(.Or, difference) != 0) return error.InvalidPrivateKey;
    try io.checkCancel();
    return .{ .storage = storage, .plaintext = storage[0 .. storage.len - padding] };
}
fn privateKeyPassphrase(password: ?[]const u8) ![]const u8 {
    const value = password orelse return error.PrivateKeyPassphraseRequired;
    if (value.len > 1024) return error.PrivateKeyPassphraseTooLong;
    return value;
}
pub fn decryptLegacyPem(a: std.mem.Allocator, io: std.Io, info: LegacyPemEncryption, ciphertext: []const u8, password: ?[]const u8) !DecryptedPrivateKey {
    const passphrase = try privateKeyPassphrase(password);
    inline for (.{ .{ LegacyPemCipher.aes128_cbc, std.crypto.core.aes.Aes128 }, .{ LegacyPemCipher.aes256_cbc, std.crypto.core.aes.Aes256 } }) |entry| if (info.cipher == entry[0]) {
        var key: [entry[1].key_bits / 8]u8 = undefined;
        defer std.crypto.secureZero(u8, &key);
        var digest: [std.crypto.hash.Md5.digest_length]u8 = undefined;
        defer std.crypto.secureZero(u8, &digest);
        var offset: usize = 0;
        while (offset < key.len) {
            try io.checkCancel();
            var hash = std.crypto.hash.Md5.init(.{});
            defer std.crypto.secureZero(u8, std.mem.asBytes(&hash));
            if (offset > 0) hash.update(&digest);
            hash.update(passphrase);
            hash.update(info.iv[0..8]);
            hash.final(&digest);
            const count = @min(digest.len, key.len - offset);
            @memcpy(key[offset..][0..count], digest[0..count]);
            offset += count;
        }
        return aesCbcDecrypt(entry[1], a, io, &key, &info.iv, ciphertext);
    };
    unreachable;
}
/// A null result denotes a non-encrypted PKCS8 schema. Recognized encrypted
/// containers retain their own unsupported/malformed/passphrase failures.
pub fn decryptPkcs8(a: std.mem.Allocator, io: std.Io, encoded: []const u8, password: ?[]const u8) !?DecryptedPrivateKey {
    try io.checkCancel();
    const info = decode(EncryptedPrivateKeyInfo, encoded) catch return null;
    if (!oidEqual(info.algorithm.oid, asn1.Oid.fromDotComptime("1.2.840.113549.1.5.13"))) return error.UnsupportedPrivateKeyEncryption;
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    const parameters = try decodeParameter(Pbes2Parameters, scratch, info.algorithm.parameters orelse return error.InvalidPrivateKey);
    const derivation = parameters.derivation.parameters orelse return error.InvalidPrivateKey;
    const kdf: Pbes2Derivation = if (oidEqual(parameters.derivation.oid, asn1.Oid.fromDotComptime("1.2.840.113549.1.5.12"))) .{ .pbkdf2 = try decodeParameter(Pbkdf2Parameters, scratch, derivation) } else if (oidEqual(parameters.derivation.oid, asn1.Oid.fromDotComptime("1.3.6.1.4.1.11591.4.11"))) .{ .scrypt = try decodeParameter(ScryptParameters, scratch, derivation) } else return error.UnsupportedPrivateKeyEncryption;
    const iv = parameters.encryption.parameters orelse return error.InvalidPrivateKey;
    if (!std.meta.eql(iv.tag, asn1.Tag.universal(.octetstring, false)) or iv.bytes.len != 16) return error.InvalidPrivateKey;
    const passphrase = try privateKeyPassphrase(password);
    const aes = std.crypto.core.aes;
    if (oidEqual(parameters.encryption.oid, asn1.Oid.fromDotComptime("2.16.840.1.101.3.4.1.2"))) return try pbes2AesCbc(aes.Aes128, a, io, passphrase, kdf, iv.bytes[0..16], info.data.bytes);
    if (oidEqual(parameters.encryption.oid, asn1.Oid.fromDotComptime("2.16.840.1.101.3.4.1.42"))) return try pbes2AesCbc(aes.Aes256, a, io, passphrase, kdf, iv.bytes[0..16], info.data.bytes);
    return error.UnsupportedPrivateKeyEncryption;
}
fn pssRules(allocator: std.mem.Allocator, parameters: ?asn1.Any) !RsaPssRules {
    var temporary = std.heap.ArenaAllocator.init(allocator);
    defer temporary.deinit();
    const a = temporary.allocator();
    const parameter = parameters orelse return .{};
    const spec = try decodeParameter(PssParameters, a, parameter);
    var mask_hash: RsaHash = .sha1;
    if (spec.mask) |mask| {
        // The source decodes MGF1's nested AlgorithmIdentifier during key
        // import; a different mask algorithm fails before signing policy.
        if (!oidEqual(mask.oid, asn1.Oid.fromDotComptime("1.2.840.113549.1.1.8"))) return error.InvalidPrivateKey;
        mask_hash = try digestKind(try decodeParameter(Algorithm, a, mask.parameters orelse return error.InvalidPrivateKey));
    }
    return .{ .hash = if (spec.hash) |hash| try digestKind(hash) else .sha1, .mask_hash = mask_hash, .min_salt = if (spec.salt) |value| legacyAsn1Integer(value) else 20, .trailer = if (spec.trailer) |value| legacyAsn1Integer(value) else 1 };
}
fn legacyAsn1Integer(value: i16384) i32 {
    // ASN1_INTEGER_get returns a target C long, or -1 on overflow. The
    // legacy PSS/PBKDF2 accessors then store that result in a C int.
    return @truncate(std.math.cast(c_long, value) orelse -1);
}
pub fn rsaPssPrivateRules(a: std.mem.Allocator, io: std.Io, pem: []const u8) !RsaPssRules {
    const material = try decodePrivatePem(a, io, pem);
    defer a.free(material.label);
    defer {
        std.crypto.secureZero(u8, material.bytes);
        a.free(material.bytes);
    }
    if (std.mem.eql(u8, material.label, "RSA PRIVATE KEY")) return .{};
    if (!std.mem.eql(u8, material.label, "PRIVATE KEY")) return error.InvalidPrivateKey;
    const info = try decode(PrivateKeyInfo, material.bytes);
    if (oidEqual(info.algorithm.oid, asn1.Oid.fromDotComptime("1.2.840.113549.1.1.1"))) return .{};
    if (!oidEqual(info.algorithm.oid, asn1.Oid.fromDotComptime("1.2.840.113549.1.1.10"))) return error.InvalidPrivateKey;
    return pssRules(a, info.algorithm.parameters);
}
pub fn rsaPssCertificateRules(a: std.mem.Allocator, certificate: std.crypto.Certificate.Parsed) !RsaPssRules {
    // The standard X.509 decoder locates SubjectPublicKeyInfo; the standard
    // typed DER decoder owns its algorithm and explicit PSS parameter fields.
    var decoder = der.Decoder{ .bytes = certificate.certificate.buffer, .index = certificate.subject_slice.end };
    const info = try decoder.any(struct { algorithm: Algorithm, key: asn1.BitString });
    if (oidEqual(info.algorithm.oid, asn1.Oid.fromDotComptime("1.2.840.113549.1.1.1"))) return .{};
    if (!oidEqual(info.algorithm.oid, asn1.Oid.fromDotComptime("1.2.840.113549.1.1.10"))) return error.InvalidPrivateKey;
    return pssRules(a, info.algorithm.parameters);
}
pub fn matchesCertificate(a: std.mem.Allocator, io: std.Io, pem: []const u8, kind: PrivateKeyType, certificate: std.crypto.Certificate.Parsed) !bool {
    switch (kind) {
        .rsa, .rsa_pss => {
            if (certificate.pub_key_algo != .rsaEncryption and certificate.pub_key_algo != .rsassa_pss) return false;
            const public = try decode(struct { modulus: u16384, publicExponent: u16384 }, certificate.pubKey());
            var private = try rsaPrivatePss(a, io, pem);
            defer std.crypto.secureZero(u8, std.mem.asBytes(&private));
            return public.modulus == private.modulus and public.publicExponent == private.publicExponent;
        },
        .p256 => {
            if (certificate.pub_key_algo != .X9_62_id_ecPublicKey or certificate.pub_key_algo.X9_62_id_ecPublicKey != .X9_62_prime256v1) return false;
            var private = try p256Private(a, io, pem);
            defer std.crypto.secureZero(u8, std.mem.asBytes(&private.secret_key));
            const public = try ecPublicFor(std.crypto.sign.ecdsa.EcdsaP256Sha256, certificate.pubKey());
            return std.mem.eql(u8, &public.toUncompressedSec1(), &private.public_key.toUncompressedSec1());
        },
        .p384 => {
            if (certificate.pub_key_algo != .X9_62_id_ecPublicKey or certificate.pub_key_algo.X9_62_id_ecPublicKey != .secp384r1) return false;
            var private = try p384Private(a, io, pem);
            defer std.crypto.secureZero(u8, std.mem.asBytes(&private.secret_key));
            const public = try ecPublicFor(std.crypto.sign.ecdsa.EcdsaP384Sha384, certificate.pubKey());
            return std.mem.eql(u8, &public.toUncompressedSec1(), &private.public_key.toUncompressedSec1());
        },
        .ed25519 => {
            if (certificate.pub_key_algo != .curveEd25519) return false;
            var private = try ed25519Private(a, io, pem);
            defer std.crypto.secureZero(u8, std.mem.asBytes(&private.secret_key));
            return std.mem.eql(u8, certificate.pubKey(), &private.public_key.toBytes());
        },
    }
}
