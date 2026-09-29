//! Logical recurring entity occurrences. A schedule describes calendar and
//! overlap policy only; it neither reads the clock nor launches an OS timer.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const deadlines = @import("../deadline.zig");
const keys = @import("../primary_key.zig");
const cron = @import("../cron.zig");
const types = @import("cluster_types.zig");

pub const CatchUp = union(enum) {
    skip_missed,
    replay_bounded: usize,

    pub fn valid(self: CatchUp) bool {
        return switch (self) {
            .skip_missed => true,
            .replay_bounded => |limit| limit > 0,
        };
    }
};

pub const Overlap = union(enum) {
    skip_if_running,
    queue,
    parallel_bounded: usize,

    pub fn valid(self: Overlap) bool {
        return switch (self) {
            .skip_if_running, .queue => true,
            .parallel_bounded => |limit| limit > 0,
        };
    }
};

pub const Error = union(enum) { invalid_schedule, calendar: cron.Error };

pub const Schedule = struct {
    name: []const u8,
    version: u32,
    calendar: cron.Rule,
    search: cron.SearchPolicy,
    catch_up: CatchUp,
    overlap: Overlap,

    pub fn define(comptime name: []const u8, comptime version: u32, comptime calendar: cron.Rule, comptime search: cron.SearchPolicy, comptime catch_up: CatchUp, comptime overlap: Overlap) Schedule {
        const schedule = Schedule{ .name = name, .version = version, .calendar = calendar, .search = search, .catch_up = catch_up, .overlap = overlap };
        if (!schedule.valid()) @compileError("invalid recurring entity schedule");
        return schedule;
    }

    pub fn valid(self: Schedule) bool {
        return self.name.len > 0 and std.unicode.utf8ValidateSlice(self.name) and
            self.version > 0 and self.calendar.valid() and self.search.valid() and
            self.catch_up.valid() and self.overlap.valid();
    }

    /// The caller injects `now` and persists the last occurrence cursor.
    /// `skip_missed` advances from now; bounded replay advances from the
    /// retained cursor, with its replay count enforced by the caller.
    pub fn nextDue(self: Schedule, zone: cron.Zone, last: deadlines.WallInstant, now: deadlines.WallInstant) outcomes.Result(?deadlines.WallInstant, Error) {
        if (!self.valid()) return .{ .err = .invalid_schedule };
        const after = switch (self.catch_up) {
            .skip_missed => if (now.unix_nanoseconds > last.unix_nanoseconds) now else last,
            .replay_bounded => last,
        };
        return switch (cron.nextAfter(self.calendar, zone, after, self.search)) {
            .ok => |instant| .{ .ok = instant },
            .err => |problem| .{ .err = .{ .calendar = problem } },
        };
    }

    /// An occurrence ID is a canonical, length-delimited key over logical
    /// entity, schedule identity and exact due instant. Its namespace is a
    /// published wire identity, not a process hash or tuning value.
    pub fn occurrenceId(self: Schedule, env: anytype, address: types.EntityAddress, due: deadlines.WallInstant) outcomes.Exit(outcomes.OwnedSlice(u8), keys.Error) {
        if (!self.valid() or !address.valid()) return .{ .fail = .malformed };
        var timestamp: [@sizeOf(i128)]u8 = undefined;
        var bits: u128 = @bitCast(due.unix_nanoseconds);
        var index = timestamp.len;
        while (index > 0) {
            index -= 1;
            timestamp[index] = @truncate(bits);
            bits >>= @bitSizeOf(u8);
        }
        const parts = [_]keys.Part{
            .{ .text = address.group },
            .{ .bytes = address.entity_key },
            .{ .text = self.name },
            .{ .unsigned = self.version },
            .{ .bytes = &timestamp },
        };
        return keys.encodeDirect(env, OccurrenceKey.key_namespace, &parts);
    }
};

const OccurrenceKey = struct {
    pub const key_namespace = "effect.cluster.occurrence.v1";
};
