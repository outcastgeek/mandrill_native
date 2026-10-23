//! Certificate subject-token supplier.
const std = @import("std");
const m = @import("../../models.zig");
const j = @import("../../json/value.zig");
const signing = @import("../signing.zig");
const ambient = @import("../aws/shared_config.zig").sdkEnv;
const transport = @import("../../transport/http.zig");
const Value = std.json.Value;
const Report = struct {
    allocator: std.mem.Allocator,
    slot: ?*?[]const u8,
    fn fail(self: @This(), err: anyerror, comptime format: []const u8, args: anytype) anyerror {
        if (self.slot) |slot| slot.* = std.fmt.allocPrint(self.allocator, format, args) catch return error.OutOfMemory;
        return err;
    }
};
fn fatal(err: anyerror) bool {
    return err == error.Canceled or err == error.OutOfMemory or err == error.NativeSigningDisabled;
}
fn nonempty(value: ?[]const u8) ?[]const u8 {
    return if (value) |text| if (text.len > 0) text else null else null;
}
fn validFile(operation: m.OperationContext, path: []const u8) !bool {
    const stat = std.Io.Dir.cwd().statFile(operation.io, path, .{ .follow_symlinks = false }) catch |err| {
        if (fatal(err)) return err;
        return false;
    };
    return stat.kind == .file;
}
pub fn validate(operation: m.OperationContext, options: Value) !void {
    return validateWithReport(.{ .allocator = operation.allocator, .slot = operation.errorMessage }, options);
}
fn validateWithReport(report: Report, options: Value) !void {
    const defaults = j.truthy(j.get(options, "use_default_certificate_config") orelse .null);
    const configured = j.truthy(j.get(options, "certificate_config_location") orelse .null);
    if (!defaults and !configured) return report.fail(error.InvalidGoogleCertificateConfiguration, "Either `useDefaultCertificateConfig` must be true or a `certificateConfigLocation` must be provided.", .{});
    if (defaults and configured) return report.fail(error.InvalidGoogleCertificateConfiguration, "Both `useDefaultCertificateConfig` and `certificateConfigLocation` cannot be provided.", .{});
}
const configJoin = @import("../../utils/node_path.zig").join;
fn configPath(operation: m.OperationContext, report: Report, options: Value) ![]const u8 {
    try validateWithReport(report, options);
    const configured = j.get(options, "certificate_config_location") orelse .null;
    const has_override = j.truthy(configured);
    const override = nonempty(j.text(configured));
    if (has_override and override == null) return report.fail(error.GoogleCertificateSourceUnavailable, "Provided certificate config path is invalid: {s}", .{try @import("../../utils/diagnostics.zig").formatThrownValue(operation.allocator, .{ .value = configured })});
    if (override) |path| {
        if (try validFile(operation, path)) return operation.allocator.dupe(u8, path);
        return report.fail(error.GoogleCertificateSourceUnavailable, "Provided certificate config path is invalid: {s}", .{path});
    }
    if (nonempty(try ambient(operation, "GOOGLE_API_CERTIFICATE_CONFIG"))) |path| {
        if (try validFile(operation, path)) return path;
        return report.fail(error.GoogleCertificateSourceUnavailable, "Path from environment variable \"GOOGLE_API_CERTIFICATE_CONFIG\" is invalid: {s}", .{path});
    }
    const a = operation.allocator;
    const directory = nonempty(try ambient(operation, "CLOUDSDK_CONFIG")) orelse if (@import("builtin").os.tag == .windows)
        try configJoin(a, &.{ nonempty(try ambient(operation, "APPDATA")) orelse "", "gcloud" })
    else
        try configJoin(a, &.{ nonempty(try ambient(operation, "HOME")) orelse "", ".config", "gcloud" });
    const path = try configJoin(a, &.{ directory, "certificate_config.json" });
    if (try validFile(operation, path)) return path;
    return report.fail(error.GoogleCertificateSourceUnavailable, "Could not find certificate configuration file. Searched override path, the \"GOOGLE_API_CERTIFICATE_CONFIG\" env var, and the gcloud path ({s}).", .{path});
}
fn readPaths(operation: m.OperationContext, report: Report, path: []const u8) !void {
    const bytes = std.Io.Dir.cwd().readFileAlloc(operation.io, path, operation.allocator, .unlimited) catch |err| {
        if (fatal(err)) return err;
        return report.fail(error.GoogleCertificateSourceUnavailable, "Failed to read certificate config file at: {s}", .{path});
    };
    defer operation.allocator.free(bytes);
    const text = try std.fmt.allocPrint(operation.allocator, "{f}", .{std.unicode.fmtUtf8(bytes)});
    defer operation.allocator.free(text);
    const config = j.parseFromSliceLeaky(Value, operation.allocator, text, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |err| {
        if (fatal(err)) return err;
        return report.fail(error.InvalidGoogleCertificateConfiguration, "Failed to parse certificate config from {s}: {s}", .{ path, @errorName(err) });
    };
    const workload = j.get(j.get(config, "cert_configs") orelse .null, "workload") orelse .null;
    if (!j.truthy(j.get(workload, "cert_path") orelse .null) or !j.truthy(j.get(workload, "key_path") orelse .null)) return report.fail(error.InvalidGoogleCertificateConfiguration, "Certificate config file ({s}) is missing required \"cert_path\" or \"key_path\" in the workload config.", .{path});
}
pub const Material = struct {
    allocator: std.mem.Allocator,
    arena: *std.heap.ArenaAllocator,
    subjectToken: []const u8,
    clientCertificate: transport.ClientCertificate,
    privateKeyStorage: []u8,
    /// Consumers must finish and join all requests borrowing this material.
    pub fn deinit(self: *@This()) void {
        std.crypto.secureZero(u8, self.privateKeyStorage);
        self.arena.deinit();
        self.allocator.destroy(self.arena);
        self.* = undefined;
    }
};
const Prepared = struct { subjectToken: []const u8, certificate: []const u8, key: []u8 };
fn prepare(operation: m.OperationContext, report: Report, options: Value) !Prepared {
    const a = operation.allocator;
    const io = operation.io;
    const path = try configPath(operation, report, options);
    try readPaths(operation, report, path);
    const inputs = if (operation.authContext) |context| context.inputs else @import("../types.zig").CredentialInputs{};
    const certificate = try a.dupe(u8, inputs.googleCertificate orelse return error.GoogleAuthenticationCertificateUnavailable);
    var leaf = signing.certificateLeaf(a, io, certificate) catch |err| {
        if (fatal(err)) return err;
        return report.fail(error.GoogleCertificateSourceUnavailable, "Failed to parse supplied authentication certificate: {s}", .{@errorName(err)});
    };
    defer leaf.deinit();
    const leaf_der = try leaf.signature.slice();
    // Authentication certificates, the subject-token chain and private key
    // material come from memory; configuration path fields are never followed.
    const raw_key = if (operation.authContext) |context| context.inputs.googleCertificatePrivateKey orelse return error.GoogleCertificatePrivateKeyUnavailable else return error.GoogleCertificatePrivateKeyUnavailable;
    var normalized = signing.privateKey(a, io, raw_key) catch |err| {
        if (fatal(err)) return err;
        return report.fail(error.GoogleCertificateSourceUnavailable, "Failed to parse supplied private key: {s}", .{@errorName(err)});
    };
    defer normalized.deinit();
    const key = try a.dupe(u8, try normalized.signature.slice());
    errdefer std.crypto.secureZero(u8, key);
    var certificates: std.ArrayList([]const u8) = .empty;
    if (inputs.googleCertificateChain != null or j.truthy(j.get(options, "trust_chain_path") orelse .null)) {
        const bytes = inputs.googleCertificateChain orelse return error.GoogleAuthenticationCertificateChainUnavailable;
        var chain = try signing.CertificateChain.open(a, io, bytes);
        defer chain.deinit();
        while (true) {
            var next = (chain.next(io) catch |err| {
                if (fatal(err)) return err;
                return report.fail(error.InvalidGoogleCertificateConfiguration, "Failed to parse supplied certificate chain at index {d}: {s}", .{ chain.index, @errorName(err) });
            }) orelse break;
            defer next.deinit();
            try certificates.append(a, try a.dupe(u8, try next.signature.slice()));
        }
        const found: ?usize = blk: {
            for (certificates.items, 0..) |item, index| if (std.mem.eql(u8, leaf_der, item)) break :blk index;
            break :blk null;
        };
        if (found) |index| {
            if (index != 0) return report.fail(error.InvalidGoogleCertificateConfiguration, "Leaf certificate exists in the trust chain but is not the first entry (found at index {d}).", .{index});
        } else try certificates.insert(a, 0, leaf_der);
    } else try certificates.append(a, leaf_der);
    var encoded = j.array(a);
    for (certificates.items) |der| {
        const value = try a.alloc(u8, std.base64.standard.Encoder.calcSize(der.len));
        _ = std.base64.standard.Encoder.encode(value, der);
        try encoded.array.append(j.str(value));
    }
    return .{ .subjectToken = try j.stringify(a, encoded, .{}), .certificate = certificate, .key = key };
}
/// Returns independently owned material. The optional diagnostic slot belongs
/// to the outer operation allocator and survives a failed material allocation.
/// The pinned regex selects armor; standard Base64 and X509 decode each block.
pub fn load(operation: m.OperationContext, options: Value) !Material {
    try operation.io.checkCancel();
    const arena = try operation.allocator.create(std.heap.ArenaAllocator);
    arena.* = .init(operation.allocator);
    errdefer {
        arena.deinit();
        operation.allocator.destroy(arena);
    }
    var local = operation;
    local.allocator = arena.allocator();
    local.errorMessage = null;
    const prepared = try prepare(local, .{ .allocator = operation.allocator, .slot = operation.errorMessage }, options);
    return .{ .allocator = operation.allocator, .arena = arena, .subjectToken = prepared.subjectToken, .clientCertificate = .{ .certificate = prepared.certificate, .privateKey = prepared.key }, .privateKeyStorage = prepared.key };
}
