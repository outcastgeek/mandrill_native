const std = @import("std");
const re = @import("../regex/regexp.zig");
fn captureInt(match: re.Match, a: std.mem.Allocator, index: usize) !u16 {
    const text = (try match.text(a, index)) orelse return 0;
    defer a.free(text);
    return std.fmt.parseInt(u16, text, 10);
}
pub fn isDate(a: std.mem.Allocator, value: []const u8) !bool {
    var regex = try re.Regex.init(a, null,
        \\^(\d\d\d\d)-(\d\d)-(\d\d)$
    , .{});
    defer regex.deinit();
    var match = (try regex.exec(a, value, 0)) orelse return false;
    defer match.deinit();
    const year = try captureInt(match, a, 1);
    const month = try captureInt(match, a, 2);
    const day = try captureInt(match, a, 3);
    if (month < 1 or month > 12 or day < 1) return false;
    return day <= std.time.epoch.getDaysInMonth(year, @fromBackingInt(@intCast(month)));
}
fn isTime(a: std.mem.Allocator, value: []const u8) !bool {
    var regex = try re.Regex.init(a, null,
        \\^(\d\d):(\d\d):(\d\d)(?:\.\d+)?(?:([Zz])|([+-])(\d\d):(\d\d))?$
    , .{});
    defer regex.deinit();
    var match = (try regex.exec(a, value, 0)) orelse return false;
    defer match.deinit();
    if (match.captures[4] == null and match.captures[5] == null) return false;
    const hour = try captureInt(match, a, 1);
    const minute = try captureInt(match, a, 2);
    const second = try captureInt(match, a, 3);
    if (hour > 23 or minute > 59 or second > 60) return false;
    const zone_hour = try captureInt(match, a, 6);
    const zone_minute = try captureInt(match, a, 7);
    if (zone_hour > 23 or zone_minute > 59) return false;
    if (second < 60) return true;
    const sign = (try match.text(a, 5));
    defer if (sign) |text| a.free(text);
    const direction: i32 = if (sign) |text| (if (std.mem.eql(u8, text, "-")) -1 else 1) else 1;
    return @mod((@as(i32, hour) * 60 + minute) - direction * (@as(i32, zone_hour) * 60 + zone_minute), 1440) == 1439;
}
fn isDateTime(a: std.mem.Allocator, value: []const u8) !bool {
    var regex = try re.Regex.init(a, null, "^([^Tt]*)[Tt]([^Tt]*)$", .{});
    defer regex.deinit();
    var match = (try regex.exec(a, value, 0)) orelse return false;
    defer match.deinit();
    const date = (try match.text(a, 1)).?;
    defer a.free(date);
    const time = (try match.text(a, 2)).?;
    defer a.free(time);
    return try isDate(a, date) and try isTime(a, time);
}
fn asciiText(value: []const u8) bool {
    for (value) |byte| if (!std.ascii.isAscii(byte)) return false;
    return true;
}
fn hostname(a: std.mem.Allocator, value: []const u8) !?bool {
    if (value.len == 0 or try @import("../utils/unicode.zig").utf16Length(value) > 253) return false;
    if (!try re.matches(a, null, "^[^.]+(?:\\.[^.]+)*(?![\\s\\S])", value, .{})) return false;
    var labels = try re.Regex.init(a, null, "[^.]+", .{});
    defer labels.deinit();
    var rejected_ace = try re.Regex.init(a, null, @import("idn_hostname.zig").rejected_ace_pattern, .{});
    defer rejected_ace.deinit();
    var reserved = try re.Regex.init(a, null, "^..--", .{});
    defer reserved.deinit();
    var start: usize = 0;
    var unknown_ace = false;
    while (try labels.exec(a, value, start)) |found| {
        var match = found;
        defer match.deinit();
        start = match.captures[0].?.end;
        const label = (try match.text(a, 0)).?;
        defer a.free(label);
        if (try @import("../utils/unicode.zig").utf16Length(label) > 63) return false;
        if (std.ascii.startsWithIgnoreCase(label, "xn--")) {
            if (try rejected_ace.exec(a, label, 0)) |invalid| {
                var owned = invalid;
                owned.deinit();
                return false;
            }
            unknown_ace = true;
        } else {
            // Only ordinary labels use LDH validation. The pinned ACE branch
            // instead validates the decoded Unicode label, so its raw basic
            // characters must not be rejected by an unrelated LDH guard.
            std.Io.net.HostName.validate(label) catch return false;
            if (try reserved.exec(a, label, 0)) |invalid| {
                var owned = invalid;
                owned.deinit();
                return false;
            }
        }
    }
    return if (unknown_ace) null else true;
}
fn idnHostname(a: std.mem.Allocator, value: []const u8) !?bool {
    return @import("idn_hostname.zig").isValid(a, value);
}
fn idnEmail(a: std.mem.Allocator, value: []const u8) !?bool {
    const normalized = try @import("../utils/unicode_nfc.zig").normalize(a, value);
    defer a.free(normalized);
    return try re.matches(a, null,
        \\^(?:[A-Za-z0-9!#$%&'*+\/=?^_`{|}~\u{0080}-\u{10FFFF}-]+(?:\.[A-Za-z0-9!#$%&'*+\/=?^_`{|}~\u{0080}-\u{10FFFF}-]+)*|"(?:[^"\\]|\\.)*")@[\p{L}\p{N}](?:[\p{L}\p{N}-]{0,62})(?<!-)(?:\.[\p{L}\p{N}](?:[\p{L}\p{N}-]{0,62})(?<!-))*$
    , normalized, .{ .ignoreCase = true, .unicode = true });
}
fn formatPattern(a: std.mem.Allocator, name: []const u8, value: []const u8) !bool {
    for (@import("format_patterns.zig").patterns) |pattern| if (std.mem.eql(u8, name, pattern.name))
        return re.matches(a, null, pattern.expression, value, .{ .ignoreCase = pattern.ignoreCase, .unicode = pattern.unicode });
    return error.InvalidBuiltinFormat;
}
fn authoritySyntax(a: std.mem.Allocator, value: []const u8, relative: bool) !bool {
    const name = if (relative) "uri-reference" else "uri";
    if (asciiText(value)) return formatPattern(a, name, value);
    // Preserve delimiters while the standard component codec percent-encodes
    // non-ASCII data for the pinned RFC syntax predicate.
    var encoded = std.Io.Writer.Allocating.init(a);
    defer encoded.deinit();
    std.Uri.Component.percentEncode(&encoded.writer, value, std.ascii.isAscii) catch return error.OutOfMemory;
    return formatPattern(a, name, encoded.written());
}
fn bracketHost(a: std.mem.Allocator, host: []const u8) !bool {
    const parsed = @import("../transport/host.zig").parse(a, .{ .percent_encoded = host }) catch |err| {
        if (err == error.OutOfMemory) return err;
        return false;
    };
    return parsed == .ipv6;
}
fn specialAuthority(a: std.mem.Allocator, value: []const u8) !bool {
    return re.matches(a, null,
        \\^(?:[A-Za-z][A-Za-z0-9+.-]*:)?//(?:[^/?#]*@)?(?:\[[^\]]*\]|[^:/?#@]*)(?::[0-9]*)?(?=[/?#]|$)
    , value, .{});
}
fn standardUrl(a: std.mem.Allocator, value: []const u8, relative: bool) !?bool {
    // These cases require WHATWG input preprocessing before URI decoding.
    // Internal path/query controls can be accepted without changing the input;
    // controls in a parsed host are checked by its host predicate below.
    if (try re.matches(a, null,
        \\[\x09\x0a\x0d]|^[\x00-\x20]|[\x00-\x20]$
    , value, .{})) return null;
    if (!relative and !(try re.matches(a, null, "^[A-Za-z][A-Za-z0-9+.-]*:", value, .{}))) return false;
    var uri_storage = std.heap.ArenaAllocator.init(a);
    defer uri_storage.deinit();
    const uri = std.Uri.parse(value) catch @import("../transport/host.zig").parseUri(uri_storage.allocator(), value) catch |err| parsed: {
        if (err == error.OutOfMemory) return err;
        if (!relative) return null;
        break :parsed std.Uri.parseAfterScheme("", value) catch return null;
    };
    const file = std.ascii.eqlIgnoreCase(uri.scheme, "file");
    const special = uri.scheme.len == 0 or file or std.ascii.eqlIgnoreCase(uri.scheme, "http") or std.ascii.eqlIgnoreCase(uri.scheme, "https") or std.ascii.eqlIgnoreCase(uri.scheme, "ws") or std.ascii.eqlIgnoreCase(uri.scheme, "wss") or std.ascii.eqlIgnoreCase(uri.scheme, "ftp");
    if (special and try re.matches(a, null, "\\\\", value, .{})) return null;
    if (file and (uri.user != null or uri.password != null or uri.port != null)) return false;
    const literal_host = if (uri.host) |component| std.mem.startsWith(u8, switch (component) {
        .raw, .percent_encoded => |text| text,
    }, "[") else false;
    if (literal_host) {
        const host = switch (uri.host.?) {
            .raw, .percent_encoded => |text| text,
        };
        // Decide the literal before the RFC authority guard: malformed IPv6,
        // IPvFuture and zone/percent escapes are failures, not missing codecs.
        // absoluteIri has already narrowed its permitted IPvFuture case.
        if (!try bracketHost(a, host)) return false;
        if (!try specialAuthority(a, value)) return false;
        return true;
    }
    // std.Uri accepts integer-port spellings beyond URL's decimal grammar and
    // can omit text after a bracketed host. Keep these cases behind the pinned
    // syntax predicate rather than accepting information lost by the decoder.
    if (uri.host != null and !(try specialAuthority(a, value))) return false;
    if (!special) {
        // WHATWG opaque hosts retain percent escapes and do not invoke IDNA
        // or IPv4 conversion. std.Uri owns the component boundaries; the
        // predicate only checks the host's forbidden-code-point rule.
        const opaque_component = uri.host orelse return uri.user == null and uri.password == null and uri.port == null;
        const opaque_host = switch (opaque_component) {
            .raw, .percent_encoded => |text| text,
        };
        return !(try re.matches(a, null,
            \\[\x00\x09\x0a\x0d #/:<>?@\[\]\\^|]
        , opaque_host, .{}));
    }
    const component = uri.host orelse {
        if (uri.user != null or uri.password != null or uri.port != null) return false;
        if (file) return true;
        if (uri.scheme.len == 0) {
            const path = switch (uri.path) {
                .raw, .percent_encoded => |text| text,
            };
            // A relative path resolves against the reference's HTTP base.
            // Leading repeated slashes instead re-enter WHATWG host parsing.
            return if (std.mem.startsWith(u8, path, "//")) null else true;
        }
        return null;
    };
    const raw_host = switch (component) {
        .raw, .percent_encoded => |text| text,
    };
    if (file and raw_host.len == 0) return true;
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    _ = @import("../transport/host.zig").parse(temporary.allocator(), component) catch |err| {
        if (err == error.OutOfMemory) return err;
        return false;
    };
    return true;
}
/// Known absolute-URL validity through standard URI components and the existing
/// host predicates. Null means unresolved WHATWG preprocessing/host behavior;
/// it must not be presented as full WHATWG parse or normalization support.
pub fn canParseAbsoluteUrl(a: std.mem.Allocator, value: []const u8) !?bool {
    return standardUrl(a, value, false);
}
fn absoluteIri(a: std.mem.Allocator, value: []const u8) !?bool {
    if (try @import("../utils/unicode.zig").utf16Length(value) >= 2048) return standardUrl(a, value, false);
    var regex = try re.Regex.init(a, null, "\\[[vV][0-9a-fA-F]+\\.[^\\]]+\\]", .{});
    defer regex.deinit();
    var match = (try regex.exec(a, value, 0)) orelse return standardUrl(a, value, false);
    defer match.deinit();
    var parsed = std.Uri.parse(value) catch return standardUrl(a, value, false);
    const host_component = parsed.host orelse return standardUrl(a, value, false);
    // std.Uri.parse borrows its encoded components from value. Compare the
    // regex's first match with that parsed component, including its position:
    // a preceding literal in userinfo must not redirect the host conversion.
    const host = switch (host_component) {
        .raw, .percent_encoded => |text| text,
    };
    const offset = @intFromPtr(host.ptr) - @intFromPtr(value.ptr);
    const start = try @import("../utils/unicode.zig").utf16Length(value[0..offset]);
    const span = match.captures[0].?;
    if (span.start != start or span.end - start != try @import("../utils/unicode.zig").utf16Length(host)) return standardUrl(a, value, false);
    if (!(try authoritySyntax(a, value, false))) return null;
    parsed.host = .{ .percent_encoded = "[::1]" };
    const normalized = try @import("../transport/uri.zig").render(a, parsed);
    defer a.free(normalized);
    return standardUrl(a, normalized, false);
}
fn iri(a: std.mem.Allocator, value: []const u8, relative: bool) !?bool {
    const invalid = if (relative)
        \\[\x00-\x20\x7F\\]|%(?![0-9a-fA-F]{2})
    else
        \\[\x00-\x20<>\^`{|}\\]|%(?![0-9a-fA-F]{2})
    ;
    if (try re.matches(a, null, invalid, value, .{})) return false;
    if (relative and try re.matches(a, null,
        \\^[a-zA-Z][a-zA-Z0-9+\-.]*\/\/
    , value, .{})) return false;
    return if (relative) standardUrl(a, value, true) else absoluteIri(a, value);
}
pub fn check(a: std.mem.Allocator, name: []const u8, value: []const u8) !?bool {
    for (@import("format_patterns.zig").patterns) |pattern| if (std.mem.eql(u8, name, pattern.name)) return try re.matches(a, null, pattern.expression, value, .{ .ignoreCase = pattern.ignoreCase, .unicode = pattern.unicode });
    if (std.mem.eql(u8, name, "date")) return try isDate(a, value);
    if (std.mem.eql(u8, name, "time")) return try isTime(a, value);
    if (std.mem.eql(u8, name, "date-time")) return try isDateTime(a, value);
    if (std.mem.eql(u8, name, "hostname")) return hostname(a, value);
    if (std.mem.eql(u8, name, "idn-hostname")) return idnHostname(a, value);
    if (std.mem.eql(u8, name, "idn-email")) return idnEmail(a, value);
    if (std.mem.eql(u8, name, "url")) return standardUrl(a, value, false);
    if (std.mem.eql(u8, name, "iri")) return iri(a, value, false);
    if (std.mem.eql(u8, name, "iri-reference")) return iri(a, value, true);
    if (std.mem.eql(u8, name, "regex")) {
        var regex = re.Regex.init(a, null, value, .{ .unicode = true }) catch |err| {
            if (err == error.InvalidRegularExpression) return false;
            return err;
        };
        regex.deinit();
        return true;
    }
    return null;
}
