const std = @import("std");
const t = @import("../../types.zig");
const m = @import("../../models.zig");
const j = @import("../../json/value.zig");
const http = @import("http.zig");
const uri = @import("../../transport/uri.zig");
const headers = @import("../../utils/headers.zig");
const network = @import("../../transport/http.zig");
const certificates = @import("certificate.zig");
const aws = @import("../aws/credentials.zig");
const ambient = @import("../aws/shared_config.zig").sdkEnv;
const Value = std.json.Value;
pub const Token = struct {
    /// null is an absent access-token property; Value.null is a present null.
    /// Conversion belongs to header construction, after refresh/date work.
    access: ?Value,
    /// Metadata's json-bigint decoder creates records without Object.prototype.
    accessFromMetadata: bool = false,
    apiKey: ?Value = null,
    accessTruthy: bool = true,
    scheme: []const u8 = "Bearer",
    tokenType: ?Value = null,
    authClientHeaders: bool = true,
    /// Optional epoch milliseconds; source numeric arithmetic may be non-finite.
    expires: ?f64 = null,
    quotaProject: ?[]const u8 = null,
    quotaValue: ?Value = null,
    /// OAuth retains an existing response expiry_date when expires_in is falsy.
    /// Its raw truthiness and later comparison conversion are both significant.
    expiryDate: ?Value = null,
};
fn diagnosticPolicy(operation: m.OperationContext, policy: http.Policy) http.Policy {
    var result = policy;
    result.errorMessage = operation.errorMessage;
    result.errorStorage = operation.errorStorage;
    result.errorAllocator = operation.allocator;
    return result;
}
fn reject(operation: m.OperationContext, response: http.Response, err: anyerror) anyerror {
    if (operation.errorMessage) |slot| slot.* = http.errorMessage(operation.allocator, response) catch |failure| return failure;
    return err;
}
fn rejectOAuth(operation: m.OperationContext, response: http.Response, err: anyerror, configured: *CredentialConfiguration) anyerror {
    if (configured.failure) |*failure| {
        failure.gaxios = false;
        failure.setName(if (response.body == .null) "TypeError" else "Error") catch |name_error| return name_error;
    }
    if (response.body == .null) {
        if (operation.errorMessage) |slot| slot.* = "Cannot read properties of null (reading 'error')";
        return err;
    }
    const a = operation.allocator;
    const diagnostics = @import("../../utils/diagnostics.zig");
    var message = std.Io.Writer.Allocating.init(a);
    defer message.deinit();
    const code = diagnostics.formatThrownValue(a, if (j.get(response.body, "error")) |value| .{ .value = value } else .undefined_value) catch |failure| return replacementFailure(operation, failure, configured);
    message.writer.print("Error code {s}", .{code}) catch return error.OutOfMemory;
    if (j.get(response.body, "error_description")) |value| {
        const description = diagnostics.formatThrownValue(a, .{ .value = value }) catch |failure| return replacementFailure(operation, failure, configured);
        message.writer.print(": {s}", .{description}) catch return error.OutOfMemory;
    }
    if (j.get(response.body, "error_uri")) |value| {
        const detail = diagnostics.formatThrownValue(a, .{ .value = value }) catch |failure| return replacementFailure(operation, failure, configured);
        message.writer.print(" - {s}", .{detail}) catch return error.OutOfMemory;
    }
    if (operation.errorMessage) |slot| slot.* = message.toOwnedSlice() catch return error.OutOfMemory;
    return err;
}
fn credentialValue(operation: m.OperationContext, credentials: Value, name: []const u8) !Value {
    const value = j.get(credentials, name) orelse .null;
    if (!j.truthy(value)) {
        if (operation.errorMessage) |slot| slot.* = try std.fmt.allocPrint(operation.allocator, "The incoming JSON object does not contain a {s} field", .{name});
        return error.InvalidGoogleCredentials;
    }
    return value;
}
fn keyOption(value: ?Value, allowed: []const []const u8) bool {
    const raw = value orelse return true;
    const text = j.text(raw) orelse return false;
    for (allowed) |candidate| if (std.mem.eql(u8, text, candidate)) return true;
    return false;
}
const SigningKey = struct {
    pem: []const u8,
    owned: []u8,
    options: @import("../signing.zig").GoogleSignatureOptions = .{},
    fn deinit(self: *@This(), a: std.mem.Allocator) void {
        std.crypto.secureZero(u8, self.owned);
        a.free(self.owned);
        self.* = undefined;
    }
};
fn signatureInteger(value: ?Value) !?i32 {
    const raw = value orelse return null;
    const number = j.number(raw) orelse return error.UnsupportedGoogleSigningKeyOptions;
    if (!std.math.isFinite(number) or @trunc(number) != number or number < std.math.minInt(i32) or number > std.math.maxInt(i32)) return error.UnsupportedGoogleSigningKeyOptions;
    return @intFromFloat(number);
}
fn utf8KeyEncoding(value: ?Value) bool {
    const raw = value orelse return true;
    // Buffer.from(string, encoding) defaults nonstrings/empty strings to
    // UTF8; the crypto adapter also recognizes the exact alias "buffer".
    if (raw != .string) return true;
    return raw.string.len == 0 or std.mem.eql(u8, raw.string, "buffer") or std.ascii.eqlIgnoreCase(raw.string, "utf8") or std.ascii.eqlIgnoreCase(raw.string, "utf-8");
}
fn signingKeyText(operation: m.OperationContext, text: []const u8, encoding: ?Value) !SigningKey {
    const a = operation.allocator;
    if (utf8KeyEncoding(encoding)) {
        const units = try std.unicode.wtf8ToWtf16LeAlloc(a, text);
        defer {
            std.crypto.secureZero(u16, units);
            a.free(units);
        }
        const bytes = try std.fmt.allocPrint(a, "{f}", .{std.unicode.fmtUtf16Le(units)});
        return .{ .pem = bytes, .owned = bytes };
    }
    const encoding_name = j.text(encoding.?) orelse "";
    inline for (.{ "utf16le", "utf-16le", "ucs2", "ucs-2" }) |alias| if (std.ascii.eqlIgnoreCase(encoding_name, alias)) {
        // The standard conversion stores little-endian code units on every
        // target. Borrow their byte representation only while duplicating
        // into the separately owned key buffer; preserve surrogate halves.
        const units = try std.unicode.wtf8ToWtf16LeAlloc(a, text);
        defer {
            std.crypto.secureZero(u16, units);
            a.free(units);
        }
        const bytes = try a.dupe(u8, std.mem.sliceAsBytes(units));
        return .{ .pem = bytes, .owned = bytes };
    };
    if (std.ascii.eqlIgnoreCase(encoding_name, "hex")) {
        var regex = try @import("../../regex/regexp.zig").Regex.init(a, operation.io, "^(?:[0-9a-fA-F]{2})*", .{});
        defer regex.deinit();
        var matched = (try regex.execSensitive(a, text, 0)) orelse return error.UnsupportedGoogleSigningKeyOptions;
        defer matched.deinit();
        const prefix = (try matched.text(a, 0)).?;
        defer {
            std.crypto.secureZero(u8, @constCast(prefix));
            a.free(prefix);
        }
        const bytes = try a.alloc(u8, prefix.len / 2);
        errdefer {
            std.crypto.secureZero(u8, bytes);
            a.free(bytes);
        }
        _ = try std.fmt.hexToBytes(bytes, prefix);
        return .{ .pem = bytes, .owned = bytes };
    }
    if (std.ascii.eqlIgnoreCase(encoding_name, "base64") or std.ascii.eqlIgnoreCase(encoding_name, "base64url")) {
        const bytes = try @import("../../utils/base64.zig").decodeNodeBuffer(a, operation.io, text);
        return .{ .pem = bytes, .owned = bytes };
    }
    return error.UnsupportedGoogleSigningKeyOptions;
}
fn jwkBytes(operation: m.OperationContext, value: Value, name: []const u8) ![]u8 {
    const text = j.field(value, name) orelse return error.UnsupportedGoogleSigningKeyOptions;
    return @import("../../utils/base64.zig").decodeNodeBuffer(operation.allocator, operation.io, text);
}
fn jwkStrings(value: Value, comptime names: anytype) !void {
    // Node validates every required property type before native JWK decoding.
    inline for (names) |name| _ = j.field(value, name) orelse return error.UnsupportedGoogleSigningKeyOptions;
}
fn jwkInteger(operation: m.OperationContext, value: Value, name: []const u8) !u16384 {
    const a = operation.allocator;
    const bytes = try jwkBytes(operation, value, name);
    defer {
        std.crypto.secureZero(u8, bytes);
        a.free(bytes);
    }
    var integer = try std.crypto.ff.Uint(16384).fromBytes(bytes, .big);
    defer std.crypto.secureZero(u8, std.mem.asBytes(&integer));
    return integer.toPrimitive(u16384);
}
fn invalidJwk(operation: m.OperationContext) anyerror {
    return executableFailure(operation.errorMessage, error.InvalidGoogleJwk, "Invalid JWK data");
}
fn invalidEcJwk(operation: m.OperationContext) anyerror {
    return executableFailure(operation.errorMessage, error.InvalidGoogleJwk, "Invalid JWK EC key");
}
fn jwkEcCoordinate(operation: m.OperationContext, value: Value, name: []const u8, bytes: []u8) !void {
    const integer = jwkInteger(operation, value, name) catch |err| {
        if (err == error.Overflow) return invalidEcJwk(operation);
        return err;
    };
    const scalar = try std.crypto.ff.Uint(16384).fromPrimitive(u16384, integer);
    scalar.toBytes(bytes, .big) catch |err| {
        if (err == error.Overflow) return invalidEcJwk(operation);
        return err;
    };
}
fn privatePem(a: std.mem.Allocator, label: []const u8, der: []const u8) !SigningKey {
    const codec = std.base64.standard.Encoder;
    const encoded = try a.alloc(u8, codec.calcSize(der.len));
    defer {
        std.crypto.secureZero(u8, encoded);
        a.free(encoded);
    }
    _ = codec.encode(encoded, der);
    const pem = try std.fmt.allocPrint(a, "-----BEGIN {s}-----\n{s}\n-----END {s}-----\n", .{ label, encoded, label });
    return .{ .pem = pem, .owned = pem };
}
fn signingRsaJwk(operation: m.OperationContext, value: Value) !SigningKey {
    try jwkStrings(value, .{ "n", "e", "d", "p", "q", "dp", "dq", "qi" });
    const a = operation.allocator;
    var key: @import("../../crypto/keys.zig").RsaPrivate = undefined;
    defer std.crypto.secureZero(u8, std.mem.asBytes(&key));
    key.version = 0;
    inline for (.{
        .{ "modulus", "n" },    .{ "publicExponent", "e" }, .{ "privateExponent", "d" },
        .{ "prime1", "p" },     .{ "prime2", "q" },         .{ "exponent1", "dp" },
        .{ "exponent2", "dq" }, .{ "coefficient", "qi" },
    }) |field| @field(key, field[0]) = try jwkInteger(operation, value, field[1]);
    const der = try std.crypto.codecs.asn1.der.encode(a, key);
    defer {
        std.crypto.secureZero(u8, der);
        a.free(der);
    }
    return privatePem(a, "RSA PRIVATE KEY", der);
}
fn signingEcJwk(comptime Scheme: type, comptime oid: []const u8, operation: m.OperationContext, value: Value) !SigningKey {
    try jwkStrings(value, .{ "x", "y", "d" });
    const a = operation.allocator;
    const asn1 = std.crypto.codecs.asn1;
    var x: [Scheme.Curve.Fe.encoded_length]u8 = undefined;
    var y: [Scheme.Curve.Fe.encoded_length]u8 = undefined;
    try jwkEcCoordinate(operation, value, "x", &x);
    try jwkEcCoordinate(operation, value, "y", &y);
    const public = Scheme.PublicKey{ .p = Scheme.Curve.fromSerializedAffineCoordinates(x, y, .big) catch return invalidEcJwk(operation) };
    // JWK private d is an unsigned integer, without the coordinate-width
    // restriction. Preserve its decoded bytes for the SEC1 import owner.
    const bytes = try jwkBytes(operation, value, "d");
    defer {
        std.crypto.secureZero(u8, bytes);
        a.free(bytes);
    }
    const Sec1 = struct {
        version: i32,
        key: asn1.Opaque(.universal(.octetstring, false)),
        parameters: asn1.Oid,
        publicKey: asn1.BitString,
        pub const asn1_tags = .{ .parameters = asn1.FieldTag.initExplicit(0, .context_specific), .publicKey = asn1.FieldTag.initExplicit(1, .context_specific) };
    };
    const point = public.toUncompressedSec1();
    const der = try asn1.der.encode(a, Sec1{ .version = 1, .key = .{ .bytes = bytes }, .parameters = asn1.Oid.fromDotComptime(oid), .publicKey = .{ .bytes = &point } });
    defer {
        std.crypto.secureZero(u8, der);
        a.free(der);
    }
    return privatePem(a, "EC PRIVATE KEY", der);
}
fn signingJwk(operation: m.OperationContext, value: Value) !SigningKey {
    if (value != .object) return error.UnsupportedGoogleSigningKeyOptions;
    const kind = j.field(value, "kty") orelse return error.UnsupportedGoogleSigningKeyOptions;
    if (std.mem.eql(u8, kind, "RSA")) return signingRsaJwk(operation, value);
    if (std.mem.eql(u8, kind, "EC")) {
        const curve = j.field(value, "crv") orelse return error.UnsupportedGoogleSigningKeyOptions;
        if (std.mem.eql(u8, curve, "P-256")) return signingEcJwk(std.crypto.sign.ecdsa.EcdsaP256Sha256, "1.2.840.10045.3.1.7", operation, value);
        if (std.mem.eql(u8, curve, "P-384")) return signingEcJwk(std.crypto.sign.ecdsa.EcdsaP384Sha384, "1.3.132.0.34", operation, value);
        if (std.mem.eql(u8, curve, "secp256k1")) return signingEcJwk(std.crypto.sign.ecdsa.EcdsaSecp256k1Sha256, "1.3.132.0.10", operation, value);
    }
    const ed_curve = j.field(value, "crv") orelse "";
    if (std.mem.eql(u8, kind, "OKP") and (std.mem.eql(u8, ed_curve, "Ed25519") or std.mem.eql(u8, ed_curve, "Ed448"))) {
        // Node validates x as a string but private OKP import uses only d.
        _ = j.field(value, "x") orelse return error.UnsupportedGoogleSigningKeyOptions;
        const a = operation.allocator;
        const asn1 = std.crypto.codecs.asn1;
        const Octets = asn1.Opaque(.universal(.octetstring, false));
        const bytes = try jwkBytes(operation, value, "d");
        defer {
            std.crypto.secureZero(u8, bytes);
            a.free(bytes);
        }
        const ed448 = std.mem.eql(u8, ed_curve, "Ed448");
        if (bytes.len != (if (ed448) @as(usize, 57) else std.crypto.sign.Ed25519.KeyPair.seed_length)) return invalidJwk(operation);
        const inner = try asn1.der.encode(a, Octets{ .bytes = bytes });
        defer {
            std.crypto.secureZero(u8, inner);
            a.free(inner);
        }
        const der = try asn1.der.encode(a, .{ .version = @as(i32, 0), .algorithm = .{ .oid = if (ed448) asn1.Oid.fromDotComptime("1.3.101.113") else asn1.Oid.fromDotComptime("1.3.101.112") }, .key = Octets{ .bytes = inner } });
        defer {
            std.crypto.secureZero(u8, der);
            a.free(der);
        }
        return privatePem(a, "PRIVATE KEY", der);
    }
    return error.UnsupportedGoogleSigningKeyOptions;
}
fn signatureOptions(value: Value) !@import("../signing.zig").GoogleSignatureOptions {
    const padding = try signatureInteger(j.get(value, "padding"));
    const salt = try signatureInteger(j.get(value, "saltLength"));
    if (!keyOption(j.get(value, "dsaEncoding"), &.{ "der", "ieee-p1363" })) return error.UnsupportedGoogleSigningKeyOptions;
    return .{ .padding = padding, .saltLength = salt, .encoding = if (std.mem.eql(u8, j.field(value, "dsaEncoding") orelse "der", "ieee-p1363")) .ieee_p1363 else .der };
}
fn signingKey(operation: m.OperationContext, value: Value) !SigningKey {
    if (j.text(value)) |text| return signingKeyText(operation, text, null);
    if (value == .object) {
        if (std.mem.eql(u8, j.field(value, "format") orelse "", "jwk")) {
            var key = try signingJwk(operation, j.get(value, "key") orelse .null);
            errdefer key.deinit(operation.allocator);
            key.options = try signatureOptions(value);
            return key;
        }
        // Node reads the known import/signer options and ignores other own
        // properties. DER requires a type; PEM ignores a validated type.
        // A passphrase leaves unencrypted imports unchanged and is retained
        // only through local encrypted-key preparation when needed.
        const text = j.field(value, "key") orelse return error.UnsupportedGoogleSigningKeyOptions;
        var decoded = try signingKeyText(operation, text, j.get(value, "encoding"));
        defer decoded.deinit(operation.allocator);
        const der_input = std.mem.eql(u8, j.field(value, "format") orelse "pem", "der");
        const accepted_import = keyOption(j.get(value, "format"), &.{ "pem", "der" }) and
            keyOption(j.get(value, "type"), &.{ "pkcs1", "pkcs8", "sec1" }) and
            (!der_input or j.get(value, "type") != null) and
            (j.get(value, "passphrase") == null or j.get(value, "passphrase").? == .string);
        if (!accepted_import) return error.UnsupportedGoogleSigningKeyOptions;
        var passphrase: ?SigningKey = null;
        defer if (passphrase) |*converted| converted.deinit(operation.allocator);
        if (j.field(value, "passphrase")) |password| passphrase = try signingKeyText(operation, password, j.get(value, "encoding"));
        const password = if (passphrase) |converted| converted.pem else null;
        // Node prepares string buffers/import options, then validates signing
        // options, before its native handle parses PEM/DER key material.
        const options = try signatureOptions(value);
        const keys = @import("../../crypto/keys.zig");
        if (!der_input) {
            var remaining = decoded.pem;
            while (true) {
                const material = try keys.decodePrivatePemArmor(operation.allocator, operation.io, remaining);
                defer operation.allocator.free(material.label);
                defer {
                    std.crypto.secureZero(u8, material.bytes);
                    operation.allocator.free(material.bytes);
                }
                remaining = remaining[material.consumed..];
                var legacy: ?keys.DecryptedPrivateKey = null;
                defer if (legacy) |*opened| opened.deinit(operation.allocator);
                if (material.encryption) |encryption| legacy = try keys.decryptLegacyPem(operation.allocator, operation.io, encryption, material.bytes, password);
                if (!keys.isPrivatePemLabel(material.label)) continue;
                const payload = if (legacy) |opened| opened.plaintext else material.bytes;
                if (std.mem.eql(u8, material.label, "ENCRYPTED PRIVATE KEY")) {
                    var opened = (try keys.decryptPkcs8(operation.allocator, operation.io, payload, password)) orelse return error.InvalidPrivateKey;
                    defer opened.deinit(operation.allocator);
                    var key = try privatePem(operation.allocator, "PRIVATE KEY", opened.plaintext);
                    key.options = options;
                    return key;
                }
                var key = try privatePem(operation.allocator, material.label, payload);
                key.options = options;
                return key;
            }
        }
        const kind = j.field(value, "type").?;
        if (std.mem.eql(u8, kind, "pkcs8")) if (try keys.decryptPkcs8(operation.allocator, operation.io, decoded.pem, password)) |decrypted| {
            var opened = decrypted;
            defer opened.deinit(operation.allocator);
            var key = try privatePem(operation.allocator, "PRIVATE KEY", opened.plaintext);
            key.options = options;
            return key;
        };
        const label = if (std.mem.eql(u8, kind, "pkcs8")) "PRIVATE KEY" else try keys.autoPrivateDerLabel(decoded.pem);
        var key = try privatePem(operation.allocator, label, decoded.pem);
        key.options = options;
        return key;
    }
    if (value == .array) return error.UnsupportedGoogleSigningKeyOptions;
    return executableFailure(operation.errorMessage, error.InvalidGoogleSigningKeyType, "key must be a string, a buffer or an object");
}
fn appendGoogleForm(a: std.mem.Allocator, fields: *std.ArrayList(uri.Pair), name: []const u8, raw: ?Value) !void {
    const value = raw orelse return;
    // removeUndefinedValuesInObject runs on raw values before URLSearchParams
    // conversion and also removes the literal string "undefined".
    if (value == .string and std.mem.eql(u8, value.string, "undefined")) return;
    try fields.append(a, .{ .name = name, .value = try @import("../../utils/diagnostics.zig").formatThrownValue(a, .{ .value = value }) });
}
fn base64(a: std.mem.Allocator, bytes: []const u8) ![]const u8 {
    const out = try a.alloc(u8, std.base64.url_safe_no_pad.Encoder.calcSize(bytes.len));
    _ = std.base64.url_safe_no_pad.Encoder.encode(out, bytes);
    return out;
}
fn compactInput(a: std.mem.Allocator, header: Value, claims: Value) ![]const u8 {
    const head = try j.stringify(a, header, .{});
    defer a.free(head);
    const body = try j.stringify(a, claims, .{});
    defer a.free(body);
    // jws4 encodes its JSON header with Buffer's binary/latin1 conversion.
    // Standard Unicode decoding supplies JS UTF16 units; the binary-string
    // boundary narrows each unit, including each half of a surrogate pair.
    const units = try std.unicode.wtf8ToWtf16LeAlloc(a, head);
    defer a.free(units);
    const binary = try a.alloc(u8, units.len);
    defer a.free(binary);
    for (units, binary) |unit, *byte| byte.* = @truncate(std.mem.littleToNative(u16, unit));
    const encoded_head = try base64(a, binary);
    defer a.free(encoded_head);
    const encoded_body = try base64(a, body);
    defer a.free(encoded_body);
    var writer = std.Io.Writer.Allocating.init(a);
    defer writer.deinit();
    try writer.writer.writeAll(encoded_head);
    try writer.writer.writeByte('.');
    try writer.writer.writeAll(encoded_body);
    return writer.toOwnedSlice();
}
fn serviceAccount(operation: m.OperationContext, configured: *CredentialConfiguration, purpose: enum { headers, access_token }) !Token {
    const credentials = configured.credentials;
    if (!j.truthy(credentials)) {
        if (operation.errorMessage) |slot| slot.* = "Must pass in a JSON object containing the service account auth settings.";
        return error.InvalidGoogleCredentials;
    }
    const email = try credentialValue(operation, credentials, "client_email");
    const raw_key = try credentialValue(operation, credentials, "private_key");
    const a = operation.allocator;
    const io = operation.io;
    const now = @divFloor(std.Io.Clock.real.now(io).toMilliseconds(), 1000);
    const raw_universe = j.get(credentials, "universe_domain") orelse .null;
    const self_signed = purpose == .headers and !sameUniverse(if (j.truthy(raw_universe)) raw_universe else j.str("googleapis.com"), j.str("googleapis.com"));
    var header = try j.from(a, .{ .alg = "RS256" });
    if (self_signed) {
        try j.put(a, &header, "typ", j.str("JWT"));
        const id = j.get(credentials, "private_key_id") orelse .null;
        if (j.truthy(id)) try j.put(a, &header, "kid", try j.clone(a, id));
    }
    // The two pinned signers construct different objects; their property
    // order is part of the compact bytes signed and sent to Google.
    const claims = if (self_signed) try j.from(a, .{ .iss = email, .sub = email, .scope = "https://www.googleapis.com/auth/cloud-platform", .exp = now + 3600, .iat = now }) else try j.from(a, .{ .iss = email, .scope = "https://www.googleapis.com/auth/cloud-platform", .aud = "https://oauth2.googleapis.com/token", .exp = now + 3600, .iat = now });
    const input = try compactInput(a, header, claims);
    const signing = @import("../signing.zig");
    var signed = signed: {
        var key = try signingKey(operation, raw_key);
        defer key.deinit(a);
        break :signed signing.googleWithOptions(a, io, .{ .client_email = signing.view(j.text(email) orelse @as([]const u8, "")), .private_key_id = signing.view(j.field(credentials, "private_key_id") orelse @as([]const u8, "")), .private_key = signing.view(key.pem), .project_id = signing.view(j.field(credentials, "project_id") orelse @as([]const u8, "")), .content = signing.view(input) }, key.options) catch |err| {
            if (err == error.GoogleUnsupportedCryptoOperation) return executableFailure(operation.errorMessage, err, "Unsupported crypto operation");
            return err;
        };
    };
    defer signed.deinit();
    var assertion = std.Io.Writer.Allocating.init(a);
    defer assertion.deinit();
    try assertion.writer.writeAll(input);
    try assertion.writer.writeByte('.');
    try assertion.writer.writeAll(try base64(a, try signed.signature.slice()));
    if (self_signed) return .{ .access = j.str(try assertion.toOwnedSlice()), .expires = @as(f64, @floatFromInt(now + 3600)) * 1000, .quotaProject = j.field(credentials, "quota_project_id"), .quotaValue = j.get(credentials, "quota_project_id") };
    const response = try http.postFormWithPolicy(a, io, operation.transport, "https://oauth2.googleapis.com/token", &.{ .{ .name = "grant_type", .value = "urn:ietf:params:oauth:grant-type:jwt-bearer" }, .{ .name = "assertion", .value = assertion.written() } }, diagnosticPolicy(operation, .{ .authClientHeaders = true, .failure = &configured.failure, .failureName = &configured.failureName }));
    if (!response.ok()) {
        const diagnostics = @import("../../utils/diagnostics.zig");
        const detail = j.get(response.body, "error") orelse .null;
        var message = if (j.truthy(detail)) try std.fmt.allocPrint(a, "{s}: {s}", .{ diagnostics.formatThrownValue(a, .{ .value = detail }) catch |err| return replacementFailure(operation, err, configured), diagnostics.formatThrownValue(a, if (j.get(response.body, "error_description")) |value| .{ .value = value } else .undefined_value) catch |err| return replacementFailure(operation, err, configured) }) else try http.errorMessage(a, response);
        if (purpose == .headers and (response.status == 403 or response.status == 404)) message = try std.fmt.allocPrint(a, "Could not refresh access token: {s}", .{message});
        if (operation.errorMessage) |slot| slot.* = message;
        return error.GoogleServiceAccountTokenFailed;
    }
    return fromTokenResponse(operation, response.body, j.get(credentials, "quota_project_id"), .jwt);
}
fn metadataObject(value: Value) bool {
    return switch (value) {
        .object => true,
        .array => |array| objects: {
            for (array.items) |item| if (metadataObject(item)) break :objects true;
            break :objects false;
        },
        else => false,
    };
}
fn fromTokenResponse(operation: m.OperationContext, value: Value, quota: ?Value, kind: enum { jwt, oauth, compute, sts, sts_impersonation, external_refresh }) !Token {
    const a = operation.allocator;
    const oauth = kind == .oauth or kind == .compute;
    const jwt_now = if (kind == .jwt) currentMilliseconds(operation.io) else null;
    if (value == .null) {
        if (operation.errorMessage) |slot| slot.* = switch (kind) {
            .jwt => "Cannot read properties of null (reading 'expires_in')",
            .oauth, .compute => "Cannot set properties of null (setting 'refresh_token')",
            .sts, .sts_impersonation, .external_refresh => "Cannot set properties of null (setting 'res')",
        };
        return error.InvalidGoogleTokenResponse;
    }
    if (kind != .jwt and value != .object and value != .array) {
        if (operation.errorMessage) |slot| {
            const type_name: []const u8 = switch (value) {
                .string => "string",
                .bool => "boolean",
                else => "number",
            };
            const text = try @import("../../utils/diagnostics.zig").formatThrownValue(a, .{ .value = value });
            slot.* = try std.fmt.allocPrint(a, "Cannot create property '{s}' on {s} '{s}'", .{ if (oauth) "refresh_token" else "res", type_name, text });
        }
        return error.InvalidGoogleTokenResponse;
    }
    const configured = j.get(value, "expires_in");
    const numbers = @import("../../utils/numbers.zig");
    const expires: ?f64 = switch (kind) {
        .jwt => jwt_now.? + (try numbers.coerceValue(a, j.getNonNull(value, "expires_in") orelse j.int(0))) * 1000,
        .external_refresh => currentMilliseconds(operation.io) + (try numbers.coerceValue(a, configured)) * 1000,
        .sts_impersonation => null,
        .compute => compute: {
            if (!j.truthy(configured orelse .null)) break :compute null;
            const now = currentMilliseconds(operation.io);
            if (metadataObject(configured.?)) return primitiveFailure(operation, error.CannotConvertObjectToPrimitive, false);
            break :compute now + (try numbers.coerceValue(a, configured)) * 1000;
        },
        .oauth, .sts => if (j.truthy(configured orelse .null)) currentMilliseconds(operation.io) + (try numbers.coerceValue(a, configured)) * 1000 else null,
    };
    return .{ .access = j.get(value, "access_token"), .accessFromMetadata = kind == .compute, .accessTruthy = j.truthy(j.get(value, "access_token") orelse .null), .tokenType = if (oauth) j.get(value, "token_type") else null, .expires = expires, .expiryDate = if (oauth and !j.truthy(configured orelse .null)) j.get(value, "expiry_date") else null, .quotaProject = if (quota) |value_quota| j.text(value_quota) else null, .quotaValue = quota };
}
fn authorizedUser(operation: m.OperationContext, configured: *CredentialConfiguration, for_headers: bool) !Token {
    const credentials = configured.credentials;
    const a = operation.allocator;
    const diagnostics = @import("../../utils/diagnostics.zig");
    var fields: std.ArrayList(uri.Pair) = .empty;
    for ([_][]const u8{ "refresh_token", "client_id", "client_secret" }) |name| try appendGoogleForm(a, &fields, name, try credentialValue(operation, credentials, name));
    try fields.append(a, .{ .name = "grant_type", .value = "refresh_token" });
    const response = try http.postFormWithPolicy(operation.allocator, operation.io, operation.transport, "https://oauth2.googleapis.com/token", fields.items, diagnosticPolicy(operation, .{ .responseType = .auto, .authClientHeaders = true, .failure = &configured.failure, .failureName = &configured.failureName }));
    if (!response.ok()) {
        var message = try http.errorMessage(a, response);
        if (std.mem.eql(u8, message, "invalid_grant")) {
            const description = diagnostics.formatThrownValue(a, if (j.get(response.body, "error_description")) |value| .{ .value = value } else .undefined_value) catch |err| return replacementFailure(operation, err, configured);
            if (try @import("../../regex/regexp.zig").matches(a, operation.io, "ReAuth", description, .{ .ignoreCase = true })) message = try j.stringify(a, response.body, .{});
        }
        if (for_headers and (response.status == 403 or response.status == 404)) message = try std.fmt.allocPrint(a, "Could not refresh access token: {s}", .{message});
        if (operation.errorMessage) |slot| slot.* = message;
        return error.GoogleAuthorizedUserTokenFailed;
    }
    return fromTokenResponse(operation, response.body, j.get(credentials, "quota_project_id"), .oauth);
}
fn externalAuthorizedUser(operation: m.OperationContext, configured: *CredentialConfiguration) !Token {
    const credentials = configured.credentials;
    const a = operation.allocator;
    const diagnostics = @import("../../utils/diagnostics.zig");
    const client_id = if (j.get(credentials, "client_id")) |value| try diagnostics.formatThrownValue(a, .{ .value = value }) else "undefined";
    const raw_secret = j.get(credentials, "client_secret") orelse .null;
    const client_secret = if (j.truthy(raw_secret)) try diagnostics.formatThrownValue(a, .{ .value = raw_secret }) else "";
    const combined = try std.fmt.allocPrint(a, "{s}:{s}", .{ client_id, client_secret });
    const units = try std.unicode.wtf8ToWtf16LeAlloc(a, combined);
    defer a.free(units);
    const utf8 = try std.fmt.allocPrint(a, "{f}", .{std.unicode.fmtUtf16Le(units)});
    defer a.free(utf8);
    const encoded = try a.alloc(u8, std.base64.standard.Encoder.calcSize(utf8.len));
    _ = std.base64.standard.Encoder.encode(encoded, utf8);
    const refresh = try diagnostics.formatThrownValue(a, if (configured.refreshToken) |value| .{ .value = value } else .undefined_value);
    const body = try uri.encodeForm(a, &.{ .{ .name = "grant_type", .value = "refresh_token" }, .{ .name = "refresh_token", .value = refresh } });
    const response = try http.requestWithPolicy(a, operation.io, operation.transport, .POST, "", body, "application/x-www-form-urlencoded;charset=UTF-8", &.{.{ .name = "authorization", .value = try std.fmt.allocPrint(a, "Basic {s}", .{encoded}) }}, diagnosticPolicy(operation, .{ .rawUrl = configured.tokenEndpoint, .authClientHeaders = authHeadersEnabled(credentials), .failure = &configured.failure, .failureName = &configured.failureName }));
    if (!response.ok()) return rejectOAuth(operation, response, error.GoogleExternalAuthorizedUserTokenFailed, configured);
    var token = try fromTokenResponse(operation, response.body, j.get(credentials, "quota_project_id"), .external_refresh);
    token.authClientHeaders = authHeadersEnabled(credentials);
    // Response nodes use the operation allocator and remain alive with this
    // per-operation configuration. The supplied credential tree is untouched.
    if (j.get(response.body, "refresh_token")) |replacement| configured.refreshToken = replacement;
    return token;
}
fn subjectHeaderString(operation: m.OperationContext, a: std.mem.Allocator, value: Value) ![]const u8 {
    const text = try @import("../../utils/diagnostics.zig").formatThrownValue(a, .{ .value = value });
    if (try headers.byteStringFailure(a, text)) |failure| {
        const message = try std.fmt.allocPrint(a, "Cannot convert argument to a ByteString because the character at index {d} has a value of {d} which is greater than 255.", .{ failure.index, failure.unit });
        defer a.free(message);
        if (operation.errorMessage) |slot| slot.* = if (operation.errorStorage) |storage| try storage.copy(message) else try operation.allocator.dupe(u8, message);
        return error.InvalidHeaderByteString;
    }
    return text;
}
fn subjectHeaders(operation: m.OperationContext, a: std.mem.Allocator, raw: ?Value) !t.ProviderHeaders {
    const value = raw orelse return &.{};
    const Pair = struct { count: usize, name: []const u8 = "", value: []const u8 = "" };
    var converted: std.ArrayList(Pair) = .empty;
    defer converted.deinit(a);
    if (value == .object) {
        const entries = try j.objectEntries(a, value.object);
        defer a.free(entries);
        for (entries) |entry| {
            const name = try subjectHeaderString(operation, a, j.str(entry.key));
            const text = try subjectHeaderString(operation, a, entry.node.*);
            if (std.mem.eql(u8, name, "__proto__")) continue;
            try converted.append(a, .{ .count = 2, .name = name, .value = text });
        }
    } else if (value == .array) {
        for (value.array.items, 0..) |pair, index| {
            if (pair != .array) {
                const message = if (pair == .object) try std.fmt.allocPrint(operation.allocator, "Headers constructor: init[{d}] is not iterable.", .{index}) else message: {
                    const text = try @import("../../utils/diagnostics.zig").formatThrownValue(a, .{ .value = pair });
                    const quote = if (pair == .string) "\"" else "";
                    break :message try std.fmt.allocPrint(operation.allocator, "Headers constructor: init[{d}] ({s}{s}{s}) is not iterable.", .{ index, quote, text, quote });
                };
                return executableFailure(operation.errorMessage, error.InvalidGoogleSubjectTokenHeaders, message);
            }
            var item = Pair{ .count = pair.array.items.len };
            for (pair.array.items, 0..) |raw_part, part_index| {
                const text = try subjectHeaderString(operation, a, raw_part);
                if (part_index == 0) item.name = text else if (part_index == 1) item.value = text;
            }
            try converted.append(a, item);
        }
    } else return executableFailure(operation.errorMessage, error.InvalidGoogleSubjectTokenHeaders, "Headers constructor: Argument 1 could not be converted to one of: sequence<sequence<ByteString>>, record<ByteString, ByteString>.");
    var result: std.ArrayList(t.ProviderHeader) = .empty;
    for (converted.items) |pair| {
        if (pair.count != 2) return executableFailure(operation.errorMessage, error.InvalidGoogleSubjectTokenHeaders, try std.fmt.allocPrint(operation.allocator, "Headers constructor: expected name/value pair to be length 2, found {d}.", .{pair.count}));
        const cookie = std.ascii.eqlIgnoreCase(pair.name, "cookie");
        var single: std.ArrayList(t.ProviderHeader) = .empty;
        defer single.deinit(a);
        var diagnostic: ?[]const u8 = null;
        defer if (diagnostic) |message| a.free(message);
        headers.appendFetch(a, if (cookie) &single else &result, pair.name, pair.value, &diagnostic) catch |err| {
            if (operation.errorMessage) |slot| {
                if (diagnostic) |message| {
                    slot.* = if (operation.errorStorage) |storage| try storage.copy(message) else try operation.allocator.dupe(u8, message);
                }
            }
            return err;
        };
        if (cookie) {
            const normalized = single.items[0];
            for (result.items) |*previous| {
                if (std.ascii.eqlIgnoreCase(previous.name, "cookie")) {
                    previous.value = try std.fmt.allocPrint(a, "{s}; {s}", .{ previous.value.?, normalized.value.? });
                    break;
                }
            } else try result.append(a, normalized);
        }
    }
    return result.toOwnedSlice(a);
}
fn identityPoolSubject(operation: m.OperationContext, storage: std.mem.Allocator, value: Value, field: ?Value, origin: enum { file, url }) !Value {
    const token = if (field) |raw| blk: {
        if (value == .null) {
            if (operation.errorMessage) |slot| {
                const display: ?[]const u8 = switch (raw) {
                    .array => null,
                    .object => if (j.get(raw, "toString") != null or j.get(raw, "constructor") != null) null else "#<Object>",
                    else => try @import("../../utils/diagnostics.zig").formatThrownValue(storage, .{ .value = raw }),
                };
                slot.* = if (display) |name| try std.fmt.allocPrint(operation.allocator, "Cannot read properties of null (reading '{s}')", .{name}) else "Cannot read properties of null";
            }
            return error.InvalidGoogleTokenResponse;
        }
        const name = try @import("../../utils/diagnostics.zig").formatThrownValue(storage, .{ .value = raw });
        if (std.mem.eql(u8, name, "length")) switch (value) {
            .array => |array| break :blk j.num(@floatFromInt(array.items.len)),
            .string => |text| break :blk j.num(@floatFromInt(try @import("../../utils/unicode.zig").utf16Length(text))),
            else => {},
        };
        const properties = if (value == .object) value else try j.objectSpread(storage, value);
        break :blk j.get(properties, name) orelse .null;
    } else value;
    if (!j.truthy(token)) {
        if (operation.errorMessage) |slot| slot.* = switch (origin) {
            .file => "Unable to parse the subject_token from the credential_source file",
            .url => "Unable to parse the subject_token from the credential_source URL",
        };
        return error.GoogleSubjectTokenFailed;
    }
    return token;
}
fn externalAccount(operation: m.OperationContext, options: t.StreamOptions, configured: *CredentialConfiguration) !Token {
    const original = configured.credentials;
    const credentials = try @import("options.zig").external(operation.allocator, original);
    const a = operation.allocator;
    const source = j.get(credentials, "credential_source") orelse .null;
    if (!j.truthy(source)) {
        if (j.truthy(j.get(credentials, "subject_token_supplier") orelse .null)) return executableFailure(operation.errorMessage, error.InvalidGoogleSubjectTokenSupplier, "this.subjectTokenSupplier.getSubjectToken is not a function");
        if (operation.errorMessage) |slot| slot.* = "A credential source or subject token supplier must be specified.";
        return error.InvalidGoogleExternalAccount;
    }
    // The SDK factory selects a client before its constructors apply aliases.
    const factory_source = j.get(original, "credential_source") orelse .null;
    const aws_source = j.truthy(j.get(factory_source, "environment_id") orelse .null);
    const executable_source = !aws_source and j.truthy(j.get(factory_source, "executable") orelse .null);
    var certificate_material: ?certificates.Material = null;
    defer if (certificate_material) |*material| material.deinit();
    var client_certificate: ?network.ClientCertificate = null;
    var subject: Value = undefined;
    if (aws_source) {
        subject = j.str(try awsSubjectToken(operation, options, credentials, configured));
    } else if (executable_source) subject = try executableSubjectToken(operation, options, credentials, j.get(source, "executable").?) else {
        const file = j.truthy(j.get(source, "file") orelse .null);
        const raw_url = j.get(source, "url") orelse .null;
        const url = if (j.truthy(raw_url)) raw_url else null;
        const certificate = if (j.get(source, "certificate")) |certificate_value| (if (j.truthy(certificate_value)) certificate_value else null) else null;
        const format = j.get(source, "format") orelse .null;
        const kind_value = j.get(format, "type") orelse .null;
        const kind = if (j.truthy(kind_value)) j.text(kind_value) orelse return error.InvalidGoogleSubjectTokenFormat else "text";
        if (!std.mem.eql(u8, kind, "text") and !std.mem.eql(u8, kind, "json")) {
            if (operation.errorMessage) |slot| slot.* = try std.fmt.allocPrint(a, "Invalid credential_source format \"{s}\"", .{kind});
            return error.InvalidGoogleSubjectTokenFormat;
        }
        const field: ?Value = if (std.mem.eql(u8, kind, "json")) blk: {
            const configured_field = j.get(format, "subject_token_field_name") orelse .null;
            if (!j.truthy(configured_field)) {
                if (operation.errorMessage) |slot| slot.* = "Missing subject_token_field_name for JSON credential_source format";
                return error.InvalidGoogleSubjectTokenFormat;
            }
            break :blk configured_field;
        } else null;
        if (@as(u8, @intFromBool(file)) + @as(u8, @intFromBool(url != null)) + @as(u8, @intFromBool(certificate != null)) != 1) {
            if (operation.errorMessage) |slot| slot.* = "No valid Identity Pool \"credential_source\" provided, must be either file, url, or certificate.";
            return error.InvalidGoogleExternalCredentialSource;
        }
        if (file) {
            const token = if (operation.authContext) |context| context.inputs.googleSubjectToken orelse return error.GoogleSubjectTokenUnavailable else return error.GoogleSubjectTokenUnavailable;
            if (token.len == 0) return executableFailure(operation.errorMessage, error.GoogleSubjectTokenFailed, "Unable to parse the subject_token from the credential_source file");
            subject = j.str(try a.dupe(u8, token));
        } else if (url) |endpoint_value| {
            var temporary = std.heap.ArenaAllocator.init(a);
            defer temporary.deinit();
            const storage = temporary.allocator();
            const extra = try subjectHeaders(operation, storage, j.get(source, "headers"));
            const response = try http.requestWithPolicy(storage, operation.io, operation.transport, .GET, "", null, null, extra, diagnosticPolicy(operation, .{ .rawUrl = endpoint_value, .responseType = if (field != null) .json else .text, .authClientHeaders = authHeadersEnabled(credentials), .failure = &configured.failure, .failureName = &configured.failureName }));
            if (!response.ok()) return reject(operation, response, error.GoogleSubjectTokenFailed);
            const token = try identityPoolSubject(operation, storage, response.body, field, .url);
            subject = try j.clone(a, token);
        } else {
            certificate_material = try certificates.load(operation, certificate.?);
            subject = j.str(certificate_material.?.subjectToken);
            client_certificate = certificate_material.?.clientCertificate;
        }
    }
    if (!j.truthy(subject)) return error.GoogleSubjectTokenFailed;
    const diagnostics = @import("../../utils/diagnostics.zig");
    const token_url = try diagnostics.formatThrownValue(a, .{ .value = configured.tokenEndpoint });
    var fields: std.ArrayList(uri.Pair) = .empty;
    try fields.append(a, .{ .name = "grant_type", .value = "urn:ietf:params:oauth:grant-type:token-exchange" });
    try appendGoogleForm(a, &fields, "audience", j.get(credentials, "audience"));
    try fields.appendSlice(a, &.{
        .{ .name = "scope", .value = "https://www.googleapis.com/auth/cloud-platform" }, .{ .name = "requested_token_type", .value = "urn:ietf:params:oauth:token-type:access_token" },
    });
    try appendGoogleForm(a, &fields, "subject_token", subject);
    try appendGoogleForm(a, &fields, "subject_token_type", j.get(credentials, "subject_token_type"));
    var extra: std.ArrayList(t.ProviderHeader) = .empty;
    const source_type: []const u8 = if (aws_source) "aws" else if (executable_source) "executable" else if (j.truthy(j.get(source, "file") orelse .null)) "file" else if (j.truthy(j.get(source, "url") orelse .null)) "url" else "certificate";
    const lifetime_requested = j.truthy(j.get(j.get(credentials, "service_account_impersonation") orelse .null, "token_lifetime_seconds") orelse .null);
    const impersonation_option = j.getNonNull(original, "service_account_impersonation_url") orelse j.get(original, "serviceAccountImpersonationUrl");
    try extra.append(a, .{ .name = "x-goog-api-client", .value = try std.fmt.allocPrint(a, "gl-node/unknown auth/10.6.2 google-byoid-sdk source/{s} sa-impersonation/{s} config-lifetime/{s}", .{ source_type, if (impersonation_option != null) "true" else "false", if (lifetime_requested) "true" else "false" }) });
    const raw_client = j.get(credentials, "client_id") orelse .null;
    if (j.truthy(raw_client)) {
        const raw_secret = j.get(credentials, "client_secret") orelse .null;
        const combined = try std.fmt.allocPrint(a, "{s}:{s}", .{ try diagnostics.formatThrownValue(a, .{ .value = raw_client }), if (j.truthy(raw_secret)) try diagnostics.formatThrownValue(a, .{ .value = raw_secret }) else "" });
        const units = try std.unicode.wtf8ToWtf16LeAlloc(a, combined);
        defer a.free(units);
        const utf8 = try std.fmt.allocPrint(a, "{f}", .{std.unicode.fmtUtf16Le(units)});
        defer a.free(utf8);
        const encoded = try a.alloc(u8, std.base64.standard.Encoder.calcSize(utf8.len));
        _ = std.base64.standard.Encoder.encode(encoded, utf8);
        try extra.append(a, .{ .name = "authorization", .value = try std.fmt.allocPrint(a, "Basic {s}", .{encoded}) });
    } else if (j.get(credentials, "workforce_pool_user_project")) |project| {
        if (j.truthy(project)) try fields.append(a, .{ .name = "options", .value = try @import("../../json/value.zig").stringify(a, .{ .userProject = project }, .{}) });
    }
    const response = http.requestWithPolicy(a, operation.io, operation.transport, .POST, token_url, try uri.encodeForm(a, fields.items), "application/x-www-form-urlencoded;charset=UTF-8", extra.items, diagnosticPolicy(operation, .{ .clientCertificate = client_certificate, .failure = &configured.failure, .failureName = &configured.failureName })) catch |err| {
        if (err == error.MtlsTransportUnavailable) {
            if (operation.errorMessage) |slot| slot.* = "Client-certificate HTTPS is unavailable in this transport.";
        }
        return err;
    };
    if (!response.ok()) return rejectOAuth(operation, response, error.GoogleStsTokenExchangeFailed, configured);
    const raw_impersonation = j.get(credentials, "service_account_impersonation_url") orelse .null;
    const impersonating = j.truthy(raw_impersonation);
    var token = try fromTokenResponse(operation, response.body, j.get(credentials, "quota_project_id"), if (impersonating) .sts_impersonation else .sts);
    if (impersonating) {
        token = try impersonate(operation, raw_impersonation, token, credentials, client_certificate, configured, null);
    }
    token.authClientHeaders = client_certificate == null and authHeadersEnabled(credentials);
    return token;
}
const SourceRequest = struct { options: t.StreamOptions, configuration: *CredentialConfiguration };
fn impersonate(operation: m.OperationContext, url: Value, source: Token, credentials: Value, client_certificate: ?network.ClientCertificate, configuration: *CredentialConfiguration, source_request: ?SourceRequest) !Token {
    const a = operation.allocator;
    const impersonated = std.mem.eql(u8, j.field(credentials, "type") orelse @as([]const u8, ""), "impersonated_service_account");
    // BaseExternal builds its explicit Authorization header before its data
    // object. Impersonated builds data before sourceClient.request gets headers.
    const external_authorization = if (!impersonated) try std.fmt.allocPrint(a, "Bearer {s}", .{try accessText(operation, source, false)}) else null;
    const lifetime_value = if (impersonated) j.getNonNull(credentials, "lifetime") orelse j.int(3600) else blk: {
        const configured = j.get(j.get(credentials, "service_account_impersonation") orelse .null, "token_lifetime_seconds") orelse .null;
        break :blk if (j.truthy(configured)) configured else j.int(3600);
    };
    const lifetime = @import("../../utils/diagnostics.zig").formatThrownValue(a, .{ .value = lifetime_value }) catch |err| {
        if (err == error.CannotConvertObjectToPrimitive) return executableFailure(operation.errorMessage, err, "Cannot convert object to primitive value");
        return err;
    };
    var value = j.object();
    if (impersonated) try j.put(a, &value, "delegates", j.getNonNull(credentials, "delegates") orelse j.array(a));
    try j.put(a, &value, "scope", try j.from(a, [_][]const u8{"https://www.googleapis.com/auth/cloud-platform"}));
    try j.put(a, &value, "lifetime", j.str(try std.fmt.allocPrint(a, "{s}s", .{lifetime})));
    const body = try @import("../../json/value.zig").stringify(a, value, .{});
    var active_source = if (source_request) |request| acquireCredentials(operation, request.options, request.configuration, .headers) catch |err| return forwardSourceFailure(operation, err, configuration, request.configuration) else source;
    var auth_retried = false;
    const response = while (true) {
        var extra: std.ArrayList(t.ProviderHeader) = .empty;
        if (active_source.apiKey) |key| {
            try headers.appendFetch(a, &extra, "x-goog-api-key", try @import("../../utils/diagnostics.zig").formatThrownValue(a, .{ .value = key }), operation.errorMessage);
        } else {
            const authorization = external_authorization orelse try std.fmt.allocPrint(a, "{s} {s}", .{ active_source.scheme, try accessText(operation, active_source, false) });
            if (impersonated) try headers.appendFetch(a, &extra, "authorization", authorization, operation.errorMessage) else try extra.append(a, .{ .name = "authorization", .value = authorization });
            if (impersonated) if (try quotaHeader(a, active_source)) |project| try headers.appendFetch(a, &extra, "x-goog-user-project", project, operation.errorMessage);
            if (source_request) |request| if (oauthApiKey(request.configuration.credentials)) |key| try headers.appendFetch(a, &extra, "x-goog-api-key", try @import("../../utils/diagnostics.zig").formatThrownValue(a, .{ .value = key }), operation.errorMessage);
        }
        const received = try http.requestWithPolicy(a, operation.io, operation.transport, .POST, "", body, "application/json", extra.items, diagnosticPolicy(operation, .{ .rawUrl = url, .clientCertificate = client_certificate, .authClientHeaders = if (impersonated) active_source.authClientHeaders else client_certificate == null and authHeadersEnabled(credentials), .failure = &configuration.failure, .failureName = &configuration.failureName }));
        if (!auth_retried and (received.status == 401 or received.status == 403)) if (source_request) |request| {
            const config = request.configuration;
            const kind = j.field(config.credentials, "type") orelse "";
            const external = std.mem.eql(u8, kind, "external_account") or std.mem.eql(u8, kind, "external_account_authorized_user");
            const supports_option = external or std.mem.eql(u8, kind, "impersonated_service_account");
            const force = supports_option and j.truthy(j.get(config.credentials, "forceRefreshOnFailure") orelse .null);
            const token = config.token orelse active_source;
            const has_expiry = hasExpiry(token);
            if (if (external) force else token.accessTruthy and (!has_expiry or force)) {
                auth_retried = true;
                if (configuration.failure) |*failure| failure.deinit();
                configuration.failure = null;
                configuration.failureName = null;
                config.token = refreshCredentials(operation, request.options, config, false) catch |err| return forwardSourceFailure(operation, err, configuration, config);
                active_source = acquireCredentials(operation, request.options, config, .headers) catch |err| return forwardSourceFailure(operation, err, configuration, config);
                continue;
            }
        };
        break received;
    };
    if (!response.ok()) return reject(operation, response, error.GoogleImpersonationFailed);
    if (response.body == .null) return executableFailure(operation.errorMessage, error.InvalidGoogleTokenResponse, "Cannot read properties of null (reading 'accessToken')");
    const raw_access = j.get(response.body, "accessToken");
    return .{ .access = raw_access, .accessTruthy = j.truthy(raw_access orelse .null), .authClientHeaders = authHeadersEnabled(credentials), .expires = try impersonatedExpiry(operation, j.get(response.body, "expireTime"), impersonated), .quotaProject = if (impersonated) j.field(credentials, "quota_project_id") orelse j.field(credentials, "quotaProjectId") else source.quotaProject, .quotaValue = if (impersonated) j.getNonNull(credentials, "quota_project_id") orelse j.getNonNull(credentials, "quotaProjectId") else source.quotaValue };
}
fn impersonatedExpiry(operation: m.OperationContext, raw: ?Value, date_parse: bool) !f64 {
    const a = operation.allocator;
    const diagnostics = @import("../../utils/diagnostics.zig");
    if (!date_parse) if (raw) |value| switch (value) {
        .null => return 0,
        .bool, .integer, .float, .number_string => {
            const numeric = try @import("../../utils/numbers.zig").coerceValue(a, value);
            if (!std.math.isFinite(numeric) or @abs(numeric) > 8.64e15) return std.math.nan(f64);
            const clipped = @trunc(numeric);
            return if (clipped == 0) 0 else clipped;
        },
        else => {},
    };
    const text = diagnostics.formatThrownValue(a, if (raw) |value| .{ .value = value } else .undefined_value) catch |err| {
        if (err == error.CannotConvertObjectToPrimitive) return executableFailure(operation.errorMessage, err, "Cannot convert object to primitive value");
        return err;
    };
    defer if (raw != null) a.free(text);
    return @import("../../utils/datetime.zig").parseJavaScriptDate(a, operation.io, text);
}
const CredentialConfiguration = struct {
    credentials: Value,
    universe: Value,
    source: ?*CredentialConfiguration = null,
    targetPrincipal: ?[]const u8 = null,
    impersonationEndpoint: Value = .null,
    ownedEndpoint: ?[]const u8 = null,
    tokenEndpoint: Value = .null,
    ownedTokenEndpoint: ?[]const u8 = null,
    token: ?Token = null,
    selfSigned: ?Token = null,
    /// null denotes an absent property; Value.null is a present JSON null.
    /// Nodes borrow caller input or an operation-owned refresh response.
    refreshToken: ?Value = null,
    awsRegion: ?[]const u8 = null,
    failure: ?http.FailureResponse = null,
    failureName: ?[]const u8 = null,
    fn deinit(self: *@This(), a: std.mem.Allocator) void {
        if (self.source) |source| {
            source.deinit(a);
            a.destroy(source);
        }
        if (self.targetPrincipal) |target| a.free(target);
        if (self.ownedEndpoint) |endpoint| a.free(endpoint);
        if (self.ownedTokenEndpoint) |endpoint| a.free(endpoint);
        if (self.failure) |*failure| failure.deinit();
    }
};
fn sameUniverse(left: Value, right: Value) bool {
    if (j.number(left)) |number| return if (j.number(right)) |other| number == other else false;
    return switch (left) {
        .string => |text| right == .string and std.mem.eql(u8, text, right.string),
        .bool => |value| right == .bool and value == right.bool,
        .null => right == .null,
        // Container identity is retained when the same supplied tree node is
        // reused; this is not structural JSON equality.
        .array => |items| right == .array and items.items.len != 0 and items.items.ptr == right.array.items.ptr,
        .object => |items| right == .object and items.count() != 0 and items.keys().ptr == right.object.keys().ptr,
        else => false,
    };
}
fn configureExternal(operation: m.OperationContext, original: Value) !void {
    var temporary = std.heap.ArenaAllocator.init(operation.allocator);
    defer temporary.deinit();
    const a = temporary.allocator();
    const credentials = try @import("options.zig").external(a, original);
    const diagnostics = @import("../../utils/diagnostics.zig");
    const universe = j.getNonNull(credentials, "universe_domain") orelse j.str("googleapis.com");
    const custom_manager = j.getNonNull(credentials, "cloud_resource_manager_url") orelse j.getNonNull(credentials, "cloudResourceManagerUrl") orelse .null;
    const manager = if (j.truthy(custom_manager)) try diagnostics.formatThrownValue(a, .{ .value = custom_manager }) else try std.fmt.allocPrint(a, "https://cloudresourcemanager.{s}/v1/projects/", .{try diagnostics.formatThrownValue(a, .{ .value = universe })});
    _ = uri.parseHttpUrl(a, manager) catch |err| {
        if (err == error.OutOfMemory or err == error.Canceled) return err;
        return executableFailure(operation.errorMessage, error.InvalidGoogleCredentials, "Invalid URL");
    };
    const audience_value = j.get(credentials, "audience");
    const workforce = j.truthy(j.get(credentials, "workforce_pool_user_project") orelse .null);
    const audience = if (audience_value) |value| j.text(value) orelse return executableFailure(operation.errorMessage, error.InvalidGoogleCredentials, if (value == .null) "Cannot read properties of null (reading 'match')" else if (workforce) "this.audience.match is not a function" else "audience.match is not a function") else return executableFailure(operation.errorMessage, error.InvalidGoogleCredentials, "Cannot read properties of undefined (reading 'match')");
    if (j.truthy(j.get(credentials, "workforce_pool_user_project") orelse .null) and !try @import("../../regex/regexp.zig").matches(a, operation.io, "//iam\\.googleapis\\.com/locations/[^/]+/workforcePools/[^/]+/providers/.+", audience, .{})) return executableFailure(operation.errorMessage, error.InvalidGoogleWorkforceAudience, "workforcePoolUserProject should not be set for non-workforce pool credentials.");
    const source = j.get(credentials, "credential_source") orelse .null;
    const factory_source = j.get(original, "credential_source") orelse .null;
    if (j.truthy(j.get(factory_source, "environment_id") orelse .null)) {
        if (j.truthy(j.get(credentials, "aws_security_credentials_supplier") orelse .null)) return executableFailure(operation.errorMessage, error.InvalidGoogleExternalCredentialSource, "Only one of credential source or AWS security credentials supplier can be specified.");
        const environment = j.get(source, "environment_id") orelse .null;
        const environment_text = j.text(environment) orelse return executableFailure(operation.errorMessage, error.InvalidGoogleExternalCredentialSource, "this.environmentId?.match is not a function");
        var regex = try @import("../../regex/regexp.zig").Regex.init(a, operation.io, "^(aws)(\\d+)$", .{});
        defer regex.deinit();
        var found = (try regex.exec(a, environment_text, 0)) orelse return executableFailure(operation.errorMessage, error.InvalidGoogleExternalCredentialSource, "No valid AWS \"credential_source\" provided");
        defer found.deinit();
        if (!j.truthy(j.get(source, "regional_cred_verification_url") orelse .null)) return executableFailure(operation.errorMessage, error.InvalidGoogleExternalCredentialSource, "No valid AWS \"credential_source\" provided");
        const version = (try found.text(a, 2)).?;
        if ((std.fmt.parseFloat(f64, version) catch std.math.inf(f64)) != 1) return executableFailure(operation.errorMessage, error.UnsupportedGoogleAwsEnvironment, try std.fmt.allocPrint(operation.allocator, "aws version \"{s}\" is not supported in the current build.", .{version}));
    } else if (j.truthy(j.get(factory_source, "executable") orelse .null)) {
        const executable = j.get(factory_source, "executable").?;
        const command = j.get(executable, "command") orelse .null;
        if (!j.truthy(command)) return executableFailure(operation.errorMessage, error.InvalidGoogleExternalCredentialSource, "No valid Pluggable Auth \"credential_source\" provided.");
        const timeout = j.get(executable, "timeout_millis") orelse j.int(30000);
        const numeric = try executableNumber(a, timeout);
        if (numeric < 5000 or numeric > 120000) return executableFailure(operation.errorMessage, error.InvalidGoogleExecutableTimeout, "Timeout must be between 5000 and 120000 milliseconds.");
        if (command != .string) return executableFailure(operation.errorMessage, error.InvalidGoogleCredentialCommand, "command.match is not a function");
        if (!try @import("../../regex/regexp.zig").matches(a, operation.io, "(?:[^\\s\"]+|\"[^\"]*\")+", command.string, .{})) return executableFailure(operation.errorMessage, error.InvalidGoogleCredentialCommand, try std.fmt.allocPrint(operation.allocator, "Provided command: \"{s}\" could not be parsed.", .{command.string}));
        if (!j.truthy(timeout)) return executableFailure(operation.errorMessage, error.InvalidGoogleExecutableTimeout, "No timeoutMillis provided.");
    } else {
        const supplier = j.truthy(j.get(credentials, "subject_token_supplier") orelse .null);
        if (j.truthy(source) and supplier) return executableFailure(operation.errorMessage, error.InvalidGoogleExternalAccount, "Only one of credential source or subject token supplier can be specified.");
        if (supplier) return;
        if (!j.truthy(source)) return executableFailure(operation.errorMessage, error.InvalidGoogleExternalAccount, "A credential source or subject token supplier must be specified.");
        const format = j.get(source, "format") orelse .null;
        const raw_kind = j.get(format, "type") orelse .null;
        const kind = if (j.truthy(raw_kind)) raw_kind else j.str("text");
        if (kind != .string or (!std.mem.eql(u8, kind.string, "text") and !std.mem.eql(u8, kind.string, "json"))) return executableFailure(operation.errorMessage, error.InvalidGoogleSubjectTokenFormat, try std.fmt.allocPrint(operation.allocator, "Invalid credential_source format \"{s}\"", .{try diagnostics.formatThrownValue(a, .{ .value = kind })}));
        if (std.mem.eql(u8, kind.string, "json") and !j.truthy(j.get(format, "subject_token_field_name") orelse .null)) return executableFailure(operation.errorMessage, error.InvalidGoogleSubjectTokenFormat, "Missing subject_token_field_name for JSON credential_source format");
        const count = @as(u8, @intFromBool(j.truthy(j.get(source, "file") orelse .null))) + @as(u8, @intFromBool(j.truthy(j.get(source, "url") orelse .null))) + @as(u8, @intFromBool(j.truthy(j.get(source, "certificate") orelse .null)));
        if (count != 1) return executableFailure(operation.errorMessage, error.InvalidGoogleExternalCredentialSource, "No valid Identity Pool \"credential_source\" provided, must be either file, url, or certificate.");
        if (j.get(source, "certificate")) |certificate| if (j.truthy(certificate)) try certificates.validate(operation, certificate);
    }
}
fn configureCredentials(operation: m.OperationContext, credentials: Value) anyerror!CredentialConfiguration {
    if (credentials == .null) {
        if (operation.errorMessage) |slot| slot.* = "Cannot read properties of null (reading 'type')";
        return error.InvalidGoogleCredentials;
    }
    const kind = j.field(credentials, "type") orelse @as([]const u8, "");
    var configured = CredentialConfiguration{ .credentials = credentials, .universe = j.str("googleapis.com") };
    configured.refreshToken = j.get(credentials, "refresh_token");
    errdefer configured.deinit(operation.allocator);
    if (std.mem.eql(u8, kind, "external_account") or std.mem.eql(u8, kind, "external_account_authorized_user") or std.mem.eql(u8, kind, "impersonated_service_account")) {
        configured.universe = j.getNonNull(credentials, "universe_domain") orelse j.getNonNull(credentials, "universeDomain") orelse configured.universe;
    } else if (j.get(credentials, "universe_domain")) |universe| {
        if (j.truthy(universe)) configured.universe = universe;
    }
    if (std.mem.eql(u8, kind, "external_account") or std.mem.eql(u8, kind, "external_account_authorized_user")) {
        const external = std.mem.eql(u8, kind, "external_account");
        const raw_url = j.getNonNull(credentials, "token_url") orelse (if (external) j.getNonNull(credentials, "tokenUrl") else null);
        if (raw_url) |value| configured.tokenEndpoint = value else {
            const domain = try @import("../../utils/diagnostics.zig").formatThrownValue(operation.allocator, .{ .value = configured.universe });
            defer operation.allocator.free(domain);
            configured.ownedTokenEndpoint = try std.fmt.allocPrint(operation.allocator, "https://sts.{s}/v1/{s}", .{ domain, if (external) "token" else "oauthtoken" });
            configured.tokenEndpoint = j.str(configured.ownedTokenEndpoint.?);
        }
    }
    if (std.mem.eql(u8, kind, "authorized_user")) {
        _ = try credentialValue(operation, credentials, "client_id");
        _ = try credentialValue(operation, credentials, "client_secret");
        _ = try credentialValue(operation, credentials, "refresh_token");
    } else if (std.mem.eql(u8, kind, "impersonated_service_account")) {
        const source_credentials = try credentialValue(operation, credentials, "source_credentials");
        const raw_url = try credentialValue(operation, credentials, "service_account_impersonation_url");
        const source = try operation.allocator.create(CredentialConfiguration);
        source.* = .{ .credentials = .null, .universe = .null };
        configured.source = source;
        source.* = try configureCredentials(operation, source_credentials);
        const diagnostics = @import("../../utils/diagnostics.zig");
        const raw_length: ?Value = switch (raw_url) {
            .string => j.num(@floatFromInt(try @import("../../utils/unicode.zig").utf16Length(raw_url.string))),
            .array => j.num(@floatFromInt(raw_url.array.items.len)),
            else => j.get(raw_url, "length"),
        };
        if (try @import("../../utils/numbers.zig").coerceValue(operation.allocator, raw_length) > 256) {
            if (operation.errorMessage) |slot| slot.* = try std.fmt.allocPrint(operation.allocator, "Target principal is too long: {s}", .{try diagnostics.formatThrownValue(operation.allocator, .{ .value = raw_url })});
            return error.GoogleImpersonationUrlTooLong;
        }
        const url = try diagnostics.formatThrownValue(operation.allocator, .{ .value = raw_url });
        defer operation.allocator.free(url);
        var regex = try @import("../../regex/regexp.zig").Regex.init(operation.allocator, operation.io, "(?<target>[^/]+):(generateAccessToken|generateIdToken)$", .{});
        defer regex.deinit();
        var found = (try regex.exec(operation.allocator, url, 0)) orelse {
            if (operation.errorMessage) |slot| slot.* = try std.fmt.allocPrint(operation.allocator, "Cannot extract target principal from {s}", .{url});
            return error.InvalidGoogleImpersonationTarget;
        };
        defer found.deinit();
        const target = (try found.text(operation.allocator, 1)).?;
        configured.targetPrincipal = target;
        const explicit = j.getNonNull(credentials, "universe_domain") orelse j.getNonNull(credentials, "universeDomain") orelse .null;
        if (!j.truthy(explicit)) configured.universe = source.universe else if (!sameUniverse(source.universe, configured.universe)) {
            const source_domain = try diagnostics.formatThrownValue(operation.allocator, .{ .value = source.universe });
            const target_domain = try diagnostics.formatThrownValue(operation.allocator, .{ .value = configured.universe });
            if (operation.errorMessage) |slot| slot.* = try std.fmt.allocPrint(operation.allocator, "Universe domain {s} in source credentials does not match {s} universe domain set for impersonated credentials.", .{ source_domain, target_domain });
            return error.GoogleImpersonationUniverseMismatch;
        }
        configured.impersonationEndpoint = if (j.getNonNull(credentials, "endpoint")) |value| value else endpoint: {
            const domain = try diagnostics.formatThrownValue(operation.allocator, .{ .value = configured.universe });
            defer operation.allocator.free(domain);
            configured.ownedEndpoint = try std.fmt.allocPrint(operation.allocator, "https://iamcredentials.{s}", .{domain});
            break :endpoint j.str(configured.ownedEndpoint.?);
        };
    } else if (std.mem.eql(u8, kind, "external_account")) {
        try configureExternal(operation, credentials);
    } else if (!std.mem.eql(u8, kind, "external_account_authorized_user")) {
        if (!j.truthy(credentials)) return executableFailure(operation.errorMessage, error.InvalidGoogleCredentials, "Must pass in a JSON object containing the service account auth settings.");
        _ = try credentialValue(operation, credentials, "client_email");
        _ = try credentialValue(operation, credentials, "private_key");
    }
    return configured;
}
fn fromCredentials(operation: m.OperationContext, options: t.StreamOptions, credentials: Value) anyerror!Token {
    var configured = try configureCredentials(operation, credentials);
    defer configured.deinit(operation.allocator);
    return acquireCredentials(operation, options, &configured, .headers);
}
fn acquireCredentials(operation: m.OperationContext, options: t.StreamOptions, configured: *CredentialConfiguration, purpose: enum { headers, access_token }) anyerror!Token {
    return acquireConfigured(operation, options, configured, if (purpose == .headers) .headers else .access_token) catch |err| {
        if (configured.failureName == null) configured.failureName = if (configured.failure) |failure| failure.name else exceptionName(err);
        return primitiveFailure(operation, err, false);
    };
}
fn acquireConfigured(operation: m.OperationContext, options: t.StreamOptions, configured: *CredentialConfiguration, purpose: enum { headers, access_token }) anyerror!Token {
    const kind = j.field(configured.credentials, "type") orelse @as([]const u8, "");
    const external = std.mem.eql(u8, kind, "external_account") or std.mem.eql(u8, kind, "external_account_authorized_user");
    const jwt = !external and !std.mem.eql(u8, kind, "authorized_user") and !std.mem.eql(u8, kind, "impersonated_service_account");
    const self_signed = jwt and purpose == .headers and !sameUniverse(configured.universe, j.str("googleapis.com"));
    const cache = if (self_signed) &configured.selfSigned else &configured.token;
    const previous_type = if (cache.*) |token| token.tokenType else null;
    if (cache.*) |token| {
        if (external or token.accessTruthy) {
            var expiring = false;
            if (hasExpiry(token)) {
                const now = @as(f64, @floatFromInt(std.Io.Clock.real.now(operation.io).toMilliseconds()));
                var threshold: f64 = 300000;
                const option = j.get(configured.credentials, "eagerRefreshThresholdMillis");
                if (!jwt and !std.mem.eql(u8, kind, "authorized_user")) if (option) |value| {
                    if (std.mem.eql(u8, kind, "external_account_authorized_user")) {
                        if (j.number(value)) |numeric| threshold = numeric;
                    } else if (j.truthy(value)) threshold = try @import("../../utils/numbers.zig").coerceValue(operation.allocator, value);
                };
                var deadline = now + threshold;
                if (!external and !jwt and !std.mem.eql(u8, kind, "authorized_user")) if (option) |raw| {
                    if (j.truthy(raw) and (raw == .string or raw == .array or raw == .object)) {
                        const diagnostics = @import("../../utils/diagnostics.zig");
                        const joined = try std.fmt.allocPrint(operation.allocator, "{s}{s}", .{ try @import("../../utils/numbers.zig").toString(operation.allocator, now), try diagnostics.formatThrownValue(operation.allocator, .{ .value = raw }) });
                        deadline = try @import("../../utils/numbers.zig").coerceValue(operation.allocator, j.str(joined));
                    }
                };
                const expiry = if (token.expiryDate) |raw| try @import("../../utils/numbers.zig").coerceValue(operation.allocator, raw) else token.expires.?;
                expiring = if (external) now >= expiry - threshold else expiry <= deadline;
            }
            if (!expiring) {
                var result = token;
                if (purpose == .headers and !external and !self_signed) result.scheme = try oauthScheme(operation.allocator, token.tokenType);
                return result;
            }
        }
    }
    if (purpose == .headers) if (oauthApiKey(configured.credentials)) |key| return .{ .access = null, .accessTruthy = false, .apiKey = key, .authClientHeaders = authHeadersEnabled(configured.credentials) };
    const fresh = try refreshCredentials(operation, options, configured, purpose == .headers);
    cache.* = fresh;
    var result = fresh;
    // OAuth2Client's refresh path builds this request's header using the
    // previous credentials' token_type. Newly returned token_type is retained
    // for subsequent cached metadata requests.
    if (purpose == .headers and !external) result.scheme = try oauthScheme(operation.allocator, previous_type);
    return result;
}
fn oauthScheme(a: std.mem.Allocator, raw: ?Value) ![]const u8 {
    return if (raw) |value| (if (j.truthy(value)) try @import("../../utils/diagnostics.zig").formatThrownValue(a, .{ .value = value }) else "Bearer") else "Bearer";
}
fn hasExpiry(token: Token) bool {
    if (token.expiryDate) |raw| return j.truthy(raw);
    return if (token.expires) |expiry| expiry != 0 and !std.math.isNan(expiry) else false;
}
fn currentMilliseconds(io: std.Io) f64 {
    return @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds());
}
fn exceptionName(err: anyerror) []const u8 {
    return switch (err) {
        error.CannotConvertObjectToPrimitive, error.InvalidGoogleTokenResponse, error.InvalidGoogleAwsMetadata, error.InvalidGoogleAwsVerificationUrl, error.InvalidGoogleSubjectTokenSupplier, error.InvalidGoogleSubjectTokenHeaders, error.InvalidGoogleSigningKeyType, error.UnsupportedGoogleSigningKeyOptions, error.InvalidGoogleJwk, error.InvalidHeader, error.InvalidHeaderByteString => "TypeError",
        error.GoogleImpersonationUrlTooLong, error.InvalidGoogleImpersonationTarget, error.GoogleImpersonationUniverseMismatch => "RangeError",
        error.InvalidGoogleCertificateConfiguration => "InvalidConfigurationError",
        error.GoogleCertificateSourceUnavailable => "CertificateSourceUnavailableError",
        else => "Error",
    };
}
fn forwardSourceFailure(operation: m.OperationContext, err: anyerror, target: *CredentialConfiguration, source: *CredentialConfiguration) anyerror {
    if (err == error.Canceled or err == error.OutOfMemory) return err;
    const copied = if (source.failure) |failure| failure.clone(operation.allocator) catch |clone_error| return clone_error else null;
    if (target.failure) |*previous| previous.deinit();
    target.failure = copied;
    target.failureName = if (target.failure) |failure| failure.name else source.failureName orelse exceptionName(err);
    return err;
}
fn wrapImpersonationFailure(operation: m.OperationContext, err: anyerror, configured: *CredentialConfiguration) anyerror {
    if (err == error.Canceled or err == error.OutOfMemory) return err;
    const slot = operation.errorMessage orelse return err;
    const a = operation.allocator;
    const diagnostics = @import("../../utils/diagnostics.zig");
    if (configured.failure) |failure| if (failure.gaxios) {
        const detail = j.get(failure.body, "error") orelse .null;
        const status = j.get(detail, "status") orelse .null;
        const message = j.get(detail, "message") orelse .null;
        if (j.truthy(status) and j.truthy(message)) {
            const status_text = diagnostics.formatThrownValue(a, .{ .value = status }) catch |failure_error| return replacementFailure(operation, failure_error, configured);
            defer a.free(status_text);
            const message_text = diagnostics.formatThrownValue(a, .{ .value = message }) catch |failure_error| return replacementFailure(operation, failure_error, configured);
            defer a.free(message_text);
            slot.* = std.fmt.allocPrint(a, "{s}: unable to impersonate: {s}", .{ status_text, message_text }) catch |failure_error| return failure_error;
            return err;
        }
    };
    const message = slot.* orelse @errorName(err);
    slot.* = std.fmt.allocPrint(a, "unable to impersonate: {s}{s}{s}", .{ configured.failureName orelse exceptionName(err), if (message.len > 0) ": " else "", message }) catch |failure_error| return failure_error;
    return err;
}
fn replacementFailure(operation: m.OperationContext, err: anyerror, configured: *CredentialConfiguration) anyerror {
    if (configured.failure) |*failure| {
        failure.gaxios = false;
        failure.setName(exceptionName(err)) catch |name_error| return name_error;
    }
    configured.failureName = exceptionName(err);
    if (err == error.CannotConvertObjectToPrimitive) if (operation.errorMessage) |slot| {
        slot.* = null;
    };
    return primitiveFailure(operation, err, false);
}
fn refreshCredentials(operation: m.OperationContext, options: t.StreamOptions, configured: *CredentialConfiguration, for_headers: bool) anyerror!Token {
    if (configured.failure) |*failure| failure.deinit();
    configured.failure = null;
    configured.failureName = null;
    return refreshConfigured(operation, options, configured, for_headers) catch |err| {
        if (configured.failureName == null) configured.failureName = if (configured.failure) |failure| failure.name else exceptionName(err);
        const normalized = primitiveFailure(operation, err, false);
        if (std.mem.eql(u8, j.field(configured.credentials, "type") orelse "", "impersonated_service_account")) return wrapImpersonationFailure(operation, normalized, configured);
        return normalized;
    };
}
fn refreshConfigured(operation: m.OperationContext, options: t.StreamOptions, configured: *CredentialConfiguration, for_headers: bool) anyerror!Token {
    const credentials = configured.credentials;
    const kind = j.field(credentials, "type") orelse @as([]const u8, "");
    if (std.mem.eql(u8, kind, "service_account")) return serviceAccount(operation, configured, if (for_headers) .headers else .access_token);
    if (std.mem.eql(u8, kind, "authorized_user")) return authorizedUser(operation, configured, for_headers);
    if (std.mem.eql(u8, kind, "external_account_authorized_user")) return externalAuthorizedUser(operation, configured);
    if (std.mem.eql(u8, kind, "external_account")) return externalAccount(operation, options, configured);
    if (std.mem.eql(u8, kind, "impersonated_service_account")) {
        const source_config = configured.source.?;
        const source = acquireCredentials(operation, options, source_config, .access_token) catch |err| return forwardSourceFailure(operation, err, configured, source_config);
        const source_kind = j.field(source_config.credentials, "type") orelse @as([]const u8, "");
        const source_external = std.mem.eql(u8, source_kind, "external_account") or std.mem.eql(u8, source_kind, "external_account_authorized_user");
        if (!source_external and !source.accessTruthy) {
            configured.failureName = "Error";
            return executableFailure(operation.errorMessage, error.InvalidGoogleTokenResponse, "Could not refresh access token.");
        }
        const endpoint_text = @import("../../utils/diagnostics.zig").formatThrownValue(operation.allocator, .{ .value = configured.impersonationEndpoint }) catch |err| return primitiveFailure(operation, err, false);
        defer operation.allocator.free(endpoint_text);
        const url = try std.fmt.allocPrint(operation.allocator, "{s}/v1/projects/-/serviceAccounts/{s}:generateAccessToken", .{ endpoint_text, configured.targetPrincipal.? });
        defer operation.allocator.free(url);
        return impersonate(operation, j.str(url), source, credentials, null, configured, .{ .options = options, .configuration = source_config });
    }
    return serviceAccount(operation, configured, if (for_headers) .headers else .access_token);
}
fn prepareDefaultProject(operation: m.OperationContext) !void {
    // GoogleAuth prepares an ADC client by resolving an optional project ID.
    // The environment/helper/metadata paths remain available without opening a
    // credential file. Each temporary result dies before token acquisition.
    var temporary = std.heap.ArenaAllocator.init(operation.allocator);
    defer temporary.deinit();
    var local = operation;
    local.allocator = temporary.allocator();
    local.errorMessage = null;
    for ([_][]const u8{ "GCLOUD_PROJECT", "GOOGLE_CLOUD_PROJECT", "gcloud_project", "google_cloud_project" }) |name| if (try ambient(local, name)) |value| {
        if (value.len > 0) return;
    };
    if (operation.authContext) |context| if (context.credentialCommandFn) |execute| {
        const result = execute(context.context, local.allocator, operation.io, .{ .command = "gcloud config config-helper --format json", .timeoutMs = 0, .kind = .shell, .maxOutputBytes = 1024 * 1024 }) catch |err| result: {
            if (err == error.Canceled or err == error.OutOfMemory) return err;
            break :result @as(Value, .null);
        };
        const core = j.get(j.get(j.get(result, "configuration") orelse .null, "properties") orelse .null, "core") orelse .null;
        if (j.truthy(j.get(core, "project") orelse .null)) return;
    };
    _ = @import("metadata.zig").project(local) catch |err| {
        if (err == error.Canceled or err == error.OutOfMemory) return err;
        return;
    };
}
pub fn resolve(operation: m.OperationContext, options: t.StreamOptions) !Token {
    const a = operation.allocator;
    if (operation.authContext) |context| if (context.inputs.googleCredentials) |credentials| {
        var token = try fromCredentials(operation, options, credentials);
        if (token.quotaProject) |project| token.quotaProject = try a.dupe(u8, project);
        return token;
    };
    if (nonempty(try aws.env(operation, options, "GOOGLE_CREDENTIALS_JSON"))) |text| {
        const credentials = try j.parseFromSliceLeaky(Value, a, text, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always });
        return fromCredentials(operation, options, credentials);
    }
    const metadata = @import("metadata.zig");
    if (!try metadata.residency(operation) and !try metadata.available(operation)) {
        if (operation.errorMessage) |slot| slot.* = "Could not load the default credentials. Browse to https://cloud.google.com/docs/authentication/getting-started for more information.";
        return error.GoogleApplicationDefaultCredentialsUnavailable;
    }
    try prepareDefaultProject(operation);
    const response = try metadata.token(operation);
    return fromTokenResponse(operation, response.body, null, .compute);
}
fn nonempty(value: ?[]const u8) ?[]const u8 {
    return if (value) |text| if (text.len > 0) text else null else null;
}
fn primitiveFailure(operation: m.OperationContext, err: anyerror, impersonated: bool) anyerror {
    if (err == error.CannotConvertObjectToPrimitive) {
        if (!impersonated) if (operation.errorMessage) |slot| if (slot.*) |message| {
            if (std.mem.startsWith(u8, message, "unable to impersonate:")) return err;
        };
        return executableFailure(operation.errorMessage, err, if (impersonated) "unable to impersonate: TypeError: Cannot convert object to primitive value" else "Cannot convert object to primitive value");
    }
    return err;
}
fn authHeadersEnabled(credentials: Value) bool {
    const option = j.get(credentials, "useAuthRequestParameters") orelse .null;
    return option != .bool or option.bool;
}
fn oauthApiKey(credentials: Value) ?Value {
    // JWT/UserRefresh are constructed from separate options before fromJSON;
    // Impersonated receives the JSON spread as its OAuth constructor options.
    if (!std.mem.eql(u8, j.field(credentials, "type") orelse "", "impersonated_service_account")) return null;
    const value = j.get(credentials, "apiKey") orelse return null;
    return if (j.truthy(value)) value else null;
}
fn accessText(operation: m.OperationContext, token: Token, impersonated: bool) ![]const u8 {
    if (token.accessFromMetadata) if (token.access) |value| if (metadataObject(value)) return primitiveFailure(operation, error.CannotConvertObjectToPrimitive, impersonated);
    return @import("../../utils/diagnostics.zig").formatThrownValue(operation.allocator, if (token.access) |value| .{ .value = value } else .undefined_value) catch |err| {
        if (err == error.CannotConvertObjectToPrimitive) return executableFailure(operation.errorMessage, err, if (impersonated) "unable to impersonate: TypeError: Cannot convert object to primitive value" else "Cannot convert object to primitive value");
        return err;
    };
}
fn quotaHeader(a: std.mem.Allocator, token: Token) !?[]const u8 {
    if (token.quotaValue) |value| return if (j.truthy(value)) try @import("../../utils/diagnostics.zig").formatThrownValue(a, .{ .value = value }) else null;
    return nonempty(token.quotaProject);
}
pub fn requestHeaders(operation: m.OperationContext, _: t.Model, options: t.StreamOptions, original: t.ProviderHeaders) !t.ProviderHeaders {
    return requestHeadersInner(operation, options, original) catch |err| return primitiveFailure(operation, err, false);
}
fn requestHeadersInner(operation: m.OperationContext, options: t.StreamOptions, original: t.ProviderHeaders) !t.ProviderHeaders {
    if (try @import("../../api/google_vertex.zig").hasApiKey(operation.allocator, options)) return original;
    const token = resolve(operation, options) catch |err| return primitiveFailure(operation, err, false);
    var auth: std.ArrayList(t.ProviderHeader) = .empty;
    if (token.apiKey) |key| {
        try headers.appendFetch(operation.allocator, &auth, "x-goog-api-key", try @import("../../utils/diagnostics.zig").formatThrownValue(operation.allocator, .{ .value = key }), operation.errorMessage);
    } else {
        try headers.appendFetch(operation.allocator, &auth, "authorization", try std.fmt.allocPrint(operation.allocator, "{s} {s}", .{ token.scheme, try accessText(operation, token, false) }), operation.errorMessage);
        if (nonempty(try ambient(operation, "GOOGLE_CLOUD_QUOTA_PROJECT")) orelse try quotaHeader(operation.allocator, token)) |project| try headers.appendFetch(operation.allocator, &auth, "x-goog-user-project", project, operation.errorMessage);
    }
    return headers.merge(operation.allocator, &.{ auth.items, original });
}
fn metadataText(operation: m.OperationContext, url: []const u8, extra: t.ProviderHeaders, configured: *CredentialConfiguration) ![]const u8 {
    const response = try http.requestWithPolicy(operation.allocator, operation.io, operation.transport, .GET, url, null, null, extra, diagnosticPolicy(operation, .{ .responseType = .text, .maxRetries = 0, .noResponseRetries = 0, .authClientHeaders = authHeadersEnabled(configured.credentials), .failure = &configured.failure, .failureName = &configured.failureName }));
    if (!response.ok()) return reject(operation, response, error.GoogleAwsMetadataFailed);
    return response.raw;
}
fn awsMetadataHeaders(operation: m.OperationContext, source: Value, configured: *CredentialConfiguration) !t.ProviderHeaders {
    const raw_url = j.get(source, "imdsv2_session_token_url") orelse .null;
    if (!j.truthy(raw_url)) return &.{};
    const url = try @import("../../utils/diagnostics.zig").formatThrownValue(operation.allocator, .{ .value = raw_url });
    const response = try http.requestWithPolicy(operation.allocator, operation.io, operation.transport, .PUT, url, null, null, &.{.{ .name = "x-aws-ec2-metadata-token-ttl-seconds", .value = "300" }}, diagnosticPolicy(operation, .{ .responseType = .text, .maxRetries = 0, .noResponseRetries = 0, .authClientHeaders = authHeadersEnabled(configured.credentials), .failure = &configured.failure, .failureName = &configured.failureName }));
    if (!response.ok()) return reject(operation, response, error.GoogleAwsMetadataFailed);
    var result: std.ArrayList(t.ProviderHeader) = .empty;
    try headers.appendFetch(operation.allocator, &result, "x-aws-ec2-metadata-token", response.raw, operation.errorMessage);
    return result.toOwnedSlice(operation.allocator);
}
/// Google AWS federation uses its pinned raw-URL canonicalization profile.
/// Only the typed STS verification request is serialized as the subject token.
fn awsAccessText(ctx: ?*anyopaque, a: std.mem.Allocator) ![]const u8 {
    const raw: *const ?Value = @ptrCast(@alignCast(ctx.?));
    return @import("../../utils/diagnostics.zig").formatThrownValue(a, if (raw.*) |value| .{ .value = value } else .undefined_value);
}
fn awsSubjectToken(operation: m.OperationContext, options: t.StreamOptions, credentials: Value, configured: *CredentialConfiguration) ![]const u8 {
    _ = options;
    const a = operation.allocator;
    const io = operation.io;
    const source = j.get(credentials, "credential_source") orelse return error.InvalidGoogleExternalAccount;
    const cached_region = &configured.awsRegion;
    const env_access = nonempty(try ambient(operation, "AWS_ACCESS_KEY_ID"));
    const env_secret = nonempty(try ambient(operation, "AWS_SECRET_ACCESS_KEY"));
    var region_value = cached_region.* orelse nonempty(try ambient(operation, "AWS_REGION")) orelse nonempty(try ambient(operation, "AWS_DEFAULT_REGION"));
    if (region_value == null) {
        const metadata_headers = try awsMetadataHeaders(operation, source, configured);
        const raw_region_url = j.get(source, "region_url") orelse .null;
        if (!j.truthy(raw_region_url)) return executableFailure(operation.errorMessage, error.GoogleAwsRegionUnavailable, "Unable to determine AWS region due to missing \"options.credential_source.region_url\"");
        const region_url = try @import("../../utils/diagnostics.zig").formatThrownValue(a, .{ .value = raw_region_url });
        const zone = try metadataText(operation, region_url, metadata_headers, configured);
        const units = try std.unicode.wtf8ToWtf16LeAlloc(a, zone);
        defer a.free(units);
        // The pinned supplier removes exactly one UTF-16 code unit, without
        // trimming or checking whether it is an availability-zone letter.
        region_value = try std.unicode.wtf16LeToWtf8Alloc(a, units[0 .. units.len - @intFromBool(units.len != 0)]);
    }
    const region_name = region_value.?;
    cached_region.* = region_name;
    const Security = struct { access: ?Value, secret: ?Value, token: ?Value };
    const security = if (env_access != null and env_secret != null) env: {
        const token = try ambient(operation, "AWS_SESSION_TOKEN");
        break :env Security{ .access = j.str(env_access.?), .secret = j.str(env_secret.?), .token = if (token) |text| j.str(text) else null };
    } else blk: {
        const metadata_headers = try awsMetadataHeaders(operation, source, configured);
        const raw_base = j.get(source, "url") orelse .null;
        if (!j.truthy(raw_base)) return executableFailure(operation.errorMessage, error.GoogleAwsMetadataFailed, "Unable to determine AWS role name due to missing \"options.credential_source.url\"");
        const base = try @import("../../utils/diagnostics.zig").formatThrownValue(a, .{ .value = raw_base });
        const role = try metadataText(operation, base, metadata_headers, configured);
        const response = try http.requestWithPolicy(a, io, operation.transport, .GET, try std.fmt.allocPrint(a, "{s}/{s}", .{ base, role }), null, null, metadata_headers, diagnosticPolicy(operation, .{ .maxRetries = 0, .noResponseRetries = 0, .authClientHeaders = authHeadersEnabled(credentials), .failure = &configured.failure, .failureName = &configured.failureName }));
        if (!response.ok()) return reject(operation, response, error.GoogleAwsMetadataFailed);
        const value = response.body;
        if (value == .null) return executableFailure(operation.errorMessage, error.InvalidGoogleAwsMetadata, "Cannot read properties of null (reading 'AccessKeyId')");
        break :blk Security{ .access = j.get(value, "AccessKeyId"), .secret = j.get(value, "SecretAccessKey"), .token = j.get(value, "Token") };
    };
    const template = j.field(source, "regional_cred_verification_url") orelse return executableFailure(operation.errorMessage, error.InvalidGoogleAwsVerificationUrl, "this.regionalCredVerificationUrl.replace is not a function");
    const target = if (std.mem.find(u8, template, "{region}")) |index| try std.fmt.allocPrint(a, "{s}{s}{s}", .{ template[0..index], region_name, template[index + "{region}".len ..] }) else template;
    const endpoint = uri.parseHttpUrl(a, target) catch |err| {
        if (err == error.OutOfMemory or err == error.Canceled) return err;
        configured.failureName = "TypeError";
        return executableFailure(operation.errorMessage, err, "Invalid URL");
    };
    const signing_target = try uri.render(a, endpoint);
    const signing_date = @import("../../crypto/aws_sigv4.zig").googleDateText(a, std.Io.Clock.real.now(io).toMilliseconds()) catch |err| {
        if (err == error.TimestampOutOfRange) {
            configured.failureName = "RangeError";
            return executableFailure(operation.errorMessage, err, "Invalid time value");
        }
        return err;
    };
    defer {
        a.free(signing_date.day);
        a.free(signing_date.time);
    }
    const audience = j.field(credentials, "audience").?;
    const signing = @import("../signing.zig");
    var authority = endpoint;
    authority.user = null;
    authority.password = null;
    if (authority.port) |port| if ((std.ascii.eqlIgnoreCase(authority.scheme, "https") and port == 443) or (std.ascii.eqlIgnoreCase(authority.scheme, "http") and port == 80)) {
        authority.port = null;
    };
    var host_writer = std.Io.Writer.Allocating.init(a);
    defer host_writer.deinit();
    try authority.writeToStream(&host_writer.writer, .{ .authority = true });
    var input_headers: std.ArrayList(signing.Pair) = .empty;
    try input_headers.append(a, .{ .name = signing.view("host"), .value = signing.view(host_writer.written()) });
    var host_labels = std.mem.splitScalar(u8, host_writer.written(), '.');
    var token_headers: std.ArrayList(t.ProviderHeader) = .empty;
    if (security.token) |raw| if (j.truthy(raw)) {
        const token = try @import("../../utils/diagnostics.zig").formatThrownValue(a, .{ .value = raw });
        defer a.free(token);
        try headers.appendFetch(a, &token_headers, "x-amz-security-token", token, operation.errorMessage);
    };
    for (token_headers.items) |header| try input_headers.append(a, .{ .name = signing.view(header.name), .value = signing.view(header.value.?) });
    const raw_secret = try @import("../../utils/diagnostics.zig").formatThrownValue(a, if (security.secret) |raw| .{ .value = raw } else .undefined_value);
    defer if (security.secret != null) {
        std.crypto.secureZero(u8, @constCast(raw_secret));
        a.free(raw_secret);
    };
    const secret_units = try std.unicode.wtf8ToWtf16LeAlloc(a, raw_secret);
    defer {
        std.crypto.secureZero(u16, secret_units);
        a.free(secret_units);
    }
    const secret = try std.fmt.allocPrint(a, "{f}", .{std.unicode.fmtUtf16Le(secret_units)});
    defer {
        std.crypto.secureZero(u8, @constCast(secret));
        a.free(secret);
    }
    const raw_access = security.access;
    var output = try signing.awsGoogleWithDateAndAccessKey(a, io, .{ .method = signing.view("POST"), .uri = signing.view(signing_target), .body = signing.view(""), .headers = signing.Pairs.fromSlice(input_headers.items), .region = signing.view(region_name), .service = signing.view(host_labels.first()), .access_key = signing.view(""), .secret_key = signing.view(secret), .session_token = signing.view(""), .timestamp_seconds = 0 }, signing_date, .{ .context = @constCast(&raw_access), .resolve = awsAccessText });
    defer output.deinit();
    // The pinned AwsClient adds this audience after signing, then iterates
    // Fetch Headers in lowercase name order when serializing the subject.
    var signer_headers: std.ArrayList(t.ProviderHeader) = .empty;
    // The low-level signer returns generated fields; the SDK's signed request
    // also carries its original host and any present session-token header.
    for (input_headers.items) |header| try headers.appendFetch(a, &signer_headers, try header.name.slice(), try header.value.slice(), operation.errorMessage);
    for (try output.headers.slice()) |header| try headers.appendFetch(a, &signer_headers, try header.name.slice(), try header.value.slice(), operation.errorMessage);
    var subject_headers: std.ArrayList(t.ProviderHeader) = .empty;
    try headers.appendFetch(a, &subject_headers, "x-goog-cloud-target-resource", audience, operation.errorMessage);
    try subject_headers.appendSlice(a, signer_headers.items);
    const header_record = try headers.headersToRecord(a, subject_headers.items);
    var signed_headers = j.array(a);
    for (try j.objectEntries(a, header_record.object)) |header| try signed_headers.array.append(try j.from(a, .{ .key = header.key, .value = header.node.* }));
    return uri.encodeJsComponent(a, try @import("../../json/value.zig").stringify(a, .{ .url = target, .method = "POST", .headers = signed_headers }, .{}));
}
const ExecutableResponse = struct { value: Value, success: bool, expiration: ?Value, token: ?Value };
fn executableFailure(slot: ?*?[]const u8, err: anyerror, message: []const u8) anyerror {
    if (slot) |value| value.* = message;
    return err;
}
fn executableResponse(value: Value, error_message: ?*?[]const u8) !ExecutableResponse {
    if (value == .null) return executableFailure(error_message, error.InvalidGoogleExecutableResponse, "Cannot read properties of null (reading 'version')");
    if (!j.truthy(j.get(value, "version") orelse .null)) return executableFailure(error_message, error.InvalidGoogleExecutableVersion, "Executable response must contain a 'version' field.");
    const success = j.truthy(j.get(value, "success") orelse return executableFailure(error_message, error.InvalidGoogleExecutableSuccess, "Executable response must contain a 'success' field."));
    var token: ?Value = null;
    if (success) {
        const kind = j.field(value, "token_type") orelse @as([]const u8, "");
        const saml = std.mem.eql(u8, kind, "urn:ietf:params:oauth:token-type:saml2");
        if (!saml and !std.mem.eql(u8, kind, "urn:ietf:params:oauth:token-type:jwt") and !std.mem.eql(u8, kind, "urn:ietf:params:oauth:token-type:id_token")) return executableFailure(error_message, error.UnsupportedGoogleExecutableTokenType, "Executable response must contain a 'token_type' field when successful and it must be one of urn:ietf:params:oauth:token-type:id_token, urn:ietf:params:oauth:token-type:jwt, or urn:ietf:params:oauth:token-type:saml2.");
        const subject = j.get(value, if (saml) "saml_response" else "id_token") orelse .null;
        if (!j.truthy(subject)) return executableFailure(error_message, error.InvalidOAuthResponse, if (saml) "Executable response must contain a 'saml_response' field when token_type=urn:ietf:params:oauth:token-type:saml2." else "Executable response must contain a 'id_token' field when token_type=urn:ietf:params:oauth:token-type:id_token or urn:ietf:params:oauth:token-type:jwt.");
        token = subject;
    } else {
        if (!j.truthy(j.get(value, "code") orelse .null)) return executableFailure(error_message, error.InvalidGoogleExecutableErrorCode, "Executable response must contain a 'code' field when unsuccessful.");
        if (!j.truthy(j.get(value, "message") orelse .null)) return executableFailure(error_message, error.InvalidGoogleExecutableErrorMessage, "Executable response must contain a 'message' field when unsuccessful.");
    }
    return .{ .value = value, .success = success, .expiration = j.get(value, "expiration_time"), .token = token };
}
fn executableNumber(a: std.mem.Allocator, value: Value) anyerror!f64 {
    return @import("../../utils/numbers.zig").coerceValue(a, value);
}
fn executableExpired(a: std.mem.Allocator, io: std.Io, value: ExecutableResponse) !bool {
    const expiration = value.expiration orelse return false;
    const now = @round(@as(f64, @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds())) / 1000);
    return (try executableNumber(a, expiration)) < now;
}
fn executableToken(a: std.mem.Allocator, io: std.Io, response: ExecutableResponse, require_expiry: bool, error_message: ?*?[]const u8) !Value {
    if ((try executableNumber(a, j.get(response.value, "version").?)) > 1) return executableFailure(error_message, error.UnsupportedGoogleExecutableVersion, "Version of executable is not currently supported, maximum supported version is 1.");
    if (!response.success) {
        if (error_message) |slot| {
            const diagnostics = @import("../../utils/diagnostics.zig");
            const code = try diagnostics.formatThrownValue(a, .{ .value = j.get(response.value, "code").? });
            const message = try diagnostics.formatThrownValue(a, .{ .value = j.get(response.value, "message").? });
            slot.* = try std.fmt.allocPrint(a, "The executable failed with exit code: {s} and error message: {s}.", .{ code, message });
        }
        return error.GoogleCredentialExecutableFailed;
    }
    if (require_expiry and !j.truthy(response.expiration orelse .null)) return executableFailure(error_message, error.GoogleExecutableCacheExpiryMissing, "The executable response must contain the `expiration_time` field for successful responses when an output_file has been specified in the configuration.");
    if (try executableExpired(a, io, response)) return executableFailure(error_message, error.GoogleExecutableTokenExpired, "Executable response is expired.");
    return response.token.?;
}
fn executableSubjectToken(operation: m.OperationContext, options: t.StreamOptions, credentials: Value, executable: Value) !Value {
    _ = options;
    const a = operation.allocator;
    const io = operation.io;
    // The SDK constructs and validates its handler before token retrieval
    // checks the executable opt-in or reads an existing output file.
    const command_value = j.get(executable, "command") orelse .null;
    if (!j.truthy(command_value)) return executableFailure(operation.errorMessage, error.InvalidGoogleExternalCredentialSource, "No valid Pluggable Auth \"credential_source\" provided.");
    const timeout_value = j.get(executable, "timeout_millis") orelse j.int(30_000);
    const timeout_number = try executableNumber(a, timeout_value);
    if (timeout_number < 5000 or timeout_number > 120000) return executableFailure(operation.errorMessage, error.InvalidGoogleExecutableTimeout, "Timeout must be between 5000 and 120000 milliseconds.");
    const command = j.text(command_value) orelse return executableFailure(operation.errorMessage, error.InvalidGoogleCredentialCommand, "command.match is not a function");
    if (!j.truthy(timeout_value)) return executableFailure(operation.errorMessage, error.InvalidGoogleExecutableTimeout, "No timeoutMillis provided.");
    // Node timers truncate fractional delays and use one millisecond for NaN.
    const timeout_ms: u64 = if (std.math.isFinite(timeout_number)) @intFromFloat(timeout_number) else 1;
    const allowed = (try ambient(operation, "GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES")) orelse @as([]const u8, "");
    if (!std.mem.eql(u8, allowed, "1")) return executableFailure(operation.errorMessage, error.GoogleCredentialExecutableNotAllowed, "Pluggable Auth executables need to be explicitly allowed to run by setting the GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES environment Variable to 1.");
    const output_file = if (j.get(executable, "output_file")) |file| if (j.truthy(file)) file else null else null;
    const context = operation.authContext orelse return error.GoogleExternalCredentialCommandHostRequired;
    const execute = context.credentialCommandFn orelse return error.GoogleExternalCredentialCommandHostRequired;
    var env = t.ProviderEnv{};
    const diagnostics = @import("../../utils/diagnostics.zig");
    try env.put(a, "GOOGLE_EXTERNAL_ACCOUNT_AUDIENCE", j.field(credentials, "audience").?);
    const raw_type = j.get(credentials, "subject_token_type");
    if (raw_type) |value| try env.put(a, "GOOGLE_EXTERNAL_ACCOUNT_TOKEN_TYPE", try diagnostics.formatThrownValue(a, .{ .value = value }));
    try env.put(a, "GOOGLE_EXTERNAL_ACCOUNT_INTERACTIVE", "0");
    if (output_file) |file| try env.put(a, "GOOGLE_EXTERNAL_ACCOUNT_OUTPUT_FILE", try diagnostics.formatThrownValue(a, .{ .value = file }));
    const raw_url = j.get(credentials, "service_account_impersonation_url") orelse .null;
    if (j.truthy(raw_url)) {
        const raw_length: ?Value = switch (raw_url) {
            .string => j.num(@floatFromInt(try @import("../../utils/unicode.zig").utf16Length(raw_url.string))),
            .array => j.num(@floatFromInt(raw_url.array.items.len)),
            else => j.get(raw_url, "length"),
        };
        if (try @import("../../utils/numbers.zig").coerceValue(a, raw_length) > 256) {
            if (operation.errorMessage) |slot| slot.* = try std.fmt.allocPrint(a, "URL is too long: {s}", .{try diagnostics.formatThrownValue(a, .{ .value = raw_url })});
            return error.GoogleImpersonationUrlTooLong;
        }
        const url = try diagnostics.formatThrownValue(a, .{ .value = raw_url });
        var regex = try @import("../../regex/regexp.zig").Regex.init(a, io, "serviceAccounts/(?<email>[^:]+):generateAccessToken$", .{});
        defer regex.deinit();
        if (try regex.exec(a, url, 0)) |found| {
            var selected = found;
            defer selected.deinit();
            try env.put(a, "GOOGLE_EXTERNAL_ACCOUNT_IMPERSONATED_EMAIL", (try selected.text(a, 1)).?);
        }
    }
    const result = try execute(context.context, a, io, .{ .command = command, .timeoutMs = timeout_ms, .env = env, .unsetEnv = if (raw_type == null) &.{"GOOGLE_EXTERNAL_ACCOUNT_TOKEN_TYPE"} else &.{}, .kind = .google_executable, .errorMessage = operation.errorMessage });
    return executableToken(a, io, try executableResponse(result, operation.errorMessage), output_file != null, operation.errorMessage);
}
