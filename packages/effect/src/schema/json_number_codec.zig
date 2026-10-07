//! Exact decimal JSON numbers. Comparisons use sign, decimal digits and scale;
//! floating-point conversion is never part of the exact-number contract.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const Issue = @import("schema_issue.zig").Issue;
const effects = @import("../effect.zig");
/// Invalid JSON decimal syntax or an exponent/scale outside the bounded i128 metadata representation.
pub const ParseError = error{ InvalidNumber, ExponentLimit };
/// Borrowed exact decimal view. text must outlive this value; no floating conversion or allocation occurs in parse/order.
pub const Number = struct {
    /// Original borrowed JSON token; normalization never mutates these bytes.
    text: []const u8,
    /// Normalized sign; every spelling of zero is treated as nonnegative.
    negative: bool,
    /// Start byte offset of the significant coefficient in text.
    first: usize,
    /// Exclusive byte offset after its significant coefficient.
    end: usize,
    /// Significant decimal digit count excluding the decimal point.
    digits: usize,
    /// Power of ten multiplying that coefficient after normalization.
    scale: i128,
    /// Validate JSON decimal grammar and retain a normalized coefficient/scale view into the original bytes.
    /// Trailing coefficient zeros are removed logically; the original lexical spelling is preserved.
    pub fn parse(text: []const u8) ParseError!Number {
        var cursor: usize = 0;
        if (text.len == 0) return error.InvalidNumber;
        const negative = text[0] == '-';
        if (negative) cursor += 1;
        if (cursor == text.len) return error.InvalidNumber;
        const integer_start = cursor;
        if (text[cursor] == '0') cursor += 1 else {
            if (text[cursor] < '1' or text[cursor] > '9') return error.InvalidNumber;
            while (cursor < text.len and std.ascii.isDigit(text[cursor])) cursor += 1;
        }
        if (cursor < text.len and std.ascii.isDigit(text[cursor])) return error.InvalidNumber;
        var fraction: usize = 0;
        if (cursor < text.len and text[cursor] == '.') {
            cursor += 1;
            const start = cursor;
            while (cursor < text.len and std.ascii.isDigit(text[cursor])) cursor += 1;
            fraction = cursor - start;
            if (fraction == 0) return error.InvalidNumber;
        }
        const coefficient_end = cursor;
        var exponent: i128 = 0;
        if (cursor < text.len and (text[cursor] == 'e' or text[cursor] == 'E')) {
            cursor += 1;
            const start = cursor;
            if (cursor < text.len and (text[cursor] == '+' or text[cursor] == '-')) cursor += 1;
            const digit_start = cursor;
            while (cursor < text.len and std.ascii.isDigit(text[cursor])) cursor += 1;
            if (cursor == digit_start) return error.InvalidNumber;
            exponent = std.fmt.parseInt(i128, text[start..cursor], 10) catch return error.ExponentLimit;
        }
        if (cursor != text.len) return error.InvalidNumber;
        var first = integer_start;
        while (first < coefficient_end and (text[first] == '0' or text[first] == '.')) first += 1;
        var end = coefficient_end;
        var trailing: usize = 0;
        while (end > first) {
            if (text[end - 1] == '.') {
                end -= 1;
                continue;
            }
            if (text[end - 1] != '0') break;
            end -= 1;
            trailing += 1;
        }
        var digits: usize = 0;
        for (text[first..end]) |character| if (character != '.') {
            digits += 1;
        };
        if (digits == 0) return .{ .text = text, .negative = false, .first = first, .end = end, .digits = 0, .scale = 0 };
        const scale = std.math.add(i128, std.math.sub(i128, exponent, @intCast(fraction)) catch return error.ExponentLimit, @intCast(trailing)) catch return error.ExponentLimit;
        return .{ .text = text, .negative = negative, .first = first, .end = end, .digits = digits, .scale = scale };
    }
    /// Count decimal integer-part digits after applying scale; zero and magnitudes below one count as one.
    pub fn integerDigits(self: Number) error{IntegerDigits}!usize {
        if (self.digits == 0) return 1;
        const count = std.math.add(i128, @intCast(self.digits), self.scale) catch return error.IntegerDigits;
        if (count <= 0) return 1;
        return std.math.cast(usize, count) orelse error.IntegerDigits;
    }
    /// Return whether the exact mathematical value has no fractional component.
    pub fn isInteger(self: Number) bool {
        return self.digits == 0 or self.scale >= 0;
    }
    fn digit(self: Number, index: usize) u8 {
        var count: usize = 0;
        for (self.text[self.first..self.end]) |character| {
            if (character == '.') continue;
            if (count == index) return character;
            count += 1;
        }
        return '0';
    }
    /// Compare exact mathematical values without f64 conversion, allocation or exponent-sized storage.
    pub fn order(self: Number, other: Number) std.math.Order {
        if (self.digits == 0 and other.digits == 0) return .eq;
        if (self.negative != other.negative) return if (self.negative) .lt else .gt;
        var ordering: std.math.Order = .eq;
        if (self.digits == 0) ordering = .lt else if (other.digits == 0) ordering = .gt else {
            // Widen before adding digit counts; extreme exponents remain ordered.
            const left = @as(i256, self.scale) + @as(i256, @intCast(self.digits));
            const right = @as(i256, other.scale) + @as(i256, @intCast(other.digits));
            ordering = std.math.order(left, right);
            if (ordering == .eq) {
                for (0..@max(self.digits, other.digits)) |index| {
                    ordering = std.math.order(self.digit(index), other.digit(index));
                    if (ordering != .eq) break;
                }
            }
        }
        return if (self.negative) switch (ordering) {
            .lt => .gt,
            .gt => .lt,
            .eq => .eq,
        } else ordering;
    }
    /// Convert into any Zig integer type with an explicit inclusive decimal digit bound.
    /// Reject fractions, excess digits and destination overflow; negative accumulation preserves signed minInt.
    pub fn toInteger(self: Number, comptime T: type, maximum_digits: usize) error{ IntegerDigits, NotInteger, Overflow }!T {
        if (!self.isInteger()) return error.NotInteger;
        const count = try self.integerDigits();
        if (count > maximum_digits) return error.IntegerDigits;
        var value: T = 0;
        if (self.digits == 0) return value;
        // Accumulate negative values directly so minInt remains representable.
        for (0..count) |index| {
            value = std.math.mul(T, value, 10) catch return error.Overflow;
            const digit_value: T = @intCast(self.digit(index) - '0');
            value = if (self.negative) std.math.sub(T, value, digit_value) catch return error.Overflow else std.math.add(T, value, digit_value) catch return error.Overflow;
        }
        return value;
    }
    /// Test exact divisibility by a positive nonzero decimal.
    /// R supplies scratch allocation; maximum_digits bounds coefficient storage, and modular exponentiation avoids expanding huge exponents.
    pub fn multipleOf(self: Number, divisor: Number, env: anytype, maximum_digits: usize) error{ OutOfMemory, IntegerDigits, InvalidDivisor }!bool {
        if (divisor.digits == 0 or divisor.negative) return error.InvalidDivisor;
        if (self.digits == 0) return true;
        // Canonical coefficients have no trailing zero. A negative scale
        // difference cannot produce an integer quotient. Positive differences
        // use modular exponentiation, never exponent-sized decimal buffers.
        if (self.scale < divisor.scale) return false;
        if (self.digits > maximum_digits or divisor.digits > maximum_digits) return error.IntegerDigits;
        const left_text = try env.allocator.alloc(u8, self.digits);
        defer env.allocator.free(left_text);
        const right_text = try env.allocator.alloc(u8, divisor.digits);
        defer env.allocator.free(right_text);
        for (left_text, 0..) |*slot, index| slot.* = self.digit(index);
        for (right_text, 0..) |*slot, index| slot.* = divisor.digit(index);
        const Big = std.math.big.int.Managed;
        var left = try Big.init(env.allocator);
        defer left.deinit();
        var right = try Big.init(env.allocator);
        defer right.deinit();
        var quotient = try Big.init(env.allocator);
        defer quotient.deinit();
        var remainder = try Big.init(env.allocator);
        defer remainder.deinit();
        var power = try Big.init(env.allocator);
        defer power.deinit();
        var product = try Big.init(env.allocator);
        defer product.deinit();
        var reduced = try Big.init(env.allocator);
        defer reduced.deinit();
        left.setString(10, left_text) catch return error.OutOfMemory;
        right.setString(10, right_text) catch return error.OutOfMemory;
        try Big.divTrunc(&quotient, &remainder, &left, &right);
        try left.set(@as(u8, 10));
        try Big.divTrunc(&quotient, &power, &left, &right);
        var exponent: u256 = @intCast(@as(i256, self.scale) - @as(i256, divisor.scale));
        while (exponent != 0) {
            if (exponent & 1 != 0) {
                try Big.mul(&product, &remainder, &power);
                try Big.divTrunc(&quotient, &reduced, &product, &right);
                try remainder.copy(reduced.toConst());
            }
            exponent >>= 1;
            if (exponent != 0) {
                try Big.mul(&product, &power, &power);
                try Big.divTrunc(&quotient, &reduced, &product, &right);
                try power.copy(reduced.toConst());
            }
        }
        return remainder.eqlZero();
    }
};

