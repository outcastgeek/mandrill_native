//! Canonical base-ten integer representation with storage derived from T's
//! bit width. No locale, heap allocation or runtime numeric policy is used.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const Issue = @import("schema_issue.zig").Issue;
const PureCodec = @import("codec.zig").PureCodec;
const deadlines = @import("../deadline.zig");

/// Specialize validated decimal text for the selected numeric value type.
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// Returns (`type`): validated decimal-text representation for the selected numeric type.
pub fn DecimalText(comptime T: type) type {
    const info = @typeInfo(T);
    if (info != .int) @compileError("DecimalText requires an integer type");
    const sign_bytes: usize = if (info.int.signedness == .signed) 1 else 0;
    // Every base-ten digit conveys more than one bit of range, so bit width
    // plus a possible sign is a strict upper bound for formatted bytes.
    const maximum_text_bytes = @bitSizeOf(T) + sign_bytes;
    return struct {
        const Self = @This();
        buffer: [maximum_text_bytes]u8,
        len: usize,

        /// Borrow the represented byte slice.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`[]const u8`): A borrowed slice; keep the owner alive while using it.
        pub fn slice(self: *const Self) []const u8 {
            return self.buffer[0..self.len];
        }

        /// Format the validated numeric value as decimal text.
        /// Parameters:
        /// - `value: T`: validated numeric value formatted as decimal text.
        /// Returns (`Self`): decimal representation of the validated numeric value.
        pub fn format(value: T) Self {
            var result: Self = .{ .buffer = undefined, .len = 0 };
            const written = std.fmt.bufPrint(&result.buffer, "{d}", .{value}) catch unreachable;
            result.len = written.len;
            return result;
        }
    };
}

/// Rejects signs on nonnegative values, leading zeroes and any other spelling
/// that does not round-trip to the one stable decimal representation.
///
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// Returns (`PureCodec(T, DecimalText(T))`): A typed codec or schema preserving the declared decoded value, representation, and environments.
pub fn integer(comptime T: type) PureCodec(T, DecimalText(T)) {
    const Text = DecimalText(T);
    return PureCodec(T, Text).init(struct {
        fn decode(encoded: *const Text) outcomes.Result(T, Issue) {
            if (encoded.len > encoded.buffer.len)
                return .{ .err = Issue.pure(.decode, .invalid_number) };
            const candidate = encoded.slice();
            if (candidate.len == 0) return .{ .err = Issue.pure(.decode, .invalid_number) };
            const value = std.fmt.parseInt(T, candidate, 10) catch return .{ .err = Issue.pure(.decode, .invalid_number) };
            const canonical = Text.format(value);
            if (!std.mem.eql(u8, canonical.slice(), candidate))
                return .{ .err = Issue.pure(.decode, .invalid_number) };
            return .{ .ok = value };
        }
    }.decode, struct {
        fn encode(value: *const T) outcomes.Result(Text, Issue) {
            return .{ .ok = Text.format(value.*) };
        }
    }.encode);
}

/// Wall-clock instants are encoded as canonical signed epoch nanoseconds.
/// This never substitutes a process-local monotonic timestamp for durable data.
pub const wall_instant = PureCodec(deadlines.WallInstant, DecimalText(i128)).init(struct {
    fn decode(encoded: *const DecimalText(i128)) outcomes.Result(deadlines.WallInstant, Issue) {
        return switch (integer(i128).decodeDirect(encoded)) {
            .ok => |value| .{ .ok = .{ .unix_nanoseconds = value } },
            .err => |issue| .{ .err = issue },
        };
    }
}.decode, struct {
    fn encode(value: *const deadlines.WallInstant) outcomes.Result(DecimalText(i128), Issue) {
        return integer(i128).encodeDirect(&value.unix_nanoseconds);
    }
}.encode);

/// Fixed-scale decimal arithmetic keeps the exact signed coefficient. Scale
/// is part of the Zig type and therefore part of the schema identity.
///
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// - `fractional_digits: comptime usize`: Compile-time the fractional digits input required by this typed
///   operation.
/// Returns (`type`): signed coefficient paired with its compile-time fractional scale.
pub fn FixedDecimal(comptime T: type, comptime fractional_digits: usize) type {
    const info = @typeInfo(T);
    if (info != .int or info.int.signedness != .signed)
        @compileError("fixed decimal coefficients require a signed integer type");
    return struct {
        /// Number of fractional decimal digits fixed at comptime.
        pub const scale = fractional_digits;
        coefficient: T,
    };
}

