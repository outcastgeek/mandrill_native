//! Source accounting assignments over retained standard JSON values.
const std = @import("std");
const j = @import("../json/value.zig");
const t = @import("../types.zig");
const Value = std.json.Value;
pub fn orZero(value: Value, key: []const u8) Value {
    const raw = j.get(value, key) orelse return j.num(0);
    return if (j.truthy(raw)) raw else j.num(0);
}
pub fn set(a: std.mem.Allocator, usage: *t.Usage, comptime name: []const u8, value: Value) !void {
    if (j.number(value)) |n| {
        @field(usage, name) = n;
        j.markPresent(usage, name);
    } else try j.retainField(a, usage, name, value);
}
pub fn sum(a: std.mem.Allocator, usage: t.Usage, comptime names: []const []const u8) !Value {
    var result: ?Value = try j.fieldValue(a, usage, names[0]);
    inline for (names[1..]) |name| result = try @import("cost.zig").addValues(a, result, try j.fieldValue(a, usage, name));
    return result orelse j.num(std.math.nan(f64));
}
pub fn total(a: std.mem.Allocator, usage: *t.Usage) !void {
    try set(a, usage, "totalTokens", try sum(a, usage.*, &.{ "input", "output", "cacheRead", "cacheWrite" }));
}
pub fn number(a: std.mem.Allocator, value: ?Value) !f64 {
    return @import("numbers.zig").coerceValue(a, value);
}
