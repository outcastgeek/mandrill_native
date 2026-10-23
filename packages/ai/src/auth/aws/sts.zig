//! STS Query requests use the standard URI encoder. The network-response XML
//! decoder remains unfinished at the explicit codecs.decodeSts boundary.
const std = @import("std");
const m = @import("../../models.zig");
const t = @import("../../types.zig");
const uri = @import("../../transport/uri.zig");
const signing = @import("../signing.zig");
const shared = @import("shared_config.zig");
const codecs = @import("codecs.zig");
const retry = @import("retry.zig");
pub const Credentials = @import("credentials.zig").Credentials;
pub fn request(operation: m.OperationContext, options: t.StreamOptions, action: []const u8, fields: []const uri.Pair, source: ?Credentials, profile_region: ?[]const u8) !Credentials {
    const a = operation.allocator;
    const io = operation.io;
    const region = profile_region orelse operation.awsRegion orelse (try shared.sdkEnv(operation, "AWS_REGION")) orelse (try shared.setting(operation, options, "region")) orelse "us-east-1";
    const normalized_region = try @import("region.zig").realRegion(operation, region);
    const signing_region = if (std.mem.eql(u8, normalized_region, "aws-global")) "us-east-1" else normalized_region;
    const endpoint = try @import("endpoints.zig").resolve(operation, options, .sts, region);
    const url = try std.Uri.parse(endpoint);
    var params: std.ArrayList(uri.Pair) = .empty;
    try params.appendSlice(a, &.{ .{ .name = "Action", .value = action }, .{ .name = "Version", .value = "2011-06-15" } });
    try params.appendSlice(a, fields);
    const body = try uri.encode(a, params.items);
    var policy = try retry.Policy.init(operation, options);
    while (true) {
        var extra = try policy.requestHeaders(a, &.{.{ .name = "content-type", .value = "application/x-www-form-urlencoded" }});
        if (source) |credentials| {
            try credentials.validateSigning(operation);
            var pairs: std.ArrayList(signing.Pair) = .empty;
            for (extra) |header| if (header.value) |value| try pairs.append(a, .{ .name = signing.view(header.name), .value = signing.view(value) });
            const timestamp = @floor((@as(f64, @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds())) + policy.clockOffsetMs) / 1000);
            if (!std.math.isFinite(timestamp) or timestamp < 0 or timestamp >= @as(f64, @floatFromInt(std.math.maxInt(u64)))) return error.InvalidAwsSigningTimestamp;
            try credentials.validateSigningToken(operation);
            var output = try signing.aws(a, io, .{ .method = signing.view("POST"), .uri = signing.view(endpoint), .body = signing.view(body), .headers = signing.Pairs.fromSlice(pairs.items), .region = signing.view(signing_region), .service = signing.view("sts"), .access_key = signing.view(credentials.accessKeyId), .secret_key = signing.view(credentials.secretAccessKey), .session_token = signing.view(credentials.sessionToken orelse ""), .timestamp_seconds = @intFromFloat(timestamp) });
            defer output.deinit();
            var signed: std.ArrayList(t.ProviderHeader) = .empty;
            try signed.appendSlice(a, extra);
            for (try output.headers.slice()) |header| try signed.append(a, .{ .name = try a.dupe(u8, try header.name.slice()), .value = try a.dupe(u8, try header.value.slice()) });
            extra = try @import("../../utils/headers.zig").merge(a, &.{signed.items});
        }
        var response = operation.transport.sendReadAll(a, io, .{ .method = .POST, .uri = url, .headers = extra, .body = body, .followRedirects = false, .decompressResponse = false }, .unlimited) catch |err| {
            if (err == error.Canceled or err == error.OutOfMemory) return err;
            if (try policy.waitForRetry(retry.transportError(err), &.{})) continue;
            return err;
        };
        defer response.response.deinit();
        defer a.free(response.bytes);
        const decoded = try codecs.decodeSts(a, io, response.bytes);
        if (response.response.status >= 200 and response.response.status < 300) {
            const value = decoded.credentials orelse return error.InvalidAwsStsResponse;
            return .{ .accessKeyId = value.accessKeyId, .secretAccessKey = value.secretAccessKey, .sessionToken = value.sessionToken, .expiration = value.expiration };
        }
        _ = try policy.responseClock(response.response.headers, null);
        if (try policy.waitForRetry(retry.classify(response.response.status, decoded.code, decoded.message), response.response.headers)) continue;
        if (operation.errorMessage) |slot| slot.* = decoded.message orelse decoded.code orelse "AWS STS request failed";
        return error.AwsStsRequestFailed;
    }
}
