//! Pure Gregorian recurring-calendar evaluation. A zone capability supplies
//! civil-time resolution, including gaps and any number of overlap candidates;
//! no process timezone or bundled timezone database is consulted.
const std = @import("std");
const outcomes = @import("outcome.zig");
const deadlines = @import("deadline.zig");

pub const Error = enum {
    invalid_date,
    invalid_policy,
    overflow,
    zone_unavailable,
    zone_contract,
};

pub fn FieldSet(comptime count: comptime_int, comptime first: comptime_int) type {
    const Bits = std.meta.Int(.unsigned, count);
    return struct {
        const Self = @This();
        bits: Bits,

        pub fn all() Self {
            return .{ .bits = std.math.maxInt(Bits) };
        }

        pub fn only(comptime values: []const u8) Self {
            if (values.len == 0) @compileError("calendar field set cannot be empty");
            var bits: Bits = 0;
            inline for (values) |value| {
                if (value < first or value >= first + count)
                    @compileError("calendar field value is out of range");
                bits |= @as(Bits, 1) << @intCast(value - first);
            }
            return .{ .bits = bits };
        }

        pub fn has(self: Self, value: u8) bool {
            if (value < first or value >= first + count) return false;
            return self.bits & (@as(Bits, 1) << @intCast(value - first)) != 0;
        }

        pub fn valid(self: Self) bool {
            return self.bits != 0;
        }
    };
}

pub const Minutes = FieldSet(60, 0);
pub const Hours = FieldSet(24, 0);
pub const Days = FieldSet(31, 1);
pub const Months = FieldSet(12, 1);
/// Sunday is 0, Saturday is 6; this is part of the rule's wire contract.
pub const Weekdays = FieldSet(7, 0);

pub const LocalMinute = struct {
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,

    pub fn valid(self: LocalMinute) bool {
        return self.month >= 1 and self.month <= 12 and self.day >= 1 and
            self.day <= daysInMonth(self.year, self.month) and
            self.hour < 24 and self.minute < 60;
    }

    pub fn epochMinute(self: LocalMinute) outcomes.Result(i64, Error) {
        if (!self.valid()) return .{ .err = .invalid_date };
        const day = daysFromCivil(self.year, self.month, self.day);
        const day_minutes = std.math.mul(i64, day, minutes_per_day) catch
            return .{ .err = .overflow };
        const minute_in_day: i64 = @as(i64, self.hour) * minutes_per_hour + self.minute;
        return .{ .ok = std.math.add(i64, day_minutes, minute_in_day) catch
            return .{ .err = .overflow } };
    }

    pub fn next(self: LocalMinute) outcomes.Result(LocalMinute, Error) {
        const current = switch (self.epochMinute()) {
            .ok => |value| value,
            .err => |problem| return .{ .err = problem },
        };
        return fromEpochMinute(std.math.add(i64, current, 1) catch
            return .{ .err = .overflow });
    }
};

const minutes_per_hour: i64 = 60;
const hours_per_day: i64 = 24;
const minutes_per_day: i64 = minutes_per_hour * hours_per_day;
const nanoseconds_per_second: i128 = 1_000_000_000;
const seconds_per_minute: i128 = 60;
const nanoseconds_per_minute: i128 = nanoseconds_per_second * seconds_per_minute;

fn leap(year: i32) bool {
    return @mod(year, 4) == 0 and (@mod(year, 100) != 0 or @mod(year, 400) == 0);
}

fn daysInMonth(year: i32, month: u8) u8 {
    return switch (month) {
        1, 3, 5, 7, 8, 10, 12 => 31,
        4, 6, 9, 11 => 30,
        2 => if (leap(year)) 29 else 28,
        else => 0,
    };
}

// The civil-day conversion uses the proleptic Gregorian calendar. 146097 is
// the day count of a complete 400-year leap cycle; 719468 aligns that cycle
// with 1970-01-01. The 153/5 month terms count the March-based five-month
// groups. The inverse's 1460, 36524 and 146096 terms locate leap-year,
// century and era boundaries. These are calendar identities, not runtime
// tuning constants or scheduling defaults.
const years_per_era: i64 = 400;
const days_per_era: i64 = 146097;
const unix_epoch_displacement_days: i64 = 719468;

