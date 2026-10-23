//! ECMAScript scalar string conversions used by schema coercion and provider
//! headers. Regex selects the lexical form; standard numeric codecs decode it.
const std = @import("std");
const re = @import("../regex/regexp.zig");
/// Number(value) for a decoded JSON value; null input denotes undefined.
pub fn coerceValue(a: std.mem.Allocator, value: ?std.json.Value) anyerror!f64 {
    const selected = value orelse return std.math.nan(f64);
    return switch (selected) {
        .null => 0,
        .bool => |boolean| if (boolean) @as(f64, 1) else 0,
        .integer, .float, .number_string => @import("../json/value.zig").number(selected) orelse std.math.nan(f64),
        else => try number(a, null, try @import("diagnostics.zig").formatThrownValue(a, .{ .value = selected })) orelse std.math.nan(f64),
    };
}
pub fn minimum(left: f64, right: f64) f64 {
    if (std.math.isNan(left) or std.math.isNan(right)) return std.math.nan(f64);
    if (left == 0 and right == 0) return if (std.math.signbit(left) or std.math.signbit(right)) -@as(f64, 0) else 0;
    return @min(left, right);
}
pub fn maximum(left: f64, right: f64) f64 {
    if (std.math.isNan(left) or std.math.isNan(right)) return std.math.nan(f64);
    if (left == 0 and right == 0) return if (std.math.signbit(left) and std.math.signbit(right)) -@as(f64, 0) else 0;
    return @max(left, right);
}
/// ECMAScript String(number), for display and scalar coercion. JSON callers
/// still use std.json for numeric token parsing and serialization.
pub fn toString(a: std.mem.Allocator, value: f64) ![]const u8 {
    if (std.math.isNan(value)) return "NaN";
    if (std.math.isPositiveInf(value)) return "Infinity";
    if (std.math.isNegativeInf(value)) return "-Infinity";
    if (value == 0) return "0";
    const magnitude = @abs(value);
    var buffer: [std.fmt.float.bufferSize(.decimal, f64)]u8 = undefined;
    if (magnitude >= 1e21) {
        const float = std.fmt.float;
        const tables = if (@import("builtin").mode == .small) &float.Backend64_TablesSmall else &float.Backend64_TablesFull;
        const decimal = float.binaryToDecimal(u64, @as(u64, @bitCast(value)), std.math.floatMantissaBits(f64), std.math.floatExponentBits(f64), false, tables);
        const digits: i32 = @intCast(std.math.log10_int(decimal.mantissa) + 1);
        const coefficient = float.FloatDecimal(u64){ .mantissa = decimal.mantissa, .exponent = 1 - digits, .sign = decimal.sign };
        const rendered = try float.formatDecimal(u64, &buffer, coefficient, null);
        return std.fmt.allocPrint(a, "{s}e+{d}", .{ rendered, decimal.exponent + digits - 1 });
    }
    const rendered = try std.fmt.float.render(&buffer, value, .{ .mode = if (magnitude >= 1e-6) .decimal else .scientific });
    return a.dupe(u8, rendered);
}
pub fn number(a: std.mem.Allocator, io: ?std.Io, input: []const u8) !?f64 {
    const text = try @import("unicode.zig").trimWhitespace(a, input);
    defer a.free(text);
    if (text.len == 0) return 0;
    if (try re.matches(a, io,
        \\^0(?:[xX][0-9a-fA-F]+|[bB][01]+|[oO][0-7]+)$
    , text, .{})) {
        var integer = try std.math.big.int.Managed.init(a);
        defer integer.deinit();
        const base: u8 = switch (text[1]) {
            'x', 'X' => 16,
            'b', 'B' => 2,
            'o', 'O' => 8,
            else => unreachable,
        };
        integer.setString(base, text[2..]) catch |err| {
            if (err == error.OutOfMemory) return err;
            return null;
        };
        return integer.toFloat(f64, .nearest_even)[0];
    }
    if (!(try re.matches(a, io,
        \\^[+-]?(?:Infinity|(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?)$
    , text, .{}))) return null;
    return std.fmt.parseFloat(f64, text) catch null;
}
pub fn parseFloat(a: std.mem.Allocator, io: ?std.Io, input: []const u8) !?f64 {
    const text = try @import("unicode.zig").trimWhitespace(a, input);
    defer a.free(text);
    var regex = try re.Regex.init(a, io,
        \\^[+-]?(?:Infinity|(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?)
    , .{});
    defer regex.deinit();
    var match = (try regex.exec(a, text, 0)) orelse return null;
    defer match.deinit();
    const prefix = (try match.text(a, 0)).?;
    defer a.free(prefix);
    return std.fmt.parseFloat(f64, prefix) catch null;
}
/// parseInt(text,10): regex selects the signed decimal integer prefix, while
/// the standard decimal-to-binary64 parser owns rounding/overflow and -0.
pub fn parseInt10(a: std.mem.Allocator, io: ?std.Io, input: []const u8) !?f64 {
    const text = try @import("unicode.zig").trimWhitespace(a, input);
    defer a.free(text);
    var regex = try re.Regex.init(a, io, "^[+-]?[0-9]+", .{});
    defer regex.deinit();
    var match = (try regex.exec(a, text, 0)) orelse return null;
    defer match.deinit();
    const prefix = (try match.text(a, 0)).?;
    defer a.free(prefix);
    return std.fmt.parseFloat(f64, prefix) catch null;
}
/// JavaScript parseInt with omitted radix: signed hexadecimal prefixes use16,
/// other signed decimal prefixes use10. Standard floating parsing supplies the
/// binary64 rounding/overflow after regex prefix selection.
pub fn parseIntDefault(a: std.mem.Allocator, io: ?std.Io, input: []const u8) !?f64 {
    const text = try @import("unicode.zig").trimWhitespace(a, input);
    defer a.free(text);
    var regex = try re.Regex.init(a, io, "^([+-]?)(?:0[xX]([0-9a-fA-F]+)|((?!0[xX])[0-9]+))", .{});
    defer regex.deinit();
    var match = (try regex.exec(a, text, 0)) orelse return null;
    defer match.deinit();
    if (try match.text(a, 2)) |digits| {
        defer a.free(digits);
        const sign = (try match.text(a, 1)).?;
        defer a.free(sign);
        const literal = try std.fmt.allocPrint(a, "{s}0x{s}p0", .{ sign, digits });
        defer a.free(literal);
        return std.fmt.parseFloat(f64, literal) catch null;
    }
    const prefix = (try match.text(a, 0)).?;
    defer a.free(prefix);
    return std.fmt.parseFloat(f64, prefix) catch null;
}
