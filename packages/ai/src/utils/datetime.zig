const std = @import("std");
const re = @import("../regex/regexp.zig");
const days_before_epoch = 719528;
const Date = struct {
    year: u16,
    month: u8,
    day: u8,
    hour: u8 = 0,
    minute: u8 = 0,
    second: u8 = 0,
    millisecond: u16 = 0,
    fn milliseconds(self: @This()) !i64 {
        if (self.month < 1 or self.month > 12 or self.day < 1 or self.day > std.time.epoch.getDaysInMonth(self.year, @fromBackingInt(@intCast(self.month))) or self.hour > 23 or self.minute > 59 or self.second > 60) return error.InvalidTimestamp;
        var days: i64 = -days_before_epoch;
        var year: u16 = 0;
        while (year < self.year) : (year += 1) days += std.time.epoch.getDaysInYear(year);
        var month: u8 = 1;
        while (month < self.month) : (month += 1) days += std.time.epoch.getDaysInMonth(self.year, @fromBackingInt(@intCast(month)));
        days += self.day - 1;
        return (((days * 24 + self.hour) * 60 + self.minute) * 60 + self.second) * 1000 + self.millisecond;
    }
    fn fromMilliseconds(input_ms: i64) !@This() {
        var days = @divFloor(input_ms, std.time.ms_per_day) + days_before_epoch;
        if (days < 0 or days >= 3652425) return error.TimestampOutOfRange;
        var year: u16 = 0;
        while (days >= std.time.epoch.getDaysInYear(year)) : (year += 1) days -= std.time.epoch.getDaysInYear(year);
        var month: u8 = 1;
        while (days >= std.time.epoch.getDaysInMonth(year, @fromBackingInt(@intCast(month)))) : (month += 1) days -= std.time.epoch.getDaysInMonth(year, @fromBackingInt(@intCast(month)));
        const time = @mod(input_ms, std.time.ms_per_day);
        return .{ .year = year, .month = month, .day = @intCast(days + 1), .hour = @intCast(@divTrunc(time, std.time.ms_per_hour)), .minute = @intCast(@mod(@divTrunc(time, std.time.ms_per_min), 60)), .second = @intCast(@mod(@divTrunc(time, 1000), 60)), .millisecond = @intCast(@mod(time, 1000)) };
    }
};
pub fn formatRfc3339(a: std.mem.Allocator, milliseconds: i64) ![]const u8 {
    const value = try Date.fromMilliseconds(milliseconds);
    return std.fmt.allocPrint(a, "{d:0>4}-{d:0>2}-{d:0>2}T{d:0>2}:{d:0>2}:{d:0>2}.{d:0>3}Z", .{ value.year, value.month, value.day, value.hour, value.minute, value.second, value.millisecond });
}
/// Date.prototype.toISOString's signed TimeClip range and expanded years.
/// Gregorian cycles let std's calendar helpers handle each year in at most
/// one 400-year cycle, including dates before year zero.
pub fn formatJavaScriptIsoUtc(a: std.mem.Allocator, milliseconds: i128) ![]u8 {
    if (milliseconds < -8640000000000000 or milliseconds > 8640000000000000) return error.TimestampOutOfRange;
    const value: i64 = @intCast(milliseconds);
    var days = @divFloor(value, std.time.ms_per_day) + days_before_epoch;
    const cycle = @divFloor(days, 146097);
    days = @mod(days, 146097);
    var cycle_year: u16 = 0;
    while (days >= std.time.epoch.getDaysInYear(cycle_year)) : (cycle_year += 1) days -= std.time.epoch.getDaysInYear(cycle_year);
    const year = cycle * 400 + cycle_year;
    var month: u8 = 1;
    while (days >= std.time.epoch.getDaysInMonth(cycle_year, @fromBackingInt(@intCast(month)))) : (month += 1) days -= std.time.epoch.getDaysInMonth(cycle_year, @fromBackingInt(@intCast(month)));
    const time = @mod(value, std.time.ms_per_day);
    const year_text = if (year >= 0 and year <= 9999) try std.fmt.allocPrint(a, "{d:0>4}", .{year}) else try std.fmt.allocPrint(a, "{c}{d:0>6}", .{ @as(u8, if (year < 0) '-' else '+'), @abs(year) });
    defer a.free(year_text);
    return std.fmt.allocPrint(a, "{s}-{d:0>2}-{d:0>2}T{d:0>2}:{d:0>2}:{d:0>2}.{d:0>3}Z", .{ year_text, month, days + 1, @divTrunc(time, std.time.ms_per_hour), @mod(@divTrunc(time, std.time.ms_per_min), 60), @mod(@divTrunc(time, 1000), 60), @mod(time, 1000) });
}
fn number(a: std.mem.Allocator, match: re.Match, index: usize) !u16 {
    const value = (try match.text(a, index)) orelse return 0;
    defer a.free(value);
    return std.fmt.parseInt(u16, std.mem.trimStart(u8, value, " "), 10) catch error.InvalidTimestamp;
}
fn fraction(a: std.mem.Allocator, match: re.Match, index: usize) !u16 {
    const value = (try match.text(a, index)) orelse return 0;
    defer a.free(value);
    const digits = value[0..@min(3, value.len)];
    return (std.fmt.parseInt(u16, digits, 10) catch return error.InvalidTimestamp) * switch (digits.len) {
        1 => @as(u16, 100),
        2 => 10,
        else => 1,
    };
}
fn javaScriptFraction(a: std.mem.Allocator, match: re.Match, index: usize) !u16 {
    const value = (try match.text(a, index)) orelse return 0;
    defer a.free(value);
    const significant = std.mem.trimStart(u8, value, "0");
    if (significant.len == 0) return 0;
    const numeric = try std.fmt.parseInt(u32, significant[0..@min(9, significant.len)], 10);
    const length = @min(9, value.len);
    return @intCast(switch (length) {
        1 => numeric * 100,
        2 => numeric * 10,
        else => numeric / std.math.pow(u32, 10, @intCast(length - 3)),
    });
}
pub fn parseRfc3339(a: std.mem.Allocator, bytes: []const u8) !i64 {
    var regex = try re.Regex.init(a, null,
        \\^(\d{4})-(\d{2})-(\d{2})[tT](\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?(?:[zZ]|([+-])(\d{2}):(\d{2}))$
    , .{});
    defer regex.deinit();
    var match = (try regex.exec(a, bytes, 0)) orelse return error.InvalidTimestamp;
    defer match.deinit();
    const date = Date{ .year = try number(a, match, 1), .month = @intCast(try number(a, match, 2)), .day = @intCast(try number(a, match, 3)), .hour = @intCast(try number(a, match, 4)), .minute = @intCast(try number(a, match, 5)), .second = @intCast(try number(a, match, 6)), .millisecond = try fraction(a, match, 7) };
    const zone_hour = try number(a, match, 9);
    const zone_minute = try number(a, match, 10);
    if (zone_hour > 23 or zone_minute > 59) return error.InvalidTimestamp;
    const sign = try match.text(a, 8);
    defer if (sign) |value| a.free(value);
    const offset = (@as(i64, zone_hour) * 60 + zone_minute) * std.time.ms_per_min;
    return (try date.milliseconds()) + (if (sign) |value| (if (std.mem.eql(u8, value, "-")) offset else -offset) else 0);
}
fn smithyDateField(a: std.mem.Allocator, value: u16, kind: []const u8, lower: u16, upper: u16, error_message: ?*?[]const u8) !u16 {
    if (value < lower or value > upper) {
        if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "{s} must be between {d} and {d}, inclusive", .{ kind, lower, upper });
        return error.InvalidTimestamp;
    }
    return value;
}
/// Smithy's UTC-only parseRfc3339DateTime, including buildDate's Date.UTC
/// year adjustment and number-to-millisecond conversion. The offset-capable
/// parser above remains a distinct boundary for its existing callers.
pub fn parseSmithyRfc3339(a: std.mem.Allocator, bytes: []const u8, error_message: ?*?[]const u8) !i64 {
    var regex = try re.Regex.init(a, null,
        \\^(\d{4})-(\d{2})-(\d{2})[tT](\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?[zZ]$
    , .{});
    defer regex.deinit();
    var match = (try regex.exec(a, bytes, 0)) orelse {
        if (error_message) |slot| slot.* = "Invalid RFC-3339 date-time value";
        return error.InvalidTimestamp;
    };
    defer match.deinit();
    const year = try number(a, match, 1);
    const month = try smithyDateField(a, try number(a, match, 2), "month", 1, 12, error_message);
    const day = try smithyDateField(a, try number(a, match, 3), "day", 1, 31, error_message);
    if (day > std.time.epoch.getDaysInMonth(year, @fromBackingInt(@intCast(month)))) {
        if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Invalid day for {s} in {d}: {d}", .{ months[month - 1], year, day });
        return error.InvalidTimestamp;
    }
    const hour = try smithyDateField(a, try number(a, match, 4), "hour", 0, 23, error_message);
    const minute = try smithyDateField(a, try number(a, match, 5), "minute", 0, 59, error_message);
    const second = try smithyDateField(a, try number(a, match, 6), "seconds", 0, 60, error_message);
    const milliseconds: u16 = if (try match.text(a, 7)) |digits| fraction_value: {
        defer a.free(digits);
        const numeric = try std.fmt.allocPrint(a, "0.{s}", .{digits});
        defer a.free(numeric);
        break :fraction_value @intFromFloat(@trunc((try std.fmt.parseFloat(f64, numeric)) * 1000));
    } else 0;
    const date = Date{ .year = if (year < 100) year + 1900 else year, .month = @intCast(month), .day = 1, .hour = @intCast(hour), .minute = @intCast(minute), .second = @intCast(second) };
    return (try date.milliseconds()) + @as(i64, day - 1) * std.time.ms_per_day + milliseconds;
}
pub fn parseJavaScriptIsoUtc(a: std.mem.Allocator, bytes: []const u8) !f64 {
    var regex = try re.Regex.init(a, null,
        \\^([+-]\d{6}|\d{4})(?:-(\d{2})(?:-(\d{2}))?)?(?:[tT](\d{2}):(\d{2})(?::(\d{2})(?:\.(\d+))?)?([zZ]|([+-])(\d{2}):?(\d{2})))?$
    , .{});
    defer regex.deinit();
    var match = (try regex.exec(a, bytes, 0)) orelse return std.math.nan(f64);
    defer match.deinit();
    const year_text = (try match.text(a, 1)).?;
    defer a.free(year_text);
    if (std.mem.eql(u8, year_text, "-000000")) return std.math.nan(f64);
    const year = std.fmt.parseInt(i32, year_text, 10) catch return std.math.nan(f64);
    const month = if (match.captures[2] != null) try number(a, match, 2) else 1;
    const day = if (match.captures[3] != null) try number(a, match, 3) else 1;
    const hour = try number(a, match, 4);
    const minute = try number(a, match, 5);
    const second = try number(a, match, 6);
    const millisecond = try javaScriptFraction(a, match, 7);
    const zone_hour = try number(a, match, 10);
    const zone_minute = try number(a, match, 11);
    if (month < 1 or month > 12 or day < 1 or day > 31 or hour > 24 or minute > 59 or second > 59 or zone_hour > 23 or zone_minute > 59) return std.math.nan(f64);
    if (hour == 24) {
        if (minute != 0 or second != 0) return std.math.nan(f64);
        if (try match.text(a, 7)) |digits| {
            defer a.free(digits);
            if (!std.mem.allEqual(u8, digits, '0')) return std.math.nan(f64);
        }
    }
    // A Gregorian calendar repeats every 400 years. Reducing the signed year
    // to that cycle lets std's unsigned-year epoch helpers cover expanded and
    // negative ISO years, without an unbounded year-by-year walk.
    const cycle_year: u16 = @intCast(@mod(year, 400));
    var days: i64 = @as(i64, @divFloor(year, 400)) * 146097 - days_before_epoch;
    var prior: u16 = 0;
    while (prior < cycle_year) : (prior += 1) days += std.time.epoch.getDaysInYear(prior);
    var prior_month: u8 = 1;
    while (prior_month < month) : (prior_month += 1) days += std.time.epoch.getDaysInMonth(cycle_year, @fromBackingInt(@intCast(prior_month)));
    // Date's MakeDay normalizes day 29/30/31 past the selected month.
    days += day - 1;
    const sign = try match.text(a, 9);
    defer if (sign) |value| a.free(value);
    const offset = (@as(i64, zone_hour) * 60 + zone_minute) * std.time.ms_per_min;
    const milliseconds = (((days * 24 + hour) * 60 + minute) * 60 + second) * 1000 + millisecond + (if (sign) |value| (if (std.mem.eql(u8, value, "-")) offset else -offset) else 0);
    if (milliseconds < -8640000000000000 or milliseconds > 8640000000000000) return std.math.nan(f64);
    return @floatFromInt(milliseconds);
}
/// Explicit-zone legacy dates compose the ISO calendar codec rather than HTTP
/// date validation: JavaScript normalizes day overflow and uses its own legacy
/// year rule. Local-time and remaining legacy forms remain separate work.
pub fn parseJavaScriptDate(a: std.mem.Allocator, io: std.Io, bytes: []const u8) !f64 {
    const input = std.mem.sliceTo(bytes, 0);
    const iso = try parseJavaScriptIsoUtc(a, input);
    if (!std.math.isNan(iso)) return iso;
    const patterns = [_][]const u8{
        \\^(?:Mon(?:day)?|Tue(?:sday)?|Wed(?:nesday)?|Thu(?:rsday)?|Fri(?:day)?|Sat(?:urday)?|Sun(?:day)?), (\d{1,2})[ -](Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)[A-Za-z]*[ -](\d{2}|\d{4}) (\d{1,2}):(\d{1,2}):(\d{1,2})(?:\.(\d+))? (GMT|UTC|UT|Z|[ECMP][DS]T)(?:([+-])(\d{1,4})(?::(\d{1,2}))?)?(?: \([^()]*\))?$
        ,
        \\^(?:Mon|Tue|Wed|Thu|Fri|Sat|Sun) (Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)[A-Za-z]* (\d{1,2}) (\d{2}|\d{4}) (\d{1,2}):(\d{1,2}):(\d{1,2})(?:\.(\d+))? (GMT|UTC|UT|Z|[ECMP][DS]T)(?:([+-])(\d{1,4})(?::(\d{1,2}))?)?(?: \([^()]*\))?$
    };
    for (patterns, 0..) |pattern, index| {
        var regex = try re.Regex.init(a, io, pattern, .{ .ignoreCase = true });
        defer regex.deinit();
        var match = (try regex.exec(a, input, 0)) orelse continue;
        defer match.deinit();
        const month_text = (try match.text(a, if (index == 0) 2 else 1)).?;
        defer a.free(month_text);
        const month: u8 = for (months, 1..) |name, ordinal| {
            if (std.ascii.eqlIgnoreCase(name, month_text)) break @intCast(ordinal);
        } else return std.math.nan(f64);
        const first = try number(a, match, if (index == 0) 1 else 2);
        const second = try number(a, match, 3);
        const first_is_day = first >= 1 and first <= 31;
        var year = if (first_is_day) second else first;
        const day = if (first_is_day) first else second;
        if (year < 50) year += 2000 else if (year < 100) year += 1900;
        const normalized = try std.fmt.allocPrint(a, "{d:0>4}-{d:0>2}-{d:0>2}T{d:0>2}:{d:0>2}:{d:0>2}.{d:0>3}Z", .{ year, month, day, try number(a, match, 4), try number(a, match, 5), try number(a, match, 6), try javaScriptFraction(a, match, 7) });
        defer a.free(normalized);
        var milliseconds = try parseJavaScriptIsoUtc(a, normalized);
        const zone = (try match.text(a, 8)).?;
        defer a.free(zone);
        var offset_minutes: i64 = 0;
        inline for (.{ .{ "CDT", -5 }, .{ "CST", -6 }, .{ "EDT", -4 }, .{ "EST", -5 }, .{ "MDT", -6 }, .{ "MST", -7 }, .{ "PDT", -7 }, .{ "PST", -8 } }) |entry| {
            if (std.ascii.eqlIgnoreCase(zone, entry[0])) offset_minutes = entry[1] * 60;
        }
        if (try match.text(a, 9)) |sign| {
            defer a.free(sign);
            const digits = (try match.text(a, 10)).?;
            defer a.free(digits);
            const offset = try std.fmt.parseInt(u16, digits, 10);
            offset_minutes = if (match.captures[11] != null) colon: {
                const minute = try number(a, match, 11);
                if (minute > 59) return std.math.nan(f64);
                break :colon @as(i64, offset) * 60 + minute;
            } else if (digits.len <= 2) @as(i64, offset) * 60 else @as(i64, offset / 100) * 60 + offset % 100;
            if (std.mem.eql(u8, sign, "-")) offset_minutes = -offset_minutes;
        }
        milliseconds -= @as(f64, @floatFromInt(offset_minutes * std.time.ms_per_min));
        return if (@abs(milliseconds) > 8.64e15) std.math.nan(f64) else milliseconds;
    }
    return std.math.nan(f64);
}
/// new Date(value) for retained JSON input. JSON objects/arrays have no callable
/// custom primitive hooks; their ordinary primitive string uses the existing
/// conversion helper. Missing input and TimeClip failures retain NaN.
pub fn parseJavaScriptDateValue(a: std.mem.Allocator, io: std.Io, value: ?std.json.Value) !f64 {
    const input = value orelse return std.math.nan(f64);
    const milliseconds = switch (input) {
        .string => |text| try parseJavaScriptDate(a, io, text),
        .object, .array => date: {
            var temporary = std.heap.ArenaAllocator.init(a);
            defer temporary.deinit();
            const text = try @import("diagnostics.zig").formatThrownValue(temporary.allocator(), .{ .value = input });
            break :date try parseJavaScriptDate(a, io, text);
        },
        .null => @as(f64, 0),
        .bool => |flag| @as(f64, if (flag) 1 else 0),
        .integer, .float, .number_string => @import("../json/value.zig").number(input) orelse std.math.nan(f64),
    };
    return if (!std.math.isFinite(milliseconds) or @abs(milliseconds) > 8.64e15) std.math.nan(f64) else @trunc(milliseconds);
}
pub fn isDate(a: std.mem.Allocator, bytes: []const u8) !bool {
    return @import("../schema/formats.zig").isDate(a, bytes);
}
const months = [_][]const u8{ "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec" };
pub fn parseHttpDate(a: std.mem.Allocator, io: std.Io, bytes: []const u8) !i64 {
    const patterns = [_][]const u8{
        \\^(?:Mon|Tue|Wed|Thu|Fri|Sat|Sun), (\d{2}) (Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) (\d{4}) (\d{1,2}):(\d{2}):(\d{2})(?:\.(\d+))? GMT$
        ,
        \\^(?:Monday|Tuesday|Wednesday|Thursday|Friday|Saturday|Sunday), (\d{2})-(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)-(\d{2}) (\d{1,2}):(\d{2}):(\d{2})(?:\.(\d+))? GMT$
        ,
        \\^(?:Mon|Tue|Wed|Thu|Fri|Sat|Sun) (Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) ( [1-9]|\d{2}) (\d{1,2}):(\d{2}):(\d{2})(?:\.(\d+))? (\d{4})$
    };
    for (patterns, 0..) |pattern, index| {
        var regex = try re.Regex.init(a, io, pattern, .{});
        defer regex.deinit();
        var match = (try regex.exec(a, bytes, 0)) orelse continue;
        defer match.deinit();
        const month_text = (try match.text(a, if (index == 2) 1 else 2)).?;
        defer a.free(month_text);
        const month: u8 = for (months, 1..) |name, ordinal| {
            if (std.mem.eql(u8, name, month_text)) break @intCast(ordinal);
        } else return error.InvalidTimestamp;
        var date = Date{ .year = try number(a, match, if (index == 2) 7 else 3), .month = month, .day = @intCast(try number(a, match, if (index == 2) 2 else 1)), .hour = @intCast(try number(a, match, if (index == 2) 3 else 4)), .minute = @intCast(try number(a, match, if (index == 2) 4 else 5)), .second = @intCast(try number(a, match, if (index == 2) 5 else 6)), .millisecond = try fraction(a, match, if (index == 2) 6 else 7) };
        if (index == 1) {
            const now = std.Io.Clock.real.now(io).toMilliseconds();
            const current = try Date.fromMilliseconds(now);
            date.year += @divTrunc(current.year, 100) * 100;
            if (date.year < current.year) date.year += 100;
            if ((try date.milliseconds()) - now > 50 * 365 * std.time.ms_per_day) date.year -= 100;
        }
        return date.milliseconds();
    }
    return error.InvalidTimestamp;
}
pub fn millisecondsFromSeconds(seconds: f64) !i64 {
    const value = seconds * 1000;
    if (!std.math.isFinite(value) or value < @as(f64, @floatFromInt(std.math.minInt(i64))) or value >= @as(f64, @floatFromInt(std.math.maxInt(i64)))) return error.TimestampOutOfRange;
    return @intFromFloat(value);
}
pub fn expiresAfter(io: std.Io, seconds: f64) !i64 {
    const now = std.math.cast(i64, std.Io.Clock.real.now(io).toMilliseconds()) orelse return error.TimestampOutOfRange;
    return std.math.add(i64, now, try millisecondsFromSeconds(seconds)) catch error.TimestampOutOfRange;
}
pub fn expiresAfterNumber(io: std.Io, seconds: f64) f64 {
    return @as(f64, @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds())) + seconds * 1000;
}
