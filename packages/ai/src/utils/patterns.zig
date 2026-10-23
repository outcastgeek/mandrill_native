//! Pinned provider-error expressions use the approved standalone ECMAScript
//! regex engine. Matching reads message text and never parses JSON bodies.
const std = @import("std");
const re = @import("../regex/regexp.zig");
fn any(a: std.mem.Allocator, io: ?std.Io, expressions: []const []const u8, text: []const u8) !bool {
    for (expressions) |expression| if (try re.matches(a, io, expression, text, .{ .ignoreCase = true })) return true;
    return false;
}
pub fn terminalLimit(a: std.mem.Allocator, io: ?std.Io, text: []const u8) !bool {
    return any(a, io, @import("retry.zig").limitPatterns, text);
}
pub fn codexHttpRetryable(a: std.mem.Allocator, io: std.Io, status: u16, text: []const u8) !bool {
    if (status == 429 and try re.matches(a, io, "GoUsageLimitError|FreeUsageLimitError|Monthly usage limit reached|available balance|insufficient_quota|out of budget|quota exceeded|billing", text, .{ .ignoreCase = true })) return false;
    if (status == 429 or status == 500 or status == 502 or status == 503 or status == 504) return true;
    return re.matches(a, io, "rate.?limit|overloaded|service.?unavailable|upstream.?connect|connection.?refused", text, .{ .ignoreCase = true });
}
pub fn retryable(a: std.mem.Allocator, io: ?std.Io, text: []const u8) !bool {
    return any(a, io, @import("retry.zig").retryPatterns, text);
}
pub fn nonOverflow(a: std.mem.Allocator, io: ?std.Io, text: []const u8) !bool {
    return any(a, io, @import("overflow.zig").nonOverflowPatterns, text);
}
pub fn overflow(a: std.mem.Allocator, io: ?std.Io, text: []const u8) !bool {
    return any(a, io, @import("overflow.zig").overflowPatterns, text);
}
pub fn cerebrasBodyless(a: std.mem.Allocator, io: ?std.Io, text: []const u8) !bool {
    return re.matches(a, io,
        \\^4(?:00|13)\s*(?:status code)?\s*\(no body\)
    , text, .{ .ignoreCase = true });
}