/// Owned validated decimal spelling. Deinitialize its lexical bytes or clone for an independent owner.
pub const OwnedNumber = struct {
    /// Exit/Result cleanup owns this value; release it once or clone it for an independent observation.
    pub const effect_owned_payload = true;
    /// Owned original token retained for exact re-encoding.
    lexical: outcomes.OwnedSlice(u8),
    /// Borrow a normalized Number view; keep this OwnedNumber alive while using it.
    pub fn number(self: *const @This()) Number {
        return Number.parse(self.lexical.items) catch unreachable;
    }
    /// Release the owned decimal spelling with its originating allocator.
    pub fn deinit(self: *@This()) void {
        self.lexical.deinit();
        self.* = undefined;
    }
    /// Copy the lexical spelling; the original remains unchanged if allocation fails.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{ .lexical = try self.lexical.clone() };
    }
};
/// Static exact-number validation. Bounds/multiples are borrowed comptime JSON decimal strings.
pub const Constraints = struct {
    /// Optional inclusive exact lower bound.
    minimum: ?[]const u8 = null,
    /// Optional inclusive exact upper bound.
    maximum: ?[]const u8 = null,
    /// Optional strict exact lower bound.
    exclusive_minimum: ?[]const u8 = null,
    /// Optional strict exact upper bound.
    exclusive_maximum: ?[]const u8 = null,
    /// Optional strictly positive exact divisor.
    multiple_of: ?[]const u8 = null,
    /// Require an exact integral mathematical value when true.
    integer: bool,
    /// Inclusive expanded integer-part digit budget.
    maximum_integer_digits: usize,
    /// Inclusive lexical byte budget for input and static constraint operands.
    maximum_value_bytes: usize,
};
/// Build a codec returning OwnedNumber and requiring R.allocator.
/// Reject invalid/contradictory bounds at comptime; both directions validate the same exact constraints.
/// Use JsonSurface.withLimits for separate input/preparation/output budgets.
pub fn exact(comptime R: type, comptime constraints: Constraints) type {
    comptime {
        if (constraints.maximum_integer_digits == 0 or constraints.maximum_value_bytes == 0) @compileError("exact numbers require explicit positive digit and byte bounds");
        for (.{ constraints.minimum, constraints.maximum, constraints.exclusive_minimum, constraints.exclusive_maximum, constraints.multiple_of }) |operand| {
            if (operand) |text| {
                if (text.len > constraints.maximum_value_bytes) @compileError("static JSON-number operand exceeds its value-byte bound");
                _ = Number.parse(text) catch @compileError("invalid static JSON-number constraint");
            }
        }
        if (constraints.multiple_of) |text| {
            const divisor = Number.parse(text) catch unreachable;
            if (divisor.digits == 0 or divisor.negative) @compileError("multipleOf must be positive");
        }
        for (.{ constraints.minimum, constraints.exclusive_minimum }, 0..) |lower, lower_index| {
            for (.{ constraints.maximum, constraints.exclusive_maximum }, 0..) |upper, upper_index| {
                if (lower != null and upper != null) {
                    const ordering = (Number.parse(lower.?) catch unreachable).order(Number.parse(upper.?) catch unreachable);
                    if (ordering == .gt or (ordering == .eq and (lower_index == 1 or upper_index == 1))) @compileError("invalid static JSON-number bounds");
                }
            }
        }
    }
    return struct {
        /// Application value produced on success; ownership follows the codec’s declared value type.
        pub const Decoded = OwnedNumber;
        /// Caller environment supplying allocation and any selected schema capabilities.
        pub const Environment = R;
        /// Public evidence for checked JSON writing without constructing an intermediate DOM.
        pub const supports_direct_writer = true;
        fn check(env: *const R, text: []const u8, direction: @import("schema_issue.zig").Direction) outcomes.Exit(void, Issue) {
            if (text.len > constraints.maximum_value_bytes) return .{ .fail = Issue.init(env, direction, .size_limit) };
            const value = Number.parse(text) catch |problem| return .{ .fail = Issue.init(env, direction, if (problem == error.ExponentLimit) .integer_digits else .invalid_number) };
            const digits = value.integerDigits() catch return .{ .fail = Issue.init(env, direction, .integer_digits) };
            if (digits > constraints.maximum_integer_digits) {
                var issue = Issue.init(env, direction, .integer_digits);
                issue.detail = .{ .integer_digits = .{ .limit = constraints.maximum_integer_digits, .required = digits } };
                return .{ .fail = issue };
            }
            if (constraints.integer and !value.isInteger()) return .{ .fail = Issue.init(env, direction, .invalid_number) };
            inline for (.{ constraints.minimum, constraints.maximum, constraints.exclusive_minimum, constraints.exclusive_maximum }, 0..) |operand, index| if (operand) |bound| {
                const ordering = value.order(Number.parse(bound) catch unreachable);
                if ((index == 0 and ordering == .lt) or (index == 1 and ordering == .gt) or (index == 2 and ordering != .gt) or (index == 3 and ordering != .lt)) return constraintFailure(env, direction, switch (index) {
                    0 => "minimum",
                    1 => "maximum",
                    2 => "exclusiveMinimum",
                    else => "exclusiveMaximum",
                }, bound, text);
            };
            if (constraints.multiple_of) |text_| {
                const multiple = value.multipleOf(Number.parse(text_) catch unreachable, env, constraints.maximum_value_bytes) catch |problem| {
                    if (problem == error.OutOfMemory) return .{ .die = outcomes.Defect.out_of_memory };
                    return .{ .fail = Issue.init(env, direction, .integer_digits) };
                };
                if (!multiple) return constraintFailure(env, direction, "multipleOf", text_, text);
            }
            return .{ .success = {} };
        }
        /// Validate an integer or number_string DOM token and copy its original spelling into R.allocator.
        /// Float DOM values are rejected rather than treated as exact decimals.
        pub fn decodeDirect(env: *const R, input: *const std.json.Value) outcomes.Exit(OwnedNumber, Issue) {
            var buffer: [std.fmt.count("{d}", .{std.math.minInt(i64)})]u8 = undefined;
            const text = switch (input.*) {
                .number_string => |text| text,
                .integer => |value| std.fmt.bufPrint(&buffer, "{d}", .{value}) catch unreachable,
                else => return .{ .fail = Issue.init(env, .decode, .invalid_number) },
            };
            const checked = check(env, text, .decode);
            if (checked != .success) return outcomes.castFailure(OwnedNumber, checked);
            const owned = env.allocator.dupe(u8, text) catch return .{ .die = outcomes.Defect.out_of_memory };
            return .{ .success = .{ .lexical = .{ .items = owned, .allocator = env.allocator } } };
        }
        /// Lazy borrowed-DOM decoding; input remains alive through join and success owns its lexical bytes.
        pub fn decode(input: *const std.json.Value) effects.Effect(OwnedNumber, Issue, R) {
            return effects.Effect(OwnedNumber, Issue, R).fromBorrowed(std.json.Value, input, struct {
                fn run(value: *const std.json.Value, env: *const R) outcomes.Exit(OwnedNumber, Issue) {
                    return decodeDirect(env, value);
                }
            }.run);
        }
        /// Check the owned number against encode-direction constraints without rewriting its token.
        pub fn validateDirect(env: *const R, input: *const OwnedNumber) outcomes.Exit(void, Issue) {
            return check(env, input.lexical.items, .encode);
        }
        /// Validate once, then write the exact original token through the bounded writer; output never rounds through f64.
        pub fn writeJson(env: *const R, input: *const OwnedNumber, writer: *@import("json_writer.zig").Writer) outcomes.Exit(void, Issue) {
            const checked = validateDirect(env, input);
            if (checked != .success) return checked;
            writer.numeric(input.lexical.items) catch |problem| return outcomes.castFailure(void, @import("json_writer.zig").failureWith(env, .encode, problem, writer));
            return .{ .success = {} };
        }
        /// Return a number_string view borrowing input’s lexical owner after encode validation.
        pub fn encodeWithin(env: *const R, input: *const OwnedNumber, _: *@import("internal/structural.zig").EncodeState) outcomes.Exit(std.json.Value, Issue) {
            const checked = validateDirect(env, input);
            if (checked != .success) return outcomes.castFailure(std.json.Value, checked);
            return .{ .success = .{ .number_string = input.lexical.items } };
        }
        /// Decode within an existing structural traversal; success still independently owns its decimal bytes.
        pub fn decodeWithin(env: *const R, input: *const std.json.Value, _: *@import("internal/structural.zig").DecodeState) outcomes.Exit(OwnedNumber, Issue) {
            return decodeDirect(env, input);
        }
        /// Return immutable encoded-shape metadata; it does not execute callbacks or construct application values.
        pub fn encodedNode() *const @import("schema_ast.zig").Node {
            const Metadata = struct {
                const node: @import("schema_ast.zig").Node = .{ .exact_number = .{ .minimum = constraints.minimum, .maximum = constraints.maximum, .exclusive_minimum = constraints.exclusive_minimum, .exclusive_maximum = constraints.exclusive_maximum, .multiple_of = constraints.multiple_of, .integer = constraints.integer } };
            };
            return &Metadata.node;
        }
    };
}

fn constraintFailure(env: anytype, direction: @import("schema_issue.zig").Direction, name: []const u8, expected: []const u8, actual: []const u8) outcomes.Exit(void, Issue) {
    const owned_name = env.allocator.dupe(u8, name) catch return .{ .die = outcomes.Defect.out_of_memory };
    const owned_expected = env.allocator.dupe(u8, expected) catch {
        env.allocator.free(owned_name);
        return .{ .die = outcomes.Defect.out_of_memory };
    };
    const owned_actual = env.allocator.dupe(u8, actual) catch {
        env.allocator.free(owned_expected);
        env.allocator.free(owned_name);
        return .{ .die = outcomes.Defect.out_of_memory };
    };
    var issue = Issue.init(env, direction, .constraint);
    issue.detail = .{ .constraint = .{ .name = .{ .items = owned_name, .allocator = env.allocator }, .expected = .{ .items = owned_expected, .allocator = env.allocator }, .actual = .{ .items = owned_actual, .allocator = env.allocator } } };
    return .{ .fail = issue };
}
