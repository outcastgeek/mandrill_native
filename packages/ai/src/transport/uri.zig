const std = @import("std");
pub const Pair = struct { name: []const u8, value: []const u8 };
fn unreserved(byte: u8) bool {
    return std.ascii.isAlphanumeric(byte) or byte == '-' or byte == '.' or byte == '_' or byte == '~';
}
fn jsComponentSafe(byte: u8) bool {
    return unreserved(byte) or byte == '!' or byte == '\'' or byte == '(' or byte == ')' or byte == '*';
}
/// ECMAScript encodeURIComponent's safe set, separate from RFC3986 signing.
/// Standard UTF8 validation rejects lone surrogate encodings before escaping.
pub fn encodeJsComponent(a: std.mem.Allocator, value: []const u8) ![]const u8 {
    if (!std.unicode.utf8ValidateSlice(value)) return error.InvalidUriComponent;
    var output = std.Io.Writer.Allocating.init(a);
    defer output.deinit();
    try std.Uri.Component.percentEncode(&output.writer, value, jsComponentSafe);
    return output.toOwnedSlice();
}
fn formSafe(byte: u8) bool {
    return std.ascii.isAlphanumeric(byte) or byte == '*' or byte == '-' or byte == '.' or byte == '_';
}
pub fn encodeComponent(a: std.mem.Allocator, value: []const u8) ![]const u8 {
    var output = std.Io.Writer.Allocating.init(a);
    defer output.deinit();
    try std.Uri.Component.percentEncode(&output.writer, value, unreserved);
    return output.toOwnedSlice();
}
/// RFC3986 query encoding, including AWS Query/signing inputs. Form bodies
/// and URLSearchParams callers use encodeForm instead. Repeated keys retain order.
pub fn encode(a: std.mem.Allocator, pairs: []const Pair) ![]const u8 {
    var output = std.Io.Writer.Allocating.init(a);
    defer output.deinit();
    for (pairs, 0..) |pair, i| {
        if (i > 0) try output.writer.writeByte('&');
        try std.Uri.Component.percentEncode(&output.writer, pair.name, unreserved);
        try output.writer.writeByte('=');
        try std.Uri.Component.percentEncode(&output.writer, pair.value, unreserved);
    }
    return output.toOwnedSlice();
}
/// URLSearchParams accepts scalar Unicode strings. The standard UTF16 formatter
/// replaces lone surrogate units with U+FFFD before standard percent encoding.
fn formScalar(a: std.mem.Allocator, value: []const u8) ![]const u8 {
    const units = try std.unicode.wtf8ToWtf16LeAlloc(a, value);
    defer a.free(units);
    return std.fmt.allocPrint(a, "{f}", .{std.unicode.fmtUtf16Le(units)});
}
const FormComponent = struct {
    scalar: []const u8,
    pub fn format(self: @This(), writer: *std.Io.Writer) std.Io.Writer.Error!void {
        // Spaces are the form codec's '+' separators. The standard component
        // encoder performs every percent escape, including literal '+' bytes.
        var pieces = std.mem.splitScalar(u8, self.scalar, ' ');
        var first = true;
        while (pieces.next()) |piece| {
            if (!first) try writer.writeByte('+');
            first = false;
            try std.Uri.Component.percentEncode(writer, piece, formSafe);
        }
    }
};
/// Typed application/x-www-form-urlencoded fields. No encoded document is
/// rewritten and no custom percent-escape lexer/encoder is used.
pub fn encodeForm(a: std.mem.Allocator, pairs: []const Pair) ![]const u8 {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    var output = std.Io.Writer.Allocating.init(a);
    defer output.deinit();
    for (pairs, 0..) |pair, index| {
        if (index > 0) output.writer.writeByte('&') catch return error.OutOfMemory;
        const name = FormComponent{ .scalar = try formScalar(scratch, pair.name) };
        const value = FormComponent{ .scalar = try formScalar(scratch, pair.value) };
        output.writer.print("{f}={f}", .{ name, value }) catch return error.OutOfMemory;
    }
    return output.toOwnedSlice();
}
fn decodeFormComponent(a: std.mem.Allocator, encoded: []const u8) ![]const u8 {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    var output = std.Io.Writer.Allocating.init(scratch);
    var pieces = std.mem.splitScalar(u8, encoded, '+');
    var first = true;
    while (pieces.next()) |piece| {
        if (!first) output.writer.writeByte(' ') catch return error.OutOfMemory;
        first = false;
        const decoded = try (std.Uri.Component{ .percent_encoded = piece }).toRawMaybeAlloc(scratch);
        output.writer.writeAll(decoded) catch return error.OutOfMemory;
    }
    // Standard percent decoding preserves malformed escape text; standard UTF8
    // formatting supplies replacement characters for malformed decoded bytes.
    return std.fmt.allocPrint(a, "{f}", .{std.unicode.fmtUtf8(output.written())});
}
/// URLSearchParams entries before any caller-specific duplicate-key policy.
pub fn decodeFormPairs(a: std.mem.Allocator, component: std.Uri.Component) ![]const Pair {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const encoded = try formScalar(temporary.allocator(), switch (component) {
        .raw => |value| value,
        .percent_encoded => |value| value,
    });
    var pairs: std.ArrayList(Pair) = .empty;
    errdefer {
        for (pairs.items) |pair| {
            a.free(pair.name);
            a.free(pair.value);
        }
        pairs.deinit(a);
    }
    var fields = std.mem.splitScalar(u8, encoded, '&');
    while (fields.next()) |field| {
        if (field.len == 0) continue;
        var reader = std.Io.Reader.fixed(field);
        const key = (try reader.takeDelimiter('=')) orelse field;
        const name = try decodeFormComponent(a, key);
        errdefer a.free(name);
        const value = try decodeFormComponent(a, reader.buffered());
        errdefer a.free(value);
        try pairs.append(a, .{ .name = name, .value = value });
    }
    return pairs.toOwnedSlice(a);
}
pub fn appendPath(a: std.mem.Allocator, base: []const u8, segments: []const []const u8) !std.Uri {
    var uri = try std.Uri.parse(base);
    var path = std.Io.Writer.Allocating.init(a);
    defer path.deinit();
    try uri.path.formatPath(&path.writer);
    if (path.written().len == 0 or path.written()[path.written().len - 1] != '/') try path.writer.writeByte('/');
    for (segments, 0..) |segment, i| {
        if (i > 0) try path.writer.writeByte('/');
        try std.Uri.Component.percentEncode(&path.writer, segment, unreserved);
    }
    uri.path = .{ .percent_encoded = try path.toOwnedSlice() };
    return uri;
}
pub fn render(a: std.mem.Allocator, uri: std.Uri) ![]const u8 {
    var out = std.Io.Writer.Allocating.init(a);
    defer out.deinit();
    try uri.format(&out.writer);
    return out.toOwnedSlice();
}
fn urlPathByte(byte: u8) bool {
    return byte > 0x20 and byte < 0x7f and std.mem.findScalar(u8, "\"#<>?^`{}", byte) == null;
}
fn urlUserinfoByte(byte: u8) bool {
    return urlPathByte(byte) and std.mem.findScalar(u8, "/:;=@[\\]|", byte) == null;
}
fn urlQueryByte(byte: u8) bool {
    return byte > 0x20 and byte < 0x7f and std.mem.findScalar(u8, "\"#<>'", byte) == null;
}
fn urlFragmentByte(byte: u8) bool {
    return byte > 0x20 and byte < 0x7f and std.mem.findScalar(u8, "\"<>`", byte) == null;
}
fn urlComponent(a: std.mem.Allocator, component: std.Uri.Component, comptime safe: fn (u8) bool) !std.Uri.Component {
    const scalar = try formScalar(a, switch (component) {
        .raw, .percent_encoded => |value| value,
    });
    var writer = std.Io.Writer.Allocating.init(a);
    defer writer.deinit();
    // Existing escapes remain escapes. The standard encoder handles all byte
    // emission; the predicate supplies this URL component's encode set.
    try std.Uri.Component.percentEncode(&writer.writer, scalar, safe);
    return .{ .percent_encoded = try writer.toOwnedSlice() };
}
/// HTTP URL preprocessing must precede RFC3986 authority parsing: a
/// backslash before the query/fragment is an HTTP path separator, including
/// when it is the first character after the host. Query/fragment text is
/// deliberately excluded from that replacement.
fn httpUrlInput(a: std.mem.Allocator, text: []const u8, base_scheme: ?[]const u8) ![]const u8 {
    const scalar = try formScalar(a, text);
    var trim = try @import("../regex/regexp.zig").Regex.init(a, null, "^[\\x00-\\x20]*([\\s\\S]*?)[\\x00-\\x20]*(?![\\s\\S])", .{});
    defer trim.deinit();
    var trimmed = (try trim.exec(a, scalar, 0)).?;
    defer trimmed.deinit();
    var selected = (try trimmed.text(a, 1)).?;
    for ([_][]const u8{ "\t", "\n", "\r" }) |character| selected = try std.mem.replaceOwned(u8, a, selected, character, "");
    const end = std.mem.findAny(u8, selected, "?#") orelse selected.len;
    const http = std.ascii.startsWithIgnoreCase(selected, "https:") or std.ascii.startsWithIgnoreCase(selected, "http:");
    const inherited_http = base_scheme != null and !http and (std.Uri.parse(selected) catch null) == null;
    var prepared = if (http or inherited_http) try std.mem.concat(a, u8, &.{ try std.mem.replaceOwned(u8, a, selected[0..end], "\\", "/"), selected[end..] }) else selected;
    if (http) {
        var scheme = try @import("../regex/regexp.zig").Regex.init(a, null, "^(https?):(/*)([\\s\\S]*)(?![\\s\\S])", .{ .ignoreCase = true });
        defer scheme.deinit();
        var parts = (try scheme.exec(a, prepared, 0)).?;
        defer parts.deinit();
        const name = (try parts.text(a, 1)).?;
        const slashes = (try parts.text(a, 2)).?;
        const tail = (try parts.text(a, 3)).?;
        // A same-special-scheme URL without two authority slashes remains
        // relative to its base. Standard resolution owns the remaining path.
        const same = if (base_scheme) |base| std.ascii.eqlIgnoreCase(name, base) else false;
        prepared = if (same and slashes.len < 2) relative: {
            // resolveInPlace reparses its input. Disambiguate a nonempty
            // relative path so a colon in its first segment cannot become
            // another scheme after the original matching scheme is removed.
            if (slashes.len == 0 and tail.len > 0 and tail[0] != '?' and tail[0] != '#') break :relative try std.fmt.allocPrint(a, "./{s}", .{tail});
            break :relative try std.mem.concat(a, u8, &.{ slashes, tail });
        } else try std.fmt.allocPrint(a, "{s}://{s}", .{ name, tail });
    } else if (inherited_http and std.mem.startsWith(u8, prepared, "//")) {
        prepared = try std.fmt.allocPrint(a, "//{s}", .{std.mem.trimStart(u8, prepared, "/")});
    }
    return prepared;
}
pub fn parseHttpUrl(a: std.mem.Allocator, text: []const u8) !std.Uri {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    const prepared = try httpUrlInput(scratch, text, null);
    if (std.ascii.startsWithIgnoreCase(prepared, "https:") or std.ascii.startsWithIgnoreCase(prepared, "http:")) {
        if (try @import("../schema/formats.zig").canParseAbsoluteUrl(scratch, prepared)) |known| if (!known) return error.InvalidFormat;
    }
    const http = std.ascii.startsWithIgnoreCase(prepared, "https:") or std.ascii.startsWithIgnoreCase(prepared, "http:");
    return normalizeHttpUrl(a, if (http) try @import("host.zig").parseUri(a, prepared) else try std.Uri.parse(prepared));
}
/// Relative HTTP input receives the same scalar/control/backslash preparation
/// before standard resolution. Returned component storage belongs to a.
pub fn resolveHttpUrl(a: std.mem.Allocator, base: std.Uri, text: []const u8) !std.Uri {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    const special = std.ascii.eqlIgnoreCase(base.scheme, "http") or std.ascii.eqlIgnoreCase(base.scheme, "https");
    const prepared = try httpUrlInput(scratch, text, if (special) base.scheme else null);
    var base_path = std.Io.Writer.Allocating.init(scratch);
    defer base_path.deinit();
    try base.path.formatPath(&base_path.writer);
    const copied = std.math.mul(usize, prepared.len, 2) catch return error.OutOfMemory;
    const extra = std.math.add(usize, base_path.written().len, 2) catch return error.OutOfMemory;
    const capacity = std.math.add(usize, copied, extra) catch return error.OutOfMemory;
    // A newly supplied HTTP authority passes through the host adapter before
    // standard resolution, including empty ports and long Unicode host text.
    const has_authority = std.mem.startsWith(u8, prepared, "//") or std.ascii.startsWithIgnoreCase(prepared, "http:") or std.ascii.startsWithIgnoreCase(prepared, "https:");
    var replaced_host: ?std.Uri.Component = null;
    const location = if (has_authority) masked: {
        var parsed = try @import("host.zig").parseUri(scratch, prepared);
        replaced_host = parsed.host;
        parsed.host = .{ .percent_encoded = "host.invalid" };
        break :masked try render(scratch, parsed);
    } else prepared;
    const required = std.math.add(usize, location.len, extra) catch return error.OutOfMemory;
    var memory = try scratch.alloc(u8, @max(capacity, required));
    @memcpy(memory[0..location.len], location);
    var resolved = try std.Uri.resolveInPlace(base, location.len, &memory);
    if (replaced_host) |host| resolved.host = host;
    if (std.ascii.eqlIgnoreCase(resolved.scheme, "http") or std.ascii.eqlIgnoreCase(resolved.scheme, "https")) return parseHttpUrl(a, try render(scratch, resolved));
    return std.Uri.parse(try render(a, resolved));
}
fn httpHost(a: std.mem.Allocator, component: std.Uri.Component) !std.Uri.Component {
    var arena = std.heap.ArenaAllocator.init(a);
    defer arena.deinit();
    return (try @import("host.zig").parse(arena.allocator(), component)).component(a);
}
/// Standard URI parsing/rendering/resolution with the HTTP URL component
/// encode sets. SDK string assembly is performed before this transformation.
pub fn normalizeHttpUrl(a: std.mem.Allocator, input: std.Uri) !std.Uri {
    var value = input;
    value.scheme = try std.ascii.allocLowerString(a, value.scheme);
    if (value.host) |host| value.host = if (std.mem.eql(u8, value.scheme, "http") or std.mem.eql(u8, value.scheme, "https")) try httpHost(a, host) else switch (host) {
        .raw => |text| .{ .raw = try std.ascii.allocLowerString(a, text) },
        .percent_encoded => |text| .{ .percent_encoded = try std.ascii.allocLowerString(a, text) },
    };
    if (std.mem.eql(u8, value.scheme, "http") or std.mem.eql(u8, value.scheme, "https")) {
        // std.Uri already splits the last '@' and the first password colon.
        // Apply URL's userinfo encode set through its standard byte encoder.
        if (value.password) |password| {
            const encoded = try urlComponent(a, password, urlUserinfoByte);
            value.password = if (encoded.percent_encoded.len == 0) null else encoded;
        }
        if (value.user) |user| {
            const encoded = try urlComponent(a, user, urlUserinfoByte);
            value.user = if (encoded.percent_encoded.len == 0 and value.password == null) null else encoded;
        } else if (value.password != null) value.user = .{ .percent_encoded = "" };
    }
    if (value.port) |port| if ((std.mem.eql(u8, value.scheme, "http") and port == 80) or (std.mem.eql(u8, value.scheme, "https") and port == 443)) {
        value.port = null;
    };
    const raw_path = switch (value.path) {
        .raw, .percent_encoded => |text| text,
    };
    const path = if (std.mem.eql(u8, value.scheme, "http") or std.mem.eql(u8, value.scheme, "https")) try std.mem.replaceOwned(u8, a, raw_path, "\\", "/") else raw_path;
    var segments: std.ArrayList([]const u8) = .empty;
    defer segments.deinit(a);
    var split = std.mem.splitScalar(u8, path, '/');
    while (split.next()) |segment| {
        const single = try @import("../regex/regexp.zig").matches(a, null, "^(?:\\.|%2e)(?![\\s\\S])", segment, .{ .ignoreCase = true });
        const double = !single and try @import("../regex/regexp.zig").matches(a, null, "^(?:\\.|%2e){2}(?![\\s\\S])", segment, .{ .ignoreCase = true });
        try segments.append(a, if (single) "." else if (double) ".." else segment);
    }
    const normalized_path = try std.mem.join(a, "/", segments.items);
    value.path = try urlComponent(a, .{ .percent_encoded = if (normalized_path.len == 0) "/" else normalized_path }, urlPathByte);
    if (value.query) |query| value.query = try urlComponent(a, query, urlQueryByte);
    if (value.fragment) |fragment| value.fragment = try urlComponent(a, fragment, urlFragmentByte);
    // Dot-segment removal remains standard. A temporary host avoids imposing
    // std.Uri's DNS length bound on an already validated WHATWG host.
    var standard = value;
    if (value.host != null) standard.host = .{ .percent_encoded = "host.invalid" };
    const encoded = try render(a, standard);
    var storage = try a.dupe(u8, encoded);
    var resolved = try std.Uri.resolveInPlace(standard, storage.len, &storage);
    resolved.host = value.host;
    return resolved;
}
pub fn queryValue(a: std.mem.Allocator, query: std.Uri.Component, expected: []const u8) !?[]const u8 {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    // Scalar conversion precedes percent decoding, as with URLSearchParams:
    // a literal lone surrogate becomes one replacement character, whereas an
    // ASCII percent sequence is decoded later by the standard URI codec.
    const encoded = try formScalar(scratch, switch (query) {
        .percent_encoded => |value| value,
        .raw => |value| value,
    });
    const expected_name = try formScalar(scratch, expected);
    var reader = std.Io.Reader.fixed(encoded);
    while (try reader.takeDelimiter('&')) |field| {
        var item = std.Io.Reader.fixed(field);
        const name = (try item.takeDelimiter('=')) orelse continue;
        const decoded_name = try decodeFormComponent(scratch, name);
        if (std.mem.eql(u8, decoded_name, expected_name)) return try decodeFormComponent(a, item.buffered());
    }
    return null;
}