fn daysFromCivil(year: i32, month: u8, day: u8) i64 {
    const adjusted_year: i64 = @as(i64, year) - @as(i64, if (month <= 2) 1 else 0);
    const era = @divFloor(adjusted_year, years_per_era);
    const year_of_era = adjusted_year - era * years_per_era;
    const adjusted_month: i64 = @as(i64, month) + (if (month > 2) @as(i64, -3) else 9);
    const day_of_year = @divTrunc(153 * adjusted_month + 2, 5) + @as(i64, day) - 1;
    const day_of_era = year_of_era * 365 + @divTrunc(year_of_era, 4) -
        @divTrunc(year_of_era, 100) + day_of_year;
    return era * days_per_era + day_of_era - unix_epoch_displacement_days;
}

fn fromEpochMinute(value: i64) outcomes.Result(LocalMinute, Error) {
    const day = @divFloor(value, minutes_per_day);
    const within_day = value - day * minutes_per_day;
    const shifted = std.math.add(i64, day, unix_epoch_displacement_days) catch
        return .{ .err = .overflow };
    const era = @divFloor(shifted, days_per_era);
    const day_of_era = shifted - era * days_per_era;
    const year_of_era = @divTrunc(day_of_era - @divTrunc(day_of_era, 1460) +
        @divTrunc(day_of_era, 36524) - @divTrunc(day_of_era, 146096), 365);
    var year = year_of_era + era * years_per_era;
    const day_of_year = day_of_era - (365 * year_of_era +
        @divTrunc(year_of_era, 4) - @divTrunc(year_of_era, 100));
    const month_piece = @divTrunc(5 * day_of_year + 2, 153);
    const civil_day = day_of_year - @divTrunc(153 * month_piece + 2, 5) + 1;
    const civil_month = month_piece + (if (month_piece < 10) @as(i64, 3) else -9);
    if (civil_month <= 2) year += 1;
    return .{ .ok = .{
        .year = std.math.cast(i32, year) orelse return .{ .err = .overflow },
        .month = @intCast(civil_month),
        .day = @intCast(civil_day),
        .hour = @intCast(@divTrunc(within_day, minutes_per_hour)),
        .minute = @intCast(@mod(within_day, minutes_per_hour)),
    } };
}

pub const Rule = struct {
    minute: Minutes,
    hour: Hours,
    day: Days,
    month: Months,
    weekday: Weekdays,

    pub fn valid(self: Rule) bool {
        return self.minute.valid() and self.hour.valid() and self.day.valid() and
            self.month.valid() and self.weekday.valid();
    }

    pub fn matches(self: Rule, local: LocalMinute) bool {
        if (!self.valid() or !local.valid()) return false;
        const weekday: u8 = @intCast(@mod(daysFromCivil(local.year, local.month, local.day) + 4, 7));
        return self.minute.has(local.minute) and self.hour.has(local.hour) and
            self.day.has(local.day) and self.month.has(local.month) and
            self.weekday.has(weekday);
    }
};

pub const OverlapPolicy = enum { first, last, both };
pub const SearchPolicy = struct {
    max_local_minutes: usize,
    max_candidates_per_local: usize,
    overlap: OverlapPolicy,

    pub fn valid(self: SearchPolicy) bool {
        return self.max_local_minutes > 0 and self.max_candidates_per_local > 0;
    }
};

/// `next_candidate` returns ascending instants for the same civil minute,
/// strictly after `after` when supplied. Null means no more candidates; a
/// gap has none. The returned local date must round-trip through `to_local`.
pub const Zone = struct {
    userdata: ?*anyopaque,
    vtable: *const VTable,

    pub const VTable = struct {
        to_local: *const fn (?*anyopaque, deadlines.WallInstant) outcomes.Result(LocalMinute, Error),
        next_candidate: *const fn (?*anyopaque, LocalMinute, ?deadlines.WallInstant) outcomes.Result(?deadlines.WallInstant, Error),
    };
};

