//! Google metadata discovery and request policy from the pinned gcp-metadata.
const std = @import("std");
const m = @import("../../models.zig");
const shared = @import("../aws/shared_config.zig");
const uri = @import("../../transport/uri.zig");
const http = @import("http.zig");
const headers = @import("../../utils/headers.zig");
fn nonempty(value: ?[]const u8) ?[]const u8 {
    return if (value) |text| if (text.len > 0) text else null else null;
}
fn configuredHost(operation: m.OperationContext) !?[]const u8 {
    for ([_][]const u8{ "GCE_METADATA_IP", "GCE_METADATA_HOST" }) |name| if (try shared.sdkEnv(operation, name)) |value| {
        if (value.len > 0) return value;
        operation.allocator.free(value);
    };
    return null;
}
fn endpoint(operation: m.OperationContext, host: []const u8, property: []const u8) !std.Uri {
    const base = if (std.mem.startsWith(u8, host, "http://") or std.mem.startsWith(u8, host, "https://")) host else try std.fmt.allocPrint(operation.allocator, "http://{s}", .{host});
    var url = try uri.parseHttpUrl(operation.allocator, base);
    url.path = .{ .raw = property };
    url.query = null;
    url.fragment = null;
    return url;
}
pub fn residency(operation: m.OperationContext) !bool {
    const state = operation.googleState orelse return detectResidency(operation);
    try state.mutex.lock(operation.io);
    defer state.mutex.unlock(operation.io);
    if (state.residency) |value| return value;
    const value = try detectResidency(operation);
    state.residency = value;
    return value;
}
fn detectResidency(operation: m.OperationContext) !bool {
    for ([_][]const u8{ "CLOUD_RUN_JOB", "FUNCTION_NAME", "K_SERVICE" }) |name| if (try shared.sdkEnv(operation, name)) |value| {
        defer operation.allocator.free(value);
        if (value.len > 0) return true;
    };
    if (comptime @import("builtin").os.tag == .linux) {
        if (try linuxBios(operation)) return true;
        return @import("metadata_linux.zig").googleMacAddress(operation.allocator, operation.io);
    }
    if (comptime @import("builtin").os.tag.isDarwin()) return @import("metadata_darwin.zig").googleMacAddress(operation.allocator, operation.io);
    return false;
}
fn linuxBios(operation: m.OperationContext) !bool {
    const directory = std.Io.Dir.cwd();
    _ = directory.statFile(operation.io, "/sys/class/dmi/id/bios_date", .{}) catch |err| {
        if (err == error.Canceled) return err;
        return false;
    };
    const vendor = directory.readFileAlloc(operation.io, "/sys/class/dmi/id/bios_vendor", operation.allocator, .unlimited) catch |err| {
        if (err == error.Canceled or err == error.OutOfMemory) return err;
        return false;
    };
    defer operation.allocator.free(vendor);
    return std.mem.find(u8, vendor, "Google") != null;
}
fn requestHostResponse(operation: m.OperationContext, host: []const u8, property: []const u8, scopes: bool, retries: u32) !http.Response {
    var url = try endpoint(operation, host, property);
    if (scopes) url.query = .{ .percent_encoded = try uri.encode(operation.allocator, &.{.{ .name = "scopes", .value = "https://www.googleapis.com/auth/cloud-platform" }}) };
    return http.requestWithPolicy(operation.allocator, operation.io, operation.transport, .GET, try uri.render(operation.allocator, url), null, null, &.{.{ .name = "metadata-flavor", .value = "Google" }}, .{ .timeoutMs = if (try residency(operation)) null else 3000, .maxRetries = 3, .noResponseRetries = retries, .responseType = .text, .errorMessage = if (scopes) operation.errorMessage else null, .errorStorage = operation.errorStorage, .errorAllocator = operation.allocator }) catch |err| {
        if (scopes and err == error.RequestTimeout) if (operation.errorMessage) |slot| {
            slot.* = try std.fmt.allocPrint(operation.allocator, "Could not refresh access token: {s}", .{http.timeoutMessage});
        };
        return err;
    };
}
fn requestHostHttp(operation: m.OperationContext, host: []const u8, property: []const u8, scopes: bool, retries: u32) !http.Response {
    const response = try requestHostResponse(operation, host, property, scopes, retries);
    if (!response.ok()) {
        if (scopes) if (operation.errorMessage) |slot| {
            const explanation = switch (response.status) {
                403 => "A Forbidden error was returned while attempting to retrieve an access token for the Compute Engine built-in service account. This may be because the Compute Engine instance does not have the correct permission scopes specified: ",
                404 => "A Not Found error was returned while attempting to retrieve an accesstoken for the Compute Engine built-in service account. This may be because the Compute Engine instance does not have any permission scopes specified: ",
                else => "",
            };
            slot.* = try std.fmt.allocPrint(operation.allocator, "{s}{s}Could not refresh access token: {s}", .{ if (response.status == 403 or response.status == 404) "Could not refresh access token: " else "", explanation, response.raw });
        };
        return error.GoogleMetadataRequestFailed;
    }
    return response;
}
fn flavorDiagnostic(operation: m.OperationContext, response: http.Response) !?[]const u8 {
    var temporary = std.heap.ArenaAllocator.init(operation.allocator);
    defer temporary.deinit();
    const received = try headers.headersToRecord(temporary.allocator(), response.headers);
    const flavor = @import("../../json/value.zig").field(received, "metadata-flavor");
    if (!std.mem.eql(u8, flavor orelse "", "Google")) {
        return try std.fmt.allocPrint(operation.allocator, "Invalid response from metadata service: incorrect Metadata-Flavor header. Expected 'Google', got {s}", .{if (flavor != null and flavor.?.len > 0) try std.fmt.allocPrint(operation.allocator, "'{s}'", .{flavor.?}) else "no header"});
    }
    return null;
}
fn validateFlavor(operation: m.OperationContext, response: http.Response, scopes: bool) !void {
    if (try flavorDiagnostic(operation, response)) |message| {
        if (scopes) if (operation.errorMessage) |slot| {
            slot.* = message;
        };
        return error.InvalidGoogleMetadataFlavor;
    }
}
fn metadataJsonAccepted(operation: m.OperationContext, bytes: []const u8) !bool {
    const regex = @import("../../regex/regexp.zig");
    var scanner = std.json.Scanner.initCompleteInput(operation.allocator, bytes);
    defer scanner.deinit();
    while (true) {
        try operation.io.checkCancel();
        const scanned_token = scanner.nextAllocMax(operation.allocator, .alloc_if_needed, bytes.len) catch |err| {
            if (err == error.OutOfMemory) return err;
            return false;
        };
        defer switch (scanned_token) {
            .allocated_number, .allocated_string => |text| operation.allocator.free(text),
            else => {},
        };
        switch (scanned_token) {
            .number, .allocated_number => |text| {
                const numeric = std.fmt.parseFloat(f64, text) catch return false;
                if (!std.math.isFinite(numeric)) return false;
            },
            .string, .allocated_string => |key| if (scanner.string_is_object_key) {
                // json-bigint's default prototype/constructor guards apply to
                // every decoded key before duplicate fields collapse. Scanner
                // owns syntax and key/value state, including escaped strings.
                if (try regex.matches(operation.allocator, operation.io,
                    \\(?:_|\\u005[Ff])(?:_|\\u005[Ff])(?:p|\\u0070)(?:r|\\u0072)(?:o|\\u006[Ff])(?:t|\\u0074)(?:o|\\u006[Ff])(?:_|\\u005[Ff])(?:_|\\u005[Ff])
                , key, .{}) or try regex.matches(operation.allocator, operation.io,
                    \\(?:c|\\u0063)(?:o|\\u006[Ff])(?:n|\\u006[Ee])(?:s|\\u0073)(?:t|\\u0074)(?:r|\\u0072)(?:u|\\u0075)(?:c|\\u0063)(?:t|\\u0074)(?:o|\\u006[Ff])(?:r|\\u0072)
                , key, .{})) return false;
            },
            .end_of_document => return true,
            else => {},
        }
    }
}
fn requestHost(operation: m.OperationContext, host: []const u8, property: []const u8, scopes: bool, retries: u32) !http.Response {
    var response = try requestHostHttp(operation, host, property, scopes, retries);
    try validateFlavor(operation, response, scopes);
    if (!try metadataJsonAccepted(operation, response.raw)) {
        response.body = .{ .string = response.raw };
        return response;
    }
    response.body = @import("../../json/value.zig").parseFromSliceLeaky(std.json.Value, operation.allocator, response.raw, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |err| blk: {
        if (err == error.OutOfMemory) return err;
        break :blk std.json.Value{ .string = response.raw };
    };
    return response;
}
pub fn token(operation: m.OperationContext) !http.Response {
    const host = try configuredHost(operation);
    defer if (host) |value| operation.allocator.free(value);
    return requestHost(operation, host orelse "http://169.254.169.254", "/computeMetadata/v1/instance/service-accounts/default/token", true, 3);
}
pub fn project(operation: m.OperationContext) !http.Response {
    const host = try configuredHost(operation);
    defer if (host) |value| operation.allocator.free(value);
    return requestHost(operation, host orelse "http://169.254.169.254", "/computeMetadata/v1/project/project-id", false, 3);
}
pub fn available(operation: m.OperationContext) !bool {
    const detection = try shared.sdkEnv(operation, "METADATA_SERVER_DETECTION");
    defer if (detection) |value| operation.allocator.free(value);
    if (nonempty(detection)) |raw| {
        const mode = try @import("../../utils/unicode.zig").trimWhitespace(operation.allocator, raw);
        defer operation.allocator.free(mode);
        if (std.ascii.eqlIgnoreCase(mode, "assume-present")) return true;
        if (std.ascii.eqlIgnoreCase(mode, "none")) return false;
        if (std.ascii.eqlIgnoreCase(mode, "bios-only")) return residency(operation);
        if (!std.ascii.eqlIgnoreCase(mode, "ping-only")) {
            if (operation.errorMessage) |slot| {
                const lowered = try std.ascii.allocLowerString(operation.allocator, mode);
                defer operation.allocator.free(lowered);
                slot.* = try std.fmt.allocPrint(operation.allocator, "Unknown `METADATA_SERVER_DETECTION` env variable. Got `{s}`, but it should be `assume-present`, `none`, `bios-only`, `ping-only`, or unset", .{lowered});
            }
            return error.InvalidGoogleMetadataDetection;
        }
    }
    const state = operation.googleState orelse return detect(operation);
    try state.mutex.lock(operation.io);
    while (state.checking) state.changed.wait(operation.io, &state.mutex) catch |err| {
        state.mutex.unlock(operation.io);
        return err;
    };
    if (state.availability) |value| {
        state.mutex.unlock(operation.io);
        return value;
    }
    state.checking = true;
    state.mutex.unlock(operation.io);
    const result = detect(operation);
    state.mutex.lockUncancelable(operation.io);
    state.checking = false;
    if (result) |value| {
        state.availability = value;
    } else |_| {}
    state.changed.broadcast(operation.io);
    state.mutex.unlock(operation.io);
    return result;
}
fn detect(operation: m.OperationContext) !bool {
    const retry_value = try shared.sdkEnv(operation, "DETECT_GCP_RETRIES");
    defer if (retry_value) |value| operation.allocator.free(value);
    const retry_text = nonempty(retry_value);
    const retries: u32 = if (retry_text) |text| blk: {
        const number = try @import("../../utils/numbers.zig").coerceValue(operation.allocator, .{ .string = text });
        if (std.math.isNan(number) or std.math.isPositiveInf(number)) break :blk 3;
        if (number <= 0) break :blk 0;
        break :blk @intFromFloat(@min(@ceil(number), @as(f64, @floatFromInt(std.math.maxInt(u32)))));
    } else 0;
    if (try configuredHost(operation)) |host| {
        defer operation.allocator.free(host);
        return probe(operation, host, retries);
    }
    // Race fulfilled HTTP requests, before checking Metadata-Flavor. A bad
    // flavor on the winning response must not fall through to the other host.
    // Keep both request arenas alive until the loser joins and the winner's
    // headers have been validated. Their caller-owned backing is synchronized.
    var backing = @import("../../utils/synchronized_allocator.zig").SynchronizedAllocator{ .child = operation.allocator, .io = operation.io };
    var ip_arena = std.heap.ArenaAllocator.init(backing.allocator());
    defer ip_arena.deinit();
    var dns_arena = std.heap.ArenaAllocator.init(backing.allocator());
    defer dns_arena.deinit();
    var ip_operation = operation;
    ip_operation.allocator = ip_arena.allocator();
    var dns_operation = operation;
    dns_operation.allocator = dns_arena.allocator();
    const Outcome = union(enum) { ip: anyerror!http.Response, dns: anyerror!http.Response };
    var buffer: [2]Outcome = undefined;
    var select = std.Io.Select(Outcome).init(operation.io, &buffer);
    defer select.cancelDiscard();
    try select.concurrent(.ip, requestHostHttp, .{ ip_operation, "http://169.254.169.254", "/computeMetadata/v1/instance", false, retries });
    try select.concurrent(.dns, requestHostHttp, .{ dns_operation, "http://metadata.google.internal.", "/computeMetadata/v1/instance", false, retries });
    for (0..2) |_| {
        const value = switch (try select.await()) {
            .ip => |result| result,
            .dns => |result| result,
        };
        const response = value catch |err| {
            if (err == error.Canceled or err == error.OutOfMemory) return err;
            continue;
        };
        select.cancelDiscard();
        if (try flavorDiagnostic(operation, response)) |message| {
            try lookupWarning(operation, "RangeError", message, "UNKNOWN", true);
            return false;
        }
        return true;
    }
    try lookupWarning(operation, "AggregateError", "All promises were rejected", "UNKNOWN", true);
    return false;
}
fn lookupWarning(operation: m.OperationContext, class: []const u8, message: []const u8, code: []const u8, emit: bool) !void {
    const debug = try shared.sdkEnv(operation, "DEBUG_AUTH");
    defer if (debug) |value| operation.allocator.free(value);
    if (nonempty(debug) != null) {
        const text = try std.fmt.allocPrint(operation.allocator, "{s}{s}{s}\n", .{ class, if (message.len > 0) ": " else "", message });
        defer operation.allocator.free(text);
        std.Io.File.stdout().writeStreamingAll(operation.io, text) catch |err| {
            if (err == error.Canceled) return err;
        };
    }
    if (emit) {
        const text = try std.fmt.allocPrint(operation.allocator, "MetadataLookupWarning: received unexpected error = {s} code = {s}\n", .{ message, code });
        defer operation.allocator.free(text);
        std.Io.File.stderr().writeStreamingAll(operation.io, text) catch |err| {
            if (err == error.Canceled) return err;
        };
    }
}
fn probe(operation: m.OperationContext, host: []const u8, retries: u32) !bool {
    var arena = std.heap.ArenaAllocator.init(operation.allocator);
    defer arena.deinit();
    var local = operation;
    local.allocator = arena.allocator();
    const response = requestHostResponse(local, host, "/computeMetadata/v1/instance", false, retries) catch |err| {
        if (err == error.Canceled or err == error.OutOfMemory) return err;
        const code: ?[]const u8 = switch (err) {
            error.ConnectionRefused => "ECONNREFUSED",
            error.HostUnreachable => "EHOSTUNREACH",
            error.HostDown => "EHOSTDOWN",
            error.NetworkUnreachable => "ENETUNREACH",
            error.FileNotFound => "ENOENT",
            error.UnknownHostName => "ENOTFOUND",
            else => null,
        };
        try lookupWarning(operation, "Error", if (err == error.RequestTimeout) http.timeoutMessage else @errorName(err), code orelse "UNKNOWN", code == null);
        return false;
    };
    if (!response.ok()) {
        const code = try std.fmt.allocPrint(operation.allocator, "{d}", .{response.status});
        defer operation.allocator.free(code);
        try lookupWarning(operation, "Error", response.raw, code, response.status != 404);
        return false;
    }
    if (try flavorDiagnostic(operation, response)) |message| {
        try lookupWarning(operation, "RangeError", message, "UNKNOWN", true);
        return false;
    }
    return true;
}
