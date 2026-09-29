//! Logical recurring entity occurrences. A schedule describes calendar and
//! overlap policy only; it neither reads the clock nor launches an OS timer.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const deadlines = @import("../deadline.zig");
const keys = @import("../primary_key.zig");
const cron = @import("../cron.zig");
const types = @import("cluster_types.zig");

/// Policy for missed scheduled firings, including bounded replay.
pub const CatchUp = union(enum) {
    skip_missed,
    replay_bounded: usize,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: CatchUp`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` for skip policy or a positive bounded replay count.
    pub fn valid(self: CatchUp) bool {
        return switch (self) {
            .skip_missed => true,
            .replay_bounded => |limit| limit > 0,
        };
    }
};

/// Policy for an invocation that overlaps an earlier run.
pub const Overlap = union(enum) {
    skip_if_running,
    queue,
    parallel_bounded: usize,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Overlap`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` for skip/queue policy or a positive parallel overlap bound.
    pub fn valid(self: Overlap) bool {
        return switch (self) {
            .skip_if_running, .queue => true,
            .parallel_bounded => |limit| limit > 0,
        };
    }
};

/// Invalid cluster schedule or underlying calendar evaluation failure.
pub const Error = union(enum) { invalid_schedule, calendar: cron.Error };

/// Stable versioned calendar schedule with explicit search, catch-up, and overlap policies.
pub const Schedule = struct {
    name: []const u8,
    version: u32,
    calendar: cron.Rule,
    search: cron.SearchPolicy,
    catch_up: CatchUp,
    overlap: Overlap,

    /// Construct a statically checked definition.
    /// Parameters:
    /// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition time.
    /// - `version: comptime u32`: Compile-time stable version used to reject incompatible encoded or replayed
    ///   data.
    /// - `calendar: comptime cron.Rule`: Compile-time calendar rule for this schedule.
    /// - `search: comptime cron.SearchPolicy`: Compile-time caller-selected, validated behavior or resource bounds; no hidden
    ///   default is installed.
    /// - `catch_up: comptime CatchUp`: Compile-time caller-selected, validated behavior or resource bounds; no
    ///   hidden default is installed.
    /// - `overlap: comptime Overlap`: Compile-time caller-selected, validated behavior or resource bounds; no hidden
    ///   default is installed.
    /// Returns (`Schedule`): A typed Schedule that computes its next delay and continuation decision.
    pub fn define(comptime name: []const u8, comptime version: u32, comptime calendar: cron.Rule, comptime search: cron.SearchPolicy, comptime catch_up: CatchUp, comptime overlap: Overlap) Schedule {
        const schedule = Schedule{ .name = name, .version = version, .calendar = calendar, .search = search, .catch_up = catch_up, .overlap = overlap };
        if (!schedule.valid()) @compileError("invalid recurring entity schedule");
        return schedule;
    }

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Schedule`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` when name, version, calendar, search, catch-up, and overlap policies all validate.
    pub fn valid(self: Schedule) bool {
        return self.name.len > 0 and std.unicode.utf8ValidateSlice(self.name) and
            self.version > 0 and self.calendar.valid() and self.search.valid() and
            self.catch_up.valid() and self.overlap.valid();
    }

    /// The caller injects `now` and persists the last occurrence cursor.
    /// `skip_missed` advances from now; bounded replay advances from the
    /// retained cursor, with its replay count enforced by the caller.
    ///
    /// Parameters:
    /// - `self: Schedule`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// - `zone: cron.Zone`: Time zone used to interpret the calendar rule.
    /// - `last: deadlines.WallInstant`: Caller-supplied time or cursor boundary used for ordering and retention.
    /// - `now: deadlines.WallInstant`: Caller-supplied time or cursor boundary used for ordering and retention.
    /// Returns (`outcomes.Result(?deadlines.WallInstant, Error)`): An ok/err Result; owned payloads must be released according to their type contract.
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
    ///
    /// Parameters:
    /// - `self: Schedule`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
    /// - `address: types.EntityAddress`: Stable identity used for matching, deduplication, or replay.
    /// - `due: deadlines.WallInstant`: Scheduled wall-clock instant for this operation.
    /// Returns (`outcomes.Exit(outcomes.OwnedSlice(u8), keys.Error)`): Carries `outcomes.OwnedSlice(u8)` on `.success` or `keys.Error` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
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
    /// Stable domain namespace used when encoding keys.
    pub const key_namespace = "effect.cluster.occurrence.v1";
};
