//! SigV4 uses standard SHA-256/HMAC, URI codecs, typed header/query records,
//! and a writer for the signature protocol's canonical representation.
const std = @import("std");
const uri_codec = @import("../transport/uri.zig");
pub const Header = std.http.Header;
pub const Canonicalization = enum { smithy, google };
/// Borrowed through sign(); invoked after HMAC when Authorization is built.
/// Returned text may borrow context or use the supplied result allocator.
pub const AccessKeyResolver = struct { context: ?*anyopaque, resolve: *const fn (?*anyopaque, std.mem.Allocator) anyerror![]const u8 };
/// Borrowed date text through sign(); the ordinary unsigned timestamp path
/// remains the default. Google supplies Date.toISOString-derived fields.
pub const DateText = struct { day: []const u8, time: []const u8 };
pub const Input = struct { method: []const u8, uri: std.Uri, body: []const u8, headers: []const Header, region: []const u8, service: []const u8, accessKey: []const u8, secretKey: []const u8, sessionToken: ?[]const u8, timestamp: u64, canonicalization: Canonicalization = .smithy, accessKeyResolver: ?AccessKeyResolver = null, dateText: ?DateText = null };
/// Both returned fields are owned by a. Match Google's literal ISO text
/// replacements, including removal of a negative expanded-year sign.
pub fn googleDateText(a: std.mem.Allocator, milliseconds: i128) !DateText {
    const iso = try @import("../utils/datetime.zig").formatJavaScriptIsoUtc(a, milliseconds);
    defer a.free(iso);
    const no_dash = try std.mem.replaceOwned(u8, a, iso, "-", "");
    defer a.free(no_dash);
    const compact = try std.mem.replaceOwned(u8, a, no_dash, ":", "");
    defer a.free(compact);
    const time = try std.fmt.allocPrint(a, "{s}Z", .{compact[0..std.mem.findScalar(u8, compact, '.').?]});
    errdefer a.free(time);
    const day = try a.dupe(u8, compact[0..std.mem.findScalar(u8, compact, 'T').?]);
    return .{ .day = day, .time = time };
}
pub const Result = struct { headers: []const Header, signature: []const u8 };
const Hmac = std.crypto.auth.hmac.sha2.HmacSha256;
fn hash(bytes: []const u8) [64]u8 {
    var digest: [32]u8 = undefined;
    std.crypto.hash.sha2.Sha256.hash(bytes, &digest, .{});
    return std.fmt.bytesToHex(&digest, .lower);
}
fn mac(key: []const u8, bytes: []const u8) [32]u8 {
    var digest: [32]u8 = undefined;
    Hmac.create(&digest, bytes, key);
    return digest;
}
fn trimHeader(a: std.mem.Allocator, bytes: []const u8) ![]const u8 {
    const pieces = try @import("../utils/unicode.zig").splitWhitespace(a, bytes);
    defer {
        for (pieces) |piece| a.free(piece);
        a.free(pieces);
    }
    return std.mem.join(a, " ", pieces);
}
fn pathCharacter(c: u8) bool {
    return std.ascii.isAlphanumeric(c) or c == '-' or c == '_' or c == '.' or c == '~' or c == '/';
}
fn path(a: std.mem.Allocator, uri: std.Uri) ![]const u8 {
    var encoded = std.Io.Writer.Allocating.init(a);
    defer encoded.deinit();
    try uri.path.formatPath(&encoded.writer);
    const original = encoded.written();
    const normalized = try std.fs.path.resolvePosix(a, &.{if (original.len == 0) "/" else original});
    defer a.free(normalized);
    var result = std.Io.Writer.Allocating.init(a);
    defer result.deinit();
    try std.Uri.Component.percentEncode(&result.writer, normalized, pathCharacter);
    if (normalized.len > 1 and std.mem.endsWith(u8, original, "/")) try result.writer.writeByte('/');
    return result.toOwnedSlice();
}
const Query = struct { name: []const u8, value: []const u8 };
fn queryOrder(_: void, left: Query, right: Query) bool {
    const order = std.mem.order(u8, left.name, right.name);
    return order == .lt or (order == .eq and std.mem.lessThan(u8, left.value, right.value));
}
fn decodeQueryComponent(a: std.mem.Allocator, encoded: []const u8) ![]const u8 {
    if (try @import("../regex/regexp.zig").matches(a, null, "%(?![0-9A-Fa-f]{2})", encoded, .{})) return error.InvalidAwsSigningQuery;
    const decoded = try (std.Uri.Component{ .percent_encoded = encoded }).toRawMaybeAlloc(a);
    if (!std.unicode.utf8ValidateSlice(decoded)) return error.InvalidAwsSigningQuery;
    return decoded;
}
// Smithy's parser uses an ordinary object and `key in query`. A first
// inherited name therefore starts a value array with the inherited value.
// __proto__ changes that object's prototype to an array, whose indexed
// values/length/methods can then be inherited by later query names.
fn inheritedQueryValue(a: std.mem.Allocator, name: []const u8, prototype: ?[]const ?[]const u8) !?[]const u8 {
    if (prototype) |items| {
        if (std.mem.eql(u8, name, "length")) return try std.fmt.allocPrint(a, "{d}", .{items.len + 1});
        if (@import("../json/value.zig").arrayIndex(name)) |index| {
            if (index == 0) return "[object Object]";
            if (index <= items.len) return items[index - 1] orelse "null";
        }
        if (std.mem.eql(u8, name, "constructor")) return "function Array() { [native code] }";
        for ([_][]const u8{ "at", "concat", "copyWithin", "fill", "find", "findIndex", "findLast", "findLastIndex", "lastIndexOf", "pop", "push", "reverse", "shift", "unshift", "slice", "sort", "splice", "includes", "indexOf", "join", "keys", "entries", "values", "forEach", "filter", "flat", "flatMap", "map", "every", "some", "reduce", "reduceRight", "toReversed", "toSorted", "toSpliced", "with", "toString", "toLocaleString" }) |method| {
            if (std.mem.eql(u8, name, method)) return try std.fmt.allocPrint(a, "function {s}() {{ [native code] }}", .{method});
        }
    }
    if (std.mem.eql(u8, name, "constructor")) return "function Object() { [native code] }";
    for ([_][]const u8{ "__defineGetter__", "__defineSetter__", "hasOwnProperty", "__lookupGetter__", "__lookupSetter__", "isPrototypeOf", "propertyIsEnumerable", "toLocaleString", "toString", "valueOf" }) |method| {
        if (std.mem.eql(u8, name, method)) return try std.fmt.allocPrint(a, "function {s}() {{ [native code] }}", .{method});
    }
    return null;
}
fn query(a: std.mem.Allocator, uri: std.Uri) ![]const u8 {
    const value = uri.query orelse return "";
    var encoded = std.Io.Writer.Allocating.init(a);
    defer encoded.deinit();
    try value.formatQuery(&encoded.writer);
    const bytes = encoded.written();
    if (bytes.len == 0) return "";
    var groups: std.StringArrayHashMapUnmanaged(std.ArrayList(?[]const u8)) = .{};
    defer {
        for (groups.values()) |*values| values.deinit(a);
        groups.deinit(a);
    }
    var prototype: ?std.ArrayList(?[]const u8) = null;
    defer if (prototype) |*items| items.deinit(a);
    var pairs: std.ArrayList(Query) = .empty;
    defer pairs.deinit(a);
    var fields = std.mem.splitScalar(u8, bytes, '&');
    while (fields.next()) |part| {
        var entry = std.mem.splitScalar(u8, part, '=');
        const key = entry.first();
        const raw_value = entry.next();
        // Smithy decodes only these first two components; discarded '='
        // suffixes are neither decoded nor rejected for malformed escapes.
        const key_raw = try decodeQueryComponent(a, key);
        const value_raw = if (raw_value) |text| try decodeQueryComponent(a, text) else null;
        if (std.mem.eql(u8, key_raw, "__proto__")) {
            if (prototype == null) prototype = .empty;
            try prototype.?.append(a, value_raw);
            continue;
        }
        const group = try groups.getOrPut(a, key_raw);
        if (!group.found_existing) {
            group.value_ptr.* = .empty;
            if (try inheritedQueryValue(a, key_raw, if (prototype) |items| items.items else null)) |inherited| try group.value_ptr.append(a, inherited);
        }
        try group.value_ptr.append(a, value_raw);
    }
    for (groups.keys(), groups.values()) |name, values| {
        if (std.ascii.eqlIgnoreCase(name, "X-Amz-Signature") or std.mem.eql(u8, name, "__proto__")) continue;
        const encoded_name = try uri_codec.encodeComponent(a, name);
        for (values.items) |text| {
            if (text == null and values.items.len == 1) continue;
            try pairs.append(a, .{ .name = encoded_name, .value = try uri_codec.encodeComponent(a, text orelse "null") });
        }
    }
    std.mem.sort(Query, pairs.items, {}, queryOrder);
    var output = std.Io.Writer.Allocating.init(a);
    defer output.deinit();
    for (pairs.items, 0..) |pair, index| {
        if (index > 0) try output.writer.writeByte('&');
        try output.writer.print("{s}={s}", .{ pair.name, pair.value });
    }
    return output.toOwnedSlice();
}
const Date = struct { day: [8]u8, time: [16]u8 };
fn date(timestamp: u64) !Date {
    const epoch = std.time.epoch.EpochSeconds{ .secs = timestamp };
    const year = epoch.getEpochDay().calculateYearDay();
    if (year.year > 9999) return error.InvalidAwsSigningTimestamp;
    const month = year.calculateMonthDay();
    const seconds = epoch.getDaySeconds();
    var result: Date = undefined;
    _ = try std.fmt.bufPrint(&result.day, "{d:0>4}{d:0>2}{d:0>2}", .{ year.year, month.month.numeric(), @as(u8, month.day_index) + 1 });
    _ = try std.fmt.bufPrint(&result.time, "{s}T{d:0>2}{d:0>2}{d:0>2}Z", .{ result.day, seconds.getHoursIntoDay(), seconds.getMinutesIntoHour(), seconds.getSecondsIntoMinute() });
    return result;
}
fn excluded(name: []const u8) bool {
    for ([_][]const u8{ "authorization", "cache-control", "connection", "expect", "from", "keep-alive", "max-forwards", "pragma", "referer", "te", "trailer", "transfer-encoding", "upgrade", "user-agent", "x-amzn-trace-id", "constructor", "__proto__" }) |key| if (std.mem.eql(u8, name, key)) return true;
    return std.mem.startsWith(u8, name, "proxy-") or std.mem.startsWith(u8, name, "sec-");
}
pub fn sign(a: std.mem.Allocator, io: std.Io, input: Input) !Result {
    try io.checkCancel();
    var default_date: Date = undefined;
    const when: DateText = if (input.dateText) |value| value else when: {
        default_date = try date(input.timestamp);
        break :when .{ .day = &default_date.day, .time = &default_date.time };
    };
    const smithy = input.canonicalization == .smithy;
    // Smithy headers are an own-property record, sorted by original name
    // before lowercasing. Exact duplicates overwrite; case variants resolve
    // in that sorted order instead of becoming comma-joined field values.
    var original: std.StringArrayHashMapUnmanaged([]const u8) = .{};
    defer original.deinit(a);
    for (input.headers) |header| try original.put(a, header.name, header.value);
    const original_names = try a.dupe([]const u8, original.keys());
    defer a.free(original_names);
    std.mem.sort([]const u8, original_names, {}, struct {
        fn less(_: void, left: []const u8, right: []const u8) bool {
            return std.mem.lessThan(u8, left, right);
        }
    }.less);
    var headers: std.StringArrayHashMapUnmanaged([]const u8) = .{};
    defer headers.deinit(a);
    var supplied_hash: ?[]const u8 = null;
    if (smithy) for (original.keys(), original.values()) |name, value| {
        if (std.ascii.eqlIgnoreCase(name, "x-amz-content-sha256")) {
            supplied_hash = value;
            break;
        }
    };
    for (if (smithy) original_names else original.keys()) |original_name| {
        const name = try std.ascii.allocLowerString(a, original_name);
        if ((smithy and (excluded(name) or std.mem.eql(u8, name, "date") or std.mem.eql(u8, name, "x-amz-date"))) or (input.sessionToken != null and std.mem.eql(u8, name, "x-amz-security-token"))) continue;
        const value = original.get(original_name).?;
        const normalized = if (smithy) try trimHeader(a, value) else std.mem.trim(u8, value, " \t\r\n");
        const combined = if (!smithy) (if (headers.get(name)) |previous| try std.fmt.allocPrint(a, "{s}, {s}", .{ previous, normalized }) else normalized) else normalized;
        try headers.put(a, name, combined);
    }
    if (!headers.contains("host")) {
        var authority = input.uri;
        authority.user = null;
        authority.password = null;
        if (authority.port) |port| if ((std.ascii.eqlIgnoreCase(authority.scheme, "https") and port == 443) or (std.ascii.eqlIgnoreCase(authority.scheme, "http") and port == 80)) {
            authority.port = null;
        };
        var output = std.Io.Writer.Allocating.init(a);
        defer output.deinit();
        try authority.writeToStream(&output.writer, .{ .authority = true });
        try headers.put(a, "host", try output.toOwnedSlice());
    }
    const generated_date = smithy or !headers.contains("date");
    if (generated_date and (smithy or !headers.contains("x-amz-date"))) try headers.put(a, "x-amz-date", try a.dupe(u8, when.time));
    if (input.sessionToken) |token| try headers.put(a, "x-amz-security-token", token);
    const payload_hash = supplied_hash orelse try a.dupe(u8, &hash(input.body));
    const generated_checksum = smithy and supplied_hash == null;
    if (generated_checksum) try headers.put(a, "x-amz-content-sha256", payload_hash);
    const names = try a.dupe([]const u8, headers.keys());
    defer a.free(names);
    std.mem.sort([]const u8, names, {}, struct {
        fn less(_: void, left: []const u8, right: []const u8) bool {
            return std.mem.lessThan(u8, left, right);
        }
    }.less);
    const signed_headers = try std.mem.join(a, ";", names);
    var canonical = std.Io.Writer.Allocating.init(a);
    defer canonical.deinit();
    var google_path = std.Io.Writer.Allocating.init(a);
    defer google_path.deinit();
    var google_query = std.Io.Writer.Allocating.init(a);
    defer google_query.deinit();
    if (!smithy) {
        try input.uri.path.formatPath(&google_path.writer);
        if (input.uri.query) |value| try value.formatQuery(&google_query.writer);
    }
    const canonical_query = if (smithy) try query(a, input.uri) else google_query.written();
    const method = if (smithy) input.method else try std.ascii.allocUpperString(a, input.method);
    try canonical.writer.print("{s}\n{s}\n{s}\n", .{ method, if (smithy) try path(a, input.uri) else if (google_path.written().len == 0) "/" else google_path.written(), canonical_query });
    for (names) |name| try canonical.writer.print("{s}:{s}\n", .{ name, headers.get(name).? });
    try canonical.writer.print("\n{s}\n{s}", .{ signed_headers, payload_hash });
    const scope = try std.fmt.allocPrint(a, "{s}/{s}/{s}/aws4_request", .{ when.day, input.region, input.service });
    const to_sign = try std.fmt.allocPrint(a, "AWS4-HMAC-SHA256\n{s}\n{s}\n{s}", .{ when.time, scope, hash(canonical.written()) });
    const first = try std.mem.concat(a, u8, &.{ "AWS4", input.secretKey });
    defer {
        std.crypto.secureZero(u8, first);
        a.free(first);
    }
    var day_key = mac(first, when.day);
    defer std.crypto.secureZero(u8, &day_key);
    var region_key = mac(&day_key, input.region);
    defer std.crypto.secureZero(u8, &region_key);
    var service_key = mac(&region_key, input.service);
    defer std.crypto.secureZero(u8, &service_key);
    var signing_key = mac(&service_key, "aws4_request");
    defer std.crypto.secureZero(u8, &signing_key);
    const signature = mac(&signing_key, to_sign);
    const hex = std.fmt.bytesToHex(&signature, .lower);
    var output: std.ArrayList(Header) = .empty;
    if (generated_date) try output.append(a, .{ .name = "x-amz-date", .value = try a.dupe(u8, headers.get("x-amz-date").?) });
    if (generated_checksum) try output.append(a, .{ .name = "x-amz-content-sha256", .value = payload_hash });
    if (input.sessionToken) |token| try output.append(a, .{ .name = "x-amz-security-token", .value = token });
    const access_key = if (input.accessKeyResolver) |resolver| try resolver.resolve(resolver.context, a) else input.accessKey;
    try output.append(a, .{ .name = "authorization", .value = try std.fmt.allocPrint(a, "AWS4-HMAC-SHA256 Credential={s}/{s}, SignedHeaders={s}, Signature={s}", .{ access_key, scope, signed_headers, hex }) });
    try io.checkCancel();
    return .{ .headers = try output.toOwnedSlice(a), .signature = try a.dupe(u8, &hex) };
}