/// The OAuth paste parser observes only URL validity and searchParams. Network
/// special schemes share HTTP's host/input algorithm; the scheme's default
/// port has no effect on this query projection. Other components remain in
/// the standard URI tree and retain the schema's scheme-specific guard.
pub fn absoluteUrlQuery(a: std.mem.Allocator, text: []const u8) !?std.Uri.Component {
    const scalar = try httpUrlInput(a, text, null);
    const re = @import("../regex/regexp.zig");
    var expression = try re.Regex.init(a, null, "^([A-Za-z][A-Za-z0-9+.-]*):", .{});
    defer expression.deinit();
    var matched = (try expression.exec(a, scalar, 0)) orelse return error.InvalidFormat;
    defer matched.deinit();
    const scheme = (try matched.text(a, 1)).?;
    defer a.free(scheme);
    if (std.ascii.eqlIgnoreCase(scheme, "http") or std.ascii.eqlIgnoreCase(scheme, "https")) return (try parseHttpUrl(a, scalar)).query;
    if (std.ascii.eqlIgnoreCase(scheme, "ftp") or std.ascii.eqlIgnoreCase(scheme, "ws") or std.ascii.eqlIgnoreCase(scheme, "wss"))
        return (try parseHttpUrl(a, try std.mem.concat(a, u8, &.{ "http:", scalar[scheme.len + 1 ..] }))).query;
    var prepared = scalar;
    if (std.ascii.eqlIgnoreCase(scheme, "file")) {
        const end = std.mem.findAny(u8, prepared, "?#") orelse prepared.len;
        prepared = try std.mem.concat(a, u8, &.{ try std.mem.replaceOwned(u8, a, prepared[0..end], "\\", "/"), prepared[end..] });
        // The file-host state treats a Windows drive as the first path
        // segment rather than as a host followed by an empty port.
        var drive_regex = try re.Regex.init(a, null,
            \\^file://([A-Za-z])[:|]([/?#][\s\S]*|)(?![\s\S])
        , .{ .ignoreCase = true });
        defer drive_regex.deinit();
        if (try drive_regex.exec(a, prepared, 0)) |found| {
            var drive = found;
            defer drive.deinit();
            const letter = (try drive.text(a, 1)).?;
            defer a.free(letter);
            const tail = (try drive.text(a, 2)).?;
            defer a.free(tail);
            prepared = try std.fmt.allocPrint(a, "file:///{s}:{s}", .{ letter, tail });
        }
    }
    if (try @import("../schema/formats.zig").canParseAbsoluteUrl(a, prepared)) |valid| if (!valid) return error.InvalidFormat;
    const parsed = std.Uri.parse(prepared) catch try @import("host.zig").parseUri(a, prepared);
    return parsed.query;
}
