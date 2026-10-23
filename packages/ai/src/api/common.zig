const std = @import("std");
const j = @import("../json/value.zig");
const t = @import("../types.zig");
pub fn rememberError(a: std.mem.Allocator, error_message: ?*?[]const u8, storage: ?*t.ErrorMessageStorage, comptime format: []const u8, args: anytype) !void {
    if (error_message) |slot| {
        const message = try std.fmt.allocPrint(a, format, args);
        if (storage) |owner| {
            defer a.free(message);
            slot.* = try owner.copy(message);
        } else slot.* = message;
    }
}
pub fn requireApi(a: std.mem.Allocator, actual: []const u8, expected: []const u8, error_message: ?*?[]const u8, storage: ?*t.ErrorMessageStorage) !void {
    if (std.mem.eql(u8, actual, expected)) return;
    try rememberError(a, error_message, storage, "Mismatched api: {s} expected {s}", .{ actual, expected });
    return error.MismatchedApi;
}
pub fn overlay(a: std.mem.Allocator, target: *std.json.Value, source: ?std.json.Value) !void {
    if (source) |v| if (v == .object) {
        var it = v.object.iterator();
        while (it.next()) |e| try j.put(a, target, e.key_ptr.*, e.value_ptr.*);
    };
}
pub fn compatBool(model: t.Model, key: []const u8, default: bool) bool {
    return if (model.compat) |v| (j.boolean(j.get(v, key) orelse return default) orelse default) else default;
}
pub fn compatText(model: t.Model, key: []const u8, default: []const u8) []const u8 {
    return if (model.compat) |v| j.field(v, key) orelse default else default;
}
pub fn extra(options: t.StreamOptions, key: []const u8) ?std.json.Value {
    return if (options.extra) |v| j.get(v, key) else null;
}
pub fn cacheRetention(options: t.StreamOptions) t.CacheRetention {
    if (options.cacheRetention) |retention| return retention;
    return if (std.mem.eql(u8, options.env.get("PI_CACHE_RETENTION") orelse "", "long")) .long else .short;
}
pub fn sampling(a: std.mem.Allocator, payload: *std.json.Value, model: t.Model, options: t.StreamOptions) !void {
    try overlay(a, payload, model.samplingParams);
    if (model.samplingParamsByThinkingLevel) |levels| try overlay(a, payload, j.get(levels, @tagName(@import("../utils/thinking.zig").clampThinkingLevel(model, options.reasoning orelse .off))));
    try overlay(a, payload, options.samplingParams);
}
pub fn imageDataUri(a: std.mem.Allocator, image: t.ImageContent) ![]const u8 {
    // A typed URI owns the scheme. The opaque data path is emitted by a writer.
    var path = std.Io.Writer.Allocating.init(a);
    defer path.deinit();
    try path.writer.writeAll(image.mimeType);
    try path.writer.writeAll(";base64,");
    try path.writer.writeAll(image.data);
    const uri = std.Uri{ .scheme = "data", .path = .{ .percent_encoded = path.written() } };
    var output = std.Io.Writer.Allocating.init(a);
    defer output.deinit();
    try uri.writeToStream(&output.writer, .{ .scheme = true, .path = true });
    return output.toOwnedSlice();
}
pub fn num(value: std.json.Value, key: []const u8) f64 {
    return j.number(j.get(value, key) orelse return 0) orelse 0;
}
/// Numeric source fields selected by `value || 0`, including signed zero/NaN.
pub fn numOrZero(value: std.json.Value, key: []const u8) f64 {
    const raw = j.get(value, key) orelse return 0;
    return if (j.truthy(raw)) j.number(raw) orelse 0 else 0;
}
/// Number-keyed JavaScript maps use SameValueZero; no integer conversion is
/// involved. A separate NaN payload represents an omitted numeric wire index.
pub fn numberMapKey(number: f64) u64 {
    if (std.math.isNan(number)) return 0x7ff8000000000000;
    return @bitCast(if (number == 0) @as(f64, 0) else number);
}
pub fn numericIndex(value: ?std.json.Value) u64 {
    return numberMapKey(j.number(value orelse return 0x7ff8000000000001) orelse return 0x7ff8000000000001);
}
pub fn array(value: std.json.Value, key: []const u8) []const std.json.Value {
    const v = j.get(value, key) orelse return &.{};
    return if (v == .array) v.array.items else &.{};
}
pub fn is(model: t.Model, provider: []const u8) bool {
    return std.mem.eql(u8, model.provider, provider);
}
pub fn has(model: t.Model, needle: []const u8) bool {
    return std.mem.find(u8, model.baseUrl, needle) != null;
}
