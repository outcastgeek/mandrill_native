//! Pure replay decisions shared by independent and composed engines. The
//! interpreter never treats an uncertain write as a completed activity.
const std = @import("std");
const types = @import("../workflow_types.zig");

pub const LookupDecision = union(enum) {
    execute,
    replay: types.RecordedActivity,
    incompatible_version: struct { recorded: u32, expected: u32 },

    pub fn deinit(self: *@This()) void {
        if (self.* == .replay) self.replay.deinit();
        self.* = undefined;
    }
};

pub fn activity(recorded: ?types.RecordedActivity, expected_version: u32) LookupDecision {
    const value = recorded orelse return .execute;
    if (value.activity_version == expected_version) return .{ .replay = value };
    const recorded_version = value.activity_version;
    var discarded = value;
    discarded.deinit();
    return .{ .incompatible_version = .{ .recorded = recorded_version, .expected = expected_version } };
}

pub const AfterRecord = enum {
    committed,
    read_back,
    conflict,
    stale_activation,
};

pub fn afterRecord(result: types.RecordResult) AfterRecord {
    return switch (result) {
        .recorded => .committed,
        .already_recorded, .possibly_recorded => .read_back,
        .conflicting_record => .conflict,
        .stale_activation => .stale_activation,
    };
}

pub const ReadBack = enum {
    confirmed,
    missing,
    conflicting_version,
    conflicting_outcome,
};

pub fn confirm(recorded: ?*const types.RecordedActivity, expected_version: u32, encoded_exit: []const u8) ReadBack {
    const value = recorded orelse return .missing;
    if (value.activity_version != expected_version) return .conflicting_version;
    return if (std.mem.eql(u8, value.outcome.items, encoded_exit)) .confirmed else .conflicting_outcome;
}
