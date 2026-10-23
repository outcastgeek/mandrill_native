const std = @import("std");
const t = @import("../types.zig");
const auth = @import("../auth/types.zig");
const re = @import("../regex/regexp.zig");
pub const UNSUPPORTED_PROXY_PROTOCOL_MESSAGE = "Unsupported proxy protocol. SOCKS and PAC proxy URLs are not supported; use an HTTP or HTTPS proxy URL.";
fn userInfo(a: std.mem.Allocator, component: ?std.Uri.Component) ![]const u8 {
    const value = component orelse return "";
    if (value == .percent_encoded and try re.matches(a, null, "%(?![0-9a-fA-F]{2})", value.percent_encoded, .{})) return error.InvalidProxyCredentials;
    const decoded = try value.toRawMaybeAlloc(a);
    if (!std.unicode.utf8ValidateSlice(decoded)) return error.InvalidProxyCredentials;
    return decoded;
}
/// Populate only the public standard proxy carrier. URI decoding and Base64
/// encoding use the standard codecs; no fixed-size credential scratch buffer.
/// a is the proxy region and must outlive the configured client's requests.
pub fn configureHttpProxy(client: *std.http.Client, a: std.mem.Allocator, url: std.Uri) !void {
    const protocol: std.http.Client.Protocol = if (std.ascii.eqlIgnoreCase(url.scheme, "https")) .tls else if (std.ascii.eqlIgnoreCase(url.scheme, "http")) .plain else return error.UnsupportedProxyProtocol;
    var host_buffer: [std.Io.net.HostName.max_len]u8 = undefined;
    const host = try std.Io.net.HostName.fromUri(url, &host_buffer);
    const user = try userInfo(a, url.user);
    const password = try userInfo(a, url.password);
    const authorization = if (user.len > 0 or password.len > 0) value: {
        const credentials = try std.mem.concat(a, u8, &.{ user, ":", password });
        defer a.free(credentials);
        break :value try std.fmt.allocPrint(a, "Basic {b64}", .{credentials});
    } else null;
    const proxy = try a.create(std.http.Client.Proxy);
    proxy.* = .{ .protocol = protocol, .host = .{ .bytes = try a.dupe(u8, host.bytes) }, .authorization = authorization, .port = url.port orelse (if (protocol == .tls) @as(u16, 443) else 80), .supports_connect = false };
    client.http_proxy = proxy;
    client.https_proxy = proxy;
}
fn variable(a: std.mem.Allocator, io: std.Io, context: ?auth.AuthContext, env: t.ProviderEnv, name: []const u8) !?[]const u8 {
    const lower = try std.ascii.allocLowerString(a, name);
    const upper = try std.ascii.allocUpperString(a, name);
    if (env.get(lower)) |value| if (value.len > 0) return value;
    if (env.get(upper)) |value| if (value.len > 0) return value;
    if (context) |ctx| {
        if (try ctx.env(a, io, lower)) |value| if (value.len > 0) return value;
        if (try ctx.env(a, io, upper)) |value| if (value.len > 0) return value;
    }
    return null;
}
fn defaultPort(scheme: []const u8) u16 {
    if (std.ascii.eqlIgnoreCase(scheme, "https") or std.ascii.eqlIgnoreCase(scheme, "wss")) return 443;
    if (std.ascii.eqlIgnoreCase(scheme, "http") or std.ascii.eqlIgnoreCase(scheme, "ws")) return 80;
    if (std.ascii.eqlIgnoreCase(scheme, "ftp")) return 21;
    if (std.ascii.eqlIgnoreCase(scheme, "gopher")) return 70;
    return 0;
}
fn stripBrackets(raw: []const u8) []const u8 {
    return if (raw.len >= 2 and raw[0] == '[' and raw[raw.len - 1] == ']') raw[1 .. raw.len - 1] else raw;
}
fn decimalPort(a: std.mem.Allocator, io: std.Io, text: []const u8) !?f64 {
    var regex = try re.Regex.init(a, io, "^\\s*([+-]?[0-9]+)", .{});
    defer regex.deinit();
    var match = (try regex.exec(a, text, 0)) orelse return null;
    defer match.deinit();
    const digits = (try match.text(a, 1)).?;
    defer a.free(digits);
    return std.fmt.parseFloat(f64, digits) catch null;
}
const NoProxyEntry = struct { host: []const u8, port: f64 = 0 };
fn noProxyEntry(a: std.mem.Allocator, io: std.Io, entry: []const u8) !NoProxyEntry {
    // This is the pinned NO_PROXY entry grammar, not URI authority parsing.
    // Ports use parseInt's decimal-prefix semantics, including signs and
    // trailing text; the standard numeric codec decodes the regex capture.
    if (std.mem.startsWith(u8, entry, "[")) if (std.mem.findScalar(u8, entry, ']')) |end| {
        const rest = entry[end + 1 ..];
        return .{ .host = entry[1..end], .port = if (std.mem.startsWith(u8, rest, ":")) (try decimalPort(a, io, rest[1..])) orelse 0 else 0 };
    };
    if (std.mem.countScalar(u8, entry, ':') == 1) {
        const colon = std.mem.findScalar(u8, entry, ':').?;
        if (try decimalPort(a, io, entry[colon + 1 ..])) |port| return .{ .host = entry[0..colon], .port = port };
    }
    return .{ .host = entry };
}
fn bypass(a: std.mem.Allocator, io: std.Io, target: std.Uri, no_proxy: []const u8) !bool {
    if (std.mem.eql(u8, no_proxy, "*")) return true;
    const hostname = stripBrackets(try (target.host orelse return false).toRawMaybeAlloc(a));
    const explicit_port = target.port orelse 0;
    const port = if (explicit_port != 0) explicit_port else defaultPort(target.scheme);
    var entries = try re.Regex.init(a, io, "[^,\\s]+", .{});
    defer entries.deinit();
    var offset: usize = 0;
    while (try entries.exec(a, no_proxy, offset)) |found| {
        var match = found;
        defer match.deinit();
        offset = match.captures[0].?.end;
        const entry = (try match.text(a, 0)).?;
        defer a.free(entry);
        const parsed = try noProxyEntry(a, io, entry);
        if (parsed.port != 0 and parsed.port != @as(f64, @floatFromInt(port))) continue;
        var host = stripBrackets(parsed.host);
        if (std.mem.startsWith(u8, host, "*.")) host = host[2..] else if (std.mem.startsWith(u8, host, ".") or std.mem.startsWith(u8, host, "*")) host = host[1..];
        if (host.len == 0) continue;
        if (std.ascii.eqlIgnoreCase(hostname, host)) return true;
        if (hostname.len > host.len and hostname[hostname.len - host.len - 1] == '.' and std.ascii.eqlIgnoreCase(hostname[hostname.len - host.len ..], host)) return true;
    }
    return false;
}
pub fn resolveHttpProxyUrlForTarget(a: std.mem.Allocator, io: std.Io, context: ?auth.AuthContext, env: t.ProviderEnv, target: std.Uri) !?std.Uri {
    return resolveHttpProxyUrlForTargetWithError(a, io, context, env, target, null);
}
fn invalidProxy(a: std.mem.Allocator, slot: ?*?[]const u8, text: []const u8) anyerror {
    if (slot) |out| out.* = std.fmt.allocPrint(a, "Invalid proxy URL {s}: Invalid URL", .{@import("../json/value.zig").stringify(a, text, .{}) catch |err| return err}) catch |err| return err;
    return error.InvalidProxyUrl;
}
pub fn resolveHttpProxyUrlForTargetWithError(a: std.mem.Allocator, io: std.Io, context: ?auth.AuthContext, env: t.ProviderEnv, target: std.Uri, error_message: ?*?[]const u8) !?std.Uri {
    if (target.host == null) return null;
    if (try variable(a, io, context, env, "no_proxy")) |value| if (try bypass(a, io, target, value)) return null;
    const scheme = try std.ascii.allocLowerString(a, target.scheme);
    const raw = (try variable(a, io, context, env, try std.fmt.allocPrint(a, "{s}_proxy", .{scheme}))) orelse (try variable(a, io, context, env, "all_proxy")) orelse return null;
    const text = if (std.mem.find(u8, raw, "://") != null) raw else try std.fmt.allocPrint(a, "{s}://{s}", .{ scheme, raw });
    if (try @import("../schema/formats.zig").canParseAbsoluteUrl(a, text) == false) return invalidProxy(a, error_message, text);
    const proxy = @import("uri.zig").parseHttpUrl(a, text) catch |err| {
        if (err == error.OutOfMemory or err == error.Canceled) return err;
        return invalidProxy(a, error_message, text);
    };
    if (!std.mem.eql(u8, proxy.scheme, "http") and !std.mem.eql(u8, proxy.scheme, "https")) {
        if (error_message) |out| out.* = try std.fmt.allocPrint(a, "{s} Got {s}:", .{ UNSUPPORTED_PROXY_PROTOCOL_MESSAGE, proxy.scheme });
        return error.UnsupportedProxyProtocol;
    }
    return proxy;
}
