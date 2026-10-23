//! Built-in signing and certificate codecs use caller-owned allocators and Io.
const std = @import("std");
const abi = @import("native_c_abi");
const keys = @import("../crypto/keys.zig");
pub const Pair = extern struct { name: abi.StringView, value: abi.StringView };
pub const Pairs = abi.BorrowedSlice(Pair);
pub const AwsInput = extern struct {
    abi_version: u32 = 1,
    struct_size: u32 = @sizeOf(AwsInput),
    method: abi.StringView,
    uri: abi.StringView,
    body: abi.StringView,
    headers: Pairs,
    region: abi.StringView,
    service: abi.StringView,
    access_key: abi.StringView,
    secret_key: abi.StringView,
    session_token: abi.StringView,
    timestamp_seconds: u64,
};
pub const GoogleInput = extern struct {
    abi_version: u32 = 1,
    struct_size: u32 = @sizeOf(GoogleInput),
    client_email: abi.StringView,
    private_key_id: abi.StringView,
    private_key: abi.StringView,
    project_id: abi.StringView,
    content: abi.StringView,
};
pub const Output = struct {
    headers: Pairs = .{ .data = null, .length = 0 },
    query: Pairs = .{ .data = null, .length = 0 },
    signature: abi.StringView = .{ .data = null, .length = 0 },
    owner: *std.heap.ArenaAllocator,
    allocator: std.mem.Allocator,
    fn init(a: std.mem.Allocator) !@This() {
        const arena = try a.create(std.heap.ArenaAllocator);
        arena.* = .init(a);
        return .{ .owner = arena, .allocator = a };
    }
    fn alloc(self: @This()) std.mem.Allocator {
        return self.owner.allocator();
    }
    pub fn deinit(self: *@This()) void {
        if (self.signature.slice()) |bytes| std.crypto.secureZero(u8, @constCast(bytes)) else |_| {}
        self.owner.deinit();
        self.allocator.destroy(self.owner);
        self.* = undefined;
    }
};
pub fn aws(a: std.mem.Allocator, io: std.Io, input: AwsInput) !Output {
    return awsWithCanonicalization(a, io, input, .smithy, null, null);
}
/// Google federation signs a WHATWG URL's existing path/query and does not
/// inject Smithy's checksum header. Its algorithm still uses native SHA/HMAC.
pub fn awsGoogle(a: std.mem.Allocator, io: std.Io, input: AwsInput) !Output {
    return awsWithCanonicalization(a, io, input, .google, null, null);
}
/// The resolver borrows its context through this synchronous signing call.
pub fn awsGoogleWithAccessKey(a: std.mem.Allocator, io: std.Io, input: AwsInput, resolver: @import("../crypto/aws_sigv4.zig").AccessKeyResolver) !Output {
    return awsWithCanonicalization(a, io, input, .google, resolver, null);
}
/// Date/access fields borrow through this call; AwsInput's C layout is intact.
pub fn awsGoogleWithDateAndAccessKey(a: std.mem.Allocator, io: std.Io, input: AwsInput, date_text: @import("../crypto/aws_sigv4.zig").DateText, resolver: @import("../crypto/aws_sigv4.zig").AccessKeyResolver) !Output {
    return awsWithCanonicalization(a, io, input, .google, resolver, date_text);
}
fn awsWithCanonicalization(a: std.mem.Allocator, io: std.Io, input: AwsInput, canonicalization: @import("../crypto/aws_sigv4.zig").Canonicalization, access_key_resolver: ?@import("../crypto/aws_sigv4.zig").AccessKeyResolver, date_text: ?@import("../crypto/aws_sigv4.zig").DateText) !Output {
    try io.checkCancel();
    var output = try Output.init(a);
    errdefer output.deinit();
    const storage = output.alloc();
    var headers: std.ArrayList(std.http.Header) = .empty;
    for (try input.headers.slice()) |pair| try headers.append(storage, .{ .name = try pair.name.slice(), .value = try pair.value.slice() });
    const token = try input.session_token.slice();
    const signed = try @import("../crypto/aws_sigv4.zig").sign(storage, io, .{
        .method = try input.method.slice(),
        .uri = try std.Uri.parse(try input.uri.slice()),
        .body = try input.body.slice(),
        .headers = headers.items,
        .region = try input.region.slice(),
        .service = try input.service.slice(),
        .accessKey = try input.access_key.slice(),
        .secretKey = try input.secret_key.slice(),
        .sessionToken = if (token.len == 0) null else token,
        .timestamp = input.timestamp_seconds,
        .canonicalization = canonicalization,
        .accessKeyResolver = access_key_resolver,
        .dateText = date_text,
    });
    const pairs = try storage.alloc(Pair, signed.headers.len);
    for (pairs, signed.headers) |*pair, header| pair.* = .{ .name = view(header.name), .value = view(try storage.dupe(u8, header.value)) };
    output.headers = Pairs.fromSlice(pairs);
    output.signature = view(signed.signature);
    return output;
}
pub const GoogleSignatureEncoding = enum { der, ieee_p1363 };
pub const GoogleSignatureOptions = struct { encoding: GoogleSignatureEncoding = .der, padding: ?i32 = null, saltLength: ?i32 = null };
pub fn google(a: std.mem.Allocator, io: std.Io, input: GoogleInput) !Output {
    return googleWithEncoding(a, io, input, .der);
}
/// Keeps the existing GoogleInput C layout and default DER entrypoint intact.
pub fn googleWithEncoding(a: std.mem.Allocator, io: std.Io, input: GoogleInput, encoding: GoogleSignatureEncoding) !Output {
    return googleWithOptions(a, io, input, .{ .encoding = encoding });
}
pub fn googleWithOptions(a: std.mem.Allocator, io: std.Io, input: GoogleInput, options: GoogleSignatureOptions) !Output {
    try io.checkCancel();
    var output = try Output.init(a);
    errdefer output.deinit();
    const pem = try input.private_key.slice();
    const content = try input.content.slice();
    const storage = output.alloc();
    output.signature = view(try googleSignature(storage, io, pem, content, options));
    try io.checkCancel();
    return output;
}
fn googleEcSignature(comptime Scheme: type, a: std.mem.Allocator, io: std.Io, pem: []const u8, content: []const u8, encoding: GoogleSignatureEncoding) ![]u8 {
    const encoded = try @import("../crypto/tls_signing.zig").ecdsa(Scheme, a, io, pem, content);
    if (encoding == .der) return encoded;
    defer a.free(encoded);
    const signature = try Scheme.Signature.fromDer(encoded);
    return a.dupe(u8, &signature.toBytes());
}
fn googleSignature(a: std.mem.Allocator, io: std.Io, pem: []const u8, content: []const u8, options: GoogleSignatureOptions) ![]u8 {
    const encoding = options.encoding;
    const kind = keys.privateKeyType(a, io, pem) catch |err| {
        if (err != error.UnsupportedPrivateKey) return err;
        if (try keys.isPrivateEcCurve(a, io, pem, std.crypto.codecs.asn1.Oid.fromDotComptime("1.3.132.0.10"))) return googleEcSignature(std.crypto.sign.ecdsa.EcdsaSecp256k1Sha256, a, io, pem, content, encoding);
        if (try keys.isPrivateDsa(a, io, pem)) return if (encoding == .der) @import("../crypto/dsa_signing.zig").sign(a, io, pem, content) else @import("../crypto/dsa_signing.zig").signP1363(a, io, pem, content);
        if (try keys.isPrivateEd448(a, io, pem)) return error.GoogleUnsupportedCryptoOperation;
        return err;
    };
    return switch (kind) {
        .rsa => switch (options.padding orelse 1) {
            1 => try keys.signRs256(a, io, pem, content),
            5 => try @import("../crypto/tls_signing.zig").rsaX931Google(a, io, pem, content),
            6 => try @import("../crypto/tls_signing.zig").rsaPssGoogle(a, io, pem, content, options.saltLength),
            else => return error.UnsupportedGoogleSigningPadding,
        },
        .p256 => try googleEcSignature(std.crypto.sign.ecdsa.EcdsaP256Sha256, a, io, pem, content, encoding),
        .p384 => try googleEcSignature(std.crypto.sign.ecdsa.Ecdsa(std.crypto.ecc.P384, std.crypto.hash.sha2.Sha256), a, io, pem, content, encoding),
        .rsa_pss => if ((options.padding orelse 6) == 6) try @import("../crypto/tls_signing.zig").rsaPssGoogle(a, io, pem, content, options.saltLength) else return error.UnsupportedGoogleSigningPadding,
        .ed25519 => {
            var key = try keys.ed25519Private(a, io, pem);
            defer std.crypto.secureZero(u8, std.mem.asBytes(&key.secret_key));
            return error.GoogleUnsupportedCryptoOperation;
        },
    };
}
pub fn certificateLeaf(a: std.mem.Allocator, io: std.Io, source: []const u8) !Output {
    try io.checkCancel();
    var output = try Output.init(a);
    errdefer output.deinit();
    const storage = output.alloc();
    const bytes = if (keys.decodePem(storage, io, source)) |pem| blk: {
        if (!std.mem.eql(u8, pem.label, "CERTIFICATE")) return error.InvalidCertificate;
        break :blk pem.bytes;
    } else |err| switch (err) {
        error.InvalidPem => try storage.dupe(u8, source),
        else => return err,
    };
    _ = try (std.crypto.Certificate{ .buffer = bytes, .index = 0 }).parse();
    output.signature = view(bytes);
    try io.checkCancel();
    return output;
}
fn validateSupportedPrivateKey(a: std.mem.Allocator, io: std.Io, source: []const u8) !bool {
    const kind = keys.privateKeyType(a, io, source) catch |err| {
        if (err != error.UnsupportedPrivateKey) return err;
        if (try keys.isPrivateEcCurve(a, io, source, std.crypto.codecs.asn1.Oid.fromDotComptime("1.3.132.0.10"))) {
            var key = try keys.ecPrivateFor(std.crypto.sign.ecdsa.EcdsaSecp256k1Sha256, a, io, source);
            std.crypto.secureZero(u8, std.mem.asBytes(&key.secret_key));
            return true;
        }
        if (try keys.isPrivateDsa(a, io, source)) {
            var key = try keys.dsaPrivate(a, io, source);
            std.crypto.secureZero(u8, std.mem.asBytes(&key));
            return true;
        }
        return keys.isPrivateEd448(a, io, source);
    };
    switch (kind) {
        .rsa => {
            var key = try keys.rsaPrivate(a, io, source);
            std.crypto.secureZero(u8, std.mem.asBytes(&key));
        },
        .rsa_pss => {
            _ = try keys.rsaPssPrivateRules(a, io, source);
            var key = try keys.rsaPrivatePss(a, io, source);
            std.crypto.secureZero(u8, std.mem.asBytes(&key));
        },
        .p256 => {
            var key = try keys.p256Private(a, io, source);
            std.crypto.secureZero(u8, std.mem.asBytes(&key.secret_key));
        },
        .p384 => {
            var key = try keys.p384Private(a, io, source);
            std.crypto.secureZero(u8, std.mem.asBytes(&key.secret_key));
        },
        .ed25519 => {
            var key = try keys.ed25519Private(a, io, source);
            std.crypto.secureZero(u8, std.mem.asBytes(&key.secret_key));
        },
    }
    return true;
}
pub fn privateKey(a: std.mem.Allocator, io: std.Io, source: []const u8) !Output {
    try io.checkCancel();
    var output = try Output.init(a);
    errdefer output.deinit();
    const storage = output.alloc();
    const pem = try keys.decodePrivatePem(storage, io, source);
    defer std.crypto.secureZero(u8, pem.bytes);
    if (std.mem.eql(u8, pem.label, "RSA PRIVATE KEY") or std.mem.eql(u8, pem.label, "EC PRIVATE KEY") or std.mem.eql(u8, pem.label, "DSA PRIVATE KEY")) {
        if (!try validateSupportedPrivateKey(storage, io, source)) return error.UnsupportedPrivateKey;
    } else if (std.mem.eql(u8, pem.label, "PRIVATE KEY")) {
        try keys.validatePrivateKeyInfo(pem.bytes);
        // Validate inner material for every implemented key family. Unknown
        // PKCS8 families retain the existing outer-container-only behavior;
        // their complete import semantics remain separate unfinished work.
        _ = try validateSupportedPrivateKey(storage, io, source);
    } else return error.InvalidPrivateKey;
    output.signature = view(try storage.dupe(u8, source));
    try io.checkCancel();
    return output;
}
pub const CertificateChain = struct {
    allocator: std.mem.Allocator,
    source: []const u8,
    regex: @import("../regex/regexp.zig").Regex,
    offset: usize = 0,
    index: usize = 0,
    pub fn open(a: std.mem.Allocator, io: std.Io, source: []const u8) !@This() {
        return .{ .allocator = a, .source = source, .regex = try @import("../regex/regexp.zig").Regex.init(a, io, "-----BEGIN CERTIFICATE-----[^-]+-----END CERTIFICATE-----", .{}) };
    }
    pub fn next(self: *@This(), io: std.Io) !?Output {
        var found = (try self.regex.exec(self.allocator, self.source, self.offset)) orelse return null;
        defer found.deinit();
        self.offset = found.captures[0].?.end;
        const pem = (try found.text(self.allocator, 0)).?;
        defer self.allocator.free(pem);
        const result = try certificateLeaf(self.allocator, io, pem);
        self.index += 1;
        return result;
    }
    pub fn deinit(self: *@This()) void {
        self.regex.deinit();
        self.* = undefined;
    }
};
pub const view = abi.StringView.fromSlice;
