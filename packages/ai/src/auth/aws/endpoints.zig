//! AWS SDK endpoint rules for the credential services. Partition records are
//! the pinned @aws-sdk/core data, decoded exclusively by std.json.
const std = @import("std");
const m = @import("../../models.zig");
const t = @import("../../types.zig");
const j = @import("../../json/value.zig");
const shared = @import("shared_config.zig");
pub const Service = enum { sts, sso, sso_oidc, signin, bedrock_runtime };
const Name = struct { dns: []const u8, env: []const u8 };
fn names(service: Service) Name {
    return switch (service) {
        .sts => .{ .dns = "sts", .env = "AWS_ENDPOINT_URL_STS" },
        .sso => .{ .dns = "portal.sso", .env = "AWS_ENDPOINT_URL_SSO" },
        .sso_oidc => .{ .dns = "oidc", .env = "AWS_ENDPOINT_URL_SSO_OIDC" },
        .signin => .{ .dns = "signin", .env = "AWS_ENDPOINT_URL_SIGNIN" },
        .bedrock_runtime => .{ .dns = "bedrock-runtime", .env = "AWS_ENDPOINT_URL_BEDROCK_RUNTIME" },
    };
}
pub const boolean = shared.configBoolean;
pub fn resolve(operation: m.OperationContext, options: t.StreamOptions, service: Service, region: []const u8) ![]const u8 {
    const normalized = try @import("region.zig").realRegion(operation, region);
    return resolveWithExplicit(operation, options, service, normalized, null, @import("region.zig").isFipsRegion(region));
}
pub fn resolveBedrock(operation: m.OperationContext, options: t.StreamOptions, region: []const u8, explicit: ?[]const u8, force_fips: bool) ![]const u8 {
    return resolveWithExplicit(operation, options, .bedrock_runtime, region, explicit, force_fips);
}
fn failure(operation: m.OperationContext, message: []const u8) anyerror {
    if (operation.errorMessage) |slot| slot.* = message;
    return error.AwsEndpointConfigurationConflict;
}
fn resolveWithExplicit(operation: m.OperationContext, options: t.StreamOptions, service: Service, region: []const u8, explicit: ?[]const u8, force_fips: bool) ![]const u8 {
    const a = operation.allocator;
    const name = names(service);
    const fips = force_fips or try boolean(operation, options, "AWS_USE_FIPS_ENDPOINT", "use_fips_endpoint");
    const dual = try boolean(operation, options, "AWS_USE_DUALSTACK_ENDPOINT", "use_dualstack_endpoint");
    var endpoint = explicit;
    if (endpoint == null and !try boolean(operation, options, "AWS_IGNORE_CONFIGURED_ENDPOINT_URLS", "ignore_configured_endpoint_urls")) {
        endpoint = nonempty(try shared.sdkEnv(operation, name.env)) orelse nonempty(try shared.sdkEnv(operation, "AWS_ENDPOINT_URL"));
        if (endpoint == null) {
            var files = try shared.Files.init(operation, options);
            defer files.deinit();
            endpoint = try files.serviceEndpoint(try shared.profile(operation, options), @tagName(service));
        }
    }
    if (endpoint) |url| if (url.len > 0) {
        if (service == .bedrock_runtime) {
            if (fips) return failure(operation, "Invalid Configuration: FIPS and custom endpoint are not supported");
            if (dual) return failure(operation, "Invalid Configuration: Dualstack and custom endpoint are not supported");
        }
        if (fips or dual) return error.AwsEndpointConfigurationConflict;
        _ = try std.Uri.parse(url);
        return url;
    };
    if (service == .sts and std.mem.eql(u8, region, "aws-global") and !fips and !dual) return a.dupe(u8, "https://sts.amazonaws.com");
    const partition = try partitionFor(operation, region);
    if (service == .bedrock_runtime) {
        const supports_fips = j.truthy(j.get(partition, "supportsFIPS") orelse .null);
        const supports_dual = j.truthy(j.get(partition, "supportsDualStack") orelse .null);
        if (fips and dual and (!supports_fips or !supports_dual)) return failure(operation, "FIPS and DualStack are enabled, but this partition does not support one or both");
        if (fips and !supports_fips) return failure(operation, "FIPS is enabled but this partition does not support FIPS");
        if (dual and !supports_dual) return failure(operation, "DualStack is enabled but this partition does not support DualStack");
    }
    if (service == .signin) return signinEndpoint(a, partition, region, fips, dual);
    const suffix = j.field(partition, if (dual) "dualStackDnsSuffix" else "dnsSuffix") orelse return error.InvalidAwsPartition;
    const is_gov = std.mem.eql(u8, j.field(partition, "name") orelse "", "aws-us-gov");
    if (service != .bedrock_runtime and fips and !dual and is_gov) return std.fmt.allocPrint(a, "https://{s}.{s}.amazonaws.com", .{ name.dns, region });
    return std.fmt.allocPrint(a, "https://{s}{s}.{s}.{s}", .{ name.dns, if (fips) "-fips" else "", region, suffix });
}
/// CreateOAuth2Token sets IsControlPlane=false and leaves IsOAuthEndpoint
/// false. These are its branches in the pinned Signin endpoint rules.
fn signinEndpoint(a: std.mem.Allocator, partition: std.json.Value, region: []const u8, fips: bool, dual: bool) ![]const u8 {
    const name = j.field(partition, "name") orelse return error.InvalidAwsPartition;
    const suffix = j.field(partition, if (dual) "dualStackDnsSuffix" else "dnsSuffix") orelse return error.InvalidAwsPartition;
    if (fips) {
        if (!j.truthy(j.get(partition, "supportsFIPS") orelse .null) or (dual and !j.truthy(j.get(partition, "supportsDualStack") orelse .null))) return error.AwsEndpointConfigurationConflict;
        if (!dual and std.mem.eql(u8, region, "us-gov-west-1")) return a.dupe(u8, "https://signin-fips.amazonaws-us-gov.com");
        if (!dual and std.mem.eql(u8, name, "aws-us-gov")) return std.fmt.allocPrint(a, "https://{s}.signin-fips.amazonaws-us-gov.com", .{region});
        return std.fmt.allocPrint(a, "https://signin-fips.{s}.{s}", .{ region, suffix });
    }
    if (dual) {
        if (!j.truthy(j.get(partition, "supportsDualStack") orelse .null)) return error.AwsEndpointConfigurationConflict;
        return std.fmt.allocPrint(a, "https://signin.{s}.{s}", .{ region, suffix });
    }
    const domain = if (std.mem.eql(u8, name, "aws")) "aws.amazon.com" else if (std.mem.eql(u8, name, "aws-us-gov")) "amazonaws-us-gov.com" else if (std.mem.eql(u8, name, "aws-iso")) "c2shome.ic.gov" else if (std.mem.eql(u8, name, "aws-iso-b")) "sc2shome.sgov.gov" else if (std.mem.eql(u8, name, "aws-iso-f")) "csphome.hci.ic.gov" else if (std.mem.eql(u8, name, "aws-iso-e")) "csphome.adc-e.uk" else if (std.mem.eql(u8, name, "aws-eusc")) "amazonaws-eusc.eu" else suffix;
    return std.fmt.allocPrint(a, "https://{s}.signin.{s}", .{ region, domain });
}
fn nonempty(value: ?[]const u8) ?[]const u8 {
    return if (value) |text| if (text.len > 0) text else null else null;
}
fn partitionFor(operation: m.OperationContext, region: []const u8) !std.json.Value {
    const a = operation.allocator;
    const data = try @import("../../json/value.zig").parseFromSliceLeaky(std.json.Value, a, @embedFile("partitions.json"), .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always });
    const partitions = j.get(data, "partitions") orelse return error.InvalidAwsPartition;
    for (partitions.array.items) |partition| if (j.get(j.get(partition, "regions") orelse .null, region) != null) return j.get(partition, "outputs").?;
    for (partitions.array.items) |partition| if (matchesPartition(j.field(partition, "id") orelse "", region)) return j.get(partition, "outputs").?;
    return j.get(partitions.array.items[0], "outputs").?;
}
/// The SDK's fixed partition identifier tests; this does not interpret
/// caller-provided patterns or parse a structured wire format.
fn matchesPartition(id: []const u8, region: []const u8) bool {
    const prefix = if (std.mem.eql(u8, id, "aws-cn")) "cn-" else if (std.mem.eql(u8, id, "aws-eusc")) "eusc-de-" else if (std.mem.eql(u8, id, "aws-iso")) "us-iso-" else if (std.mem.eql(u8, id, "aws-iso-b")) "us-isob-" else if (std.mem.eql(u8, id, "aws-iso-e")) "eu-isoe-" else if (std.mem.eql(u8, id, "aws-iso-f")) "us-isof-" else if (std.mem.eql(u8, id, "aws-us-gov")) "us-gov-" else "";
    var rest = region;
    if (prefix.len > 0) {
        if (!std.mem.startsWith(u8, region, prefix)) return false;
        rest = region[prefix.len..];
    } else {
        var parts = std.mem.splitScalar(u8, region, '-');
        const first = parts.next() orelse return false;
        var found = false;
        for ([_][]const u8{ "us", "eu", "ap", "sa", "ca", "me", "af", "il", "mx" }) |known| if (std.mem.eql(u8, first, known)) {
            found = true;
            break;
        };
        if (!found or region.len <= first.len) return false;
        rest = region[first.len + 1 ..];
    }
    var parts = std.mem.splitScalar(u8, rest, '-');
    const word = parts.next() orelse return false;
    const digits = parts.next() orelse return false;
    if (parts.next() != null or word.len == 0 or digits.len == 0) return false;
    for (word) |byte| if (!std.ascii.isAlphanumeric(byte) and byte != '_') return false;
    for (digits) |byte| if (!std.ascii.isDigit(byte)) return false;
    return true;
}