/// Specialize fixed-scale decimal text for a numeric type and fractional-digit count.
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// - `fractional_digits: comptime usize`: Compile-time the fractional digits input required by this typed
///   operation.
/// Returns (`type`): validated decimal text for the selected fixed-scale value.
pub fn FixedDecimalText(comptime T: type, comptime fractional_digits: usize) type {
    const info = @typeInfo(T);
    if (info != .int or info.int.signedness != .signed)
        @compileError("fixed decimal coefficients require a signed integer type");
    const sign_bytes: usize = 1;
    const separator_bytes: usize = if (fractional_digits == 0) 0 else 1;
    const leading_zero_bytes: usize = if (fractional_digits == 0) 0 else 1;
    const maximum_text_bytes = @max(@bitSizeOf(T), fractional_digits) +
        sign_bytes + separator_bytes + leading_zero_bytes;
    return struct {
        const Self = @This();
        buffer: [maximum_text_bytes]u8,
        len: usize,

        /// Borrow the represented byte slice.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`[]const u8`): A borrowed slice; keep the owner alive while using it.
        pub fn slice(self: *const Self) []const u8 {
            return self.buffer[0..self.len];
        }

        /// Format the validated numeric value as decimal text.
        /// Parameters:
        /// - `value: FixedDecimal(T, fractional_digits)`: validated numeric value formatted as decimal text.
        /// Returns (`Self`): decimal representation of the fixed-decimal value with its configured scale.
        pub fn format(value: FixedDecimal(T, fractional_digits)) Self {
            var result: Self = .{ .buffer = undefined, .len = 0 };
            var digits_buffer: [@bitSizeOf(T)]u8 = undefined;
            const magnitude: std.meta.Int(.unsigned, @bitSizeOf(T)) = @abs(value.coefficient);
            const digits = std.fmt.bufPrint(&digits_buffer, "{d}", .{magnitude}) catch unreachable;
            var cursor: usize = 0;
            if (value.coefficient < 0) {
                result.buffer[cursor] = '-';
                cursor += sign_bytes;
            }
            if (fractional_digits == 0) {
                @memcpy(result.buffer[cursor..][0..digits.len], digits);
                cursor += digits.len;
            } else if (digits.len > fractional_digits) {
                const integral_digits = digits.len - fractional_digits;
                @memcpy(result.buffer[cursor..][0..integral_digits], digits[0..integral_digits]);
                cursor += integral_digits;
                result.buffer[cursor] = '.';
                cursor += separator_bytes;
                @memcpy(result.buffer[cursor..][0..fractional_digits], digits[integral_digits..]);
                cursor += fractional_digits;
            } else {
                result.buffer[cursor] = '0';
                cursor += leading_zero_bytes;
                result.buffer[cursor] = '.';
                cursor += separator_bytes;
                const padding = fractional_digits - digits.len;
                @memset(result.buffer[cursor..][0..padding], '0');
                cursor += padding;
                @memcpy(result.buffer[cursor..][0..digits.len], digits);
                cursor += digits.len;
            }
            result.len = cursor;
            return result;
        }
    };
}

/// Construct a fixed-scale decimal codec.
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// - `fractional_digits: comptime usize`: Compile-time the fractional digits input required by this typed
///   operation.
/// Returns (`PureCodec(FixedDecimal(T, fractional_digits), FixedDecimalText(T, fractional_digits))`): A typed codec or schema preserving the declared decoded value, representation, and environments.
pub fn fixedDecimal(comptime T: type, comptime fractional_digits: usize) PureCodec(FixedDecimal(T, fractional_digits), FixedDecimalText(T, fractional_digits)) {
    const Value = FixedDecimal(T, fractional_digits);
    const Text = FixedDecimalText(T, fractional_digits);
    return PureCodec(Value, Text).init(struct {
        fn decode(encoded: *const Text) outcomes.Result(Value, Issue) {
            if (encoded.len == 0 or encoded.len > encoded.buffer.len)
                return .{ .err = Issue.pure(.decode, .invalid_number) };
            const coefficient_sign_bytes: usize = 1;
            var coefficient_text: [@max(@bitSizeOf(T), fractional_digits) + coefficient_sign_bytes]u8 = undefined;
            var count: usize = 0;
            for (encoded.slice()) |character| {
                if (character == '.') continue;
                if (count == coefficient_text.len)
                    return .{ .err = Issue.pure(.decode, .invalid_number) };
                coefficient_text[count] = character;
                count += 1;
            }
            const coefficient = std.fmt.parseInt(T, coefficient_text[0..count], 10) catch
                return .{ .err = Issue.pure(.decode, .invalid_number) };
            const value: Value = .{ .coefficient = coefficient };
            const canonical = Text.format(value);
            if (!std.mem.eql(u8, encoded.slice(), canonical.slice()))
                return .{ .err = Issue.pure(.decode, .invalid_number) };
            return .{ .ok = value };
        }
    }.decode, struct {
        fn encode(value: *const Value) outcomes.Result(Text, Issue) {
            return .{ .ok = Text.format(value.*) };
        }
    }.encode);
}