pub const FixedOffset = struct {
    minutes_east_of_utc: i32,

    pub fn zone(self: *const FixedOffset) Zone {
        return .{ .userdata = @ptrCast(@constCast(self)), .vtable = &.{ .to_local = toLocal, .next_candidate = nextCandidate } };
    }

    fn toLocal(data: ?*anyopaque, instant: deadlines.WallInstant) outcomes.Result(LocalMinute, Error) {
        const self: *const FixedOffset = @ptrCast(@alignCast(data.?));
        const utc_minute = @divFloor(instant.unix_nanoseconds, nanoseconds_per_minute);
        const local_minute = std.math.add(i128, utc_minute, self.minutes_east_of_utc) catch return .{ .err = .overflow };
        return fromEpochMinute(std.math.cast(i64, local_minute) orelse
            return .{ .err = .overflow });
    }

    fn nextCandidate(data: ?*anyopaque, local: LocalMinute, after: ?deadlines.WallInstant) outcomes.Result(?deadlines.WallInstant, Error) {
        const self: *const FixedOffset = @ptrCast(@alignCast(data.?));
        const minute = switch (local.epochMinute()) {
            .ok => |value| value,
            .err => |problem| return .{ .err = problem },
        };
        const utc_minute = @as(i128, minute) - self.minutes_east_of_utc;
        const instant = deadlines.WallInstant{ .unix_nanoseconds = std.math.mul(i128, utc_minute, nanoseconds_per_minute) catch
            return .{ .err = .overflow } };
        if (after) |previous| if (instant.unix_nanoseconds <= previous.unix_nanoseconds)
            return .{ .ok = null };
        return .{ .ok = instant };
    }
};

/// Returns null when the caller's search bound is exhausted. The caller
/// chooses catch-up behavior by advancing its retained occurrence cursor;
/// this evaluator never reads the ambient clock.
pub fn nextAfter(rule: Rule, zone: Zone, after: deadlines.WallInstant, policy: SearchPolicy) outcomes.Result(?deadlines.WallInstant, Error) {
    if (!rule.valid() or !policy.valid()) return .{ .err = .invalid_policy };
    var local = switch (zone.vtable.to_local(zone.userdata, after)) {
        .ok => |value| value,
        .err => |problem| return .{ .err = problem },
    };
    for (0..policy.max_local_minutes) |_| {
        if (rule.matches(local)) {
            var cursor: ?deadlines.WallInstant = null;
            var selected: ?deadlines.WallInstant = null;
            var candidates: usize = 0;
            while (true) {
                const next = switch (zone.vtable.next_candidate(zone.userdata, local, cursor)) {
                    .ok => |value| value orelse break,
                    .err => |problem| return .{ .err = problem },
                };
                candidates = std.math.add(usize, candidates, 1) catch return .{ .err = .overflow };
                if (candidates > policy.max_candidates_per_local or
                    (cursor != null and next.unix_nanoseconds <= cursor.?.unix_nanoseconds))
                    return .{ .err = .zone_contract };
                const returned_local = switch (zone.vtable.to_local(zone.userdata, next)) {
                    .ok => |value| value,
                    .err => |problem| return .{ .err = problem },
                };
                if (!std.meta.eql(returned_local, local)) return .{ .err = .zone_contract };
                cursor = next;
                switch (policy.overlap) {
                    .first => {
                        if (next.unix_nanoseconds > after.unix_nanoseconds)
                            return .{ .ok = next };
                        break;
                    },
                    .last => selected = next,
                    .both => if (next.unix_nanoseconds > after.unix_nanoseconds)
                        return .{ .ok = next },
                }
            }
            if (policy.overlap == .last) if (selected) |last|
                if (last.unix_nanoseconds > after.unix_nanoseconds) return .{ .ok = last };
        }
        local = switch (local.next()) {
            .ok => |value| value,
            .err => |problem| return .{ .err = problem },
        };
    }
    return .{ .ok = null };
}
