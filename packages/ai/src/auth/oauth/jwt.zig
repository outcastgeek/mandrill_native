//! Compact JWS framing and the pinned Codex atob/JSON.parse boundary.
//! Base64, Unicode conversion and JSON decoding stay in std.
//! This decodes untrusted claims only. It does not verify the signature,
//! issuer, audience, expiration or the identity represented by those claims.
const std = @import("std");
pub fn payload(a: std.mem.Allocator, compact: []const u8) !std.json.Value {
    var parts = std.mem.splitScalar(u8, compact, '.');
    _ = parts.next() orelse return error.InvalidCompactJws;
    const encoded = parts.next() orelse return error.InvalidCompactJws;
    _ = parts.next() orelse return error.InvalidCompactJws;
    if (parts.next() != null) return error.InvalidCompactJws;
    const decoded = try @import("../../utils/base64.zig").decodeAtob(a, encoded);
    defer a.free(decoded);
    // atob yields a binary string: each decoded byte becomes U+0000..U+00FF,
    // rather than being interpreted as a byte of UTF-8 source JSON.
    const units = try a.alloc(u16, decoded.len);
    defer a.free(units);
    for (decoded, units) |byte, *unit| unit.* = std.mem.nativeToLittle(u16, byte);
    const text = try std.unicode.utf16LeToUtf8Alloc(a, units);
    defer a.free(text);
    return @import("../../json/value.zig").parseFromSliceLeaky(std.json.Value, a, text, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always });
}
