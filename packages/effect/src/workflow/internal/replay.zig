//! Pure replay decisions shared by independent and composed engines. The
//! interpreter never treats an uncertain write as a completed activity.
const std = @import("std");
const types = @import("../workflow_types.zig");

/// Replay lookup result selecting a recorded value or a new execution.
pub const LookupDecision = union(enum) {
    execute,
    replay: types.RecordedActivity,
    incompatible_version: struct { recorded: u32, expected: u32 },

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        if (self.* == .replay) self.replay.deinit();
        self.* = undefined;
    }
};

/// Decide whether a recorded activity matches the expected version.
/// Parameters:
/// - `recorded: ?types.RecordedActivity`: Previously stored value consulted during replay.
/// - `expected_version: u32`: Stable version used to reject incompatible encoded or replayed data.
/// Returns (`LookupDecision`): Replay lookup result for the recorded activity.
pub fn activity(recorded: ?types.RecordedActivity, expected_version: u32) LookupDecision {
    const value = recorded orelse return .execute;
    if (value.activity_version == expected_version) return .{ .replay = value };
    const recorded_version = value.activity_version;
    var discarded = value;
    discarded.deinit();
    return .{ .incompatible_version = .{ .recorded = recorded_version, .expected = expected_version } };
}

/// `AfterRecord` cases: `committed`, `read_back`, `conflict`, `stale_activation`.
pub const AfterRecord = enum {
    committed,
    read_back,
    conflict,
    stale_activation,
};

/// Classify the result of recording an activity for replay.
/// Parameters:
/// - `result: types.RecordResult`: activity result retained after its durable record is committed.
/// Returns (`AfterRecord`): Replay decision after an activity record attempt.
pub fn afterRecord(result: types.RecordResult) AfterRecord {
    return switch (result) {
        .recorded => .committed,
        .already_recorded, .possibly_recorded => .read_back,
        .conflicting_record => .conflict,
        .stale_activation => .stale_activation,
    };
}

/// `ReadBack` cases: `confirmed`, `missing`, `conflicting_version`, `conflicting_outcome`.
pub const ReadBack = enum {
    confirmed,
    missing,
    conflicting_version,
    conflicting_outcome,
};

/// Confirm that a persisted activity matches the encoded Exit.
/// Parameters:
/// - `recorded: ?*const types.RecordedActivity`: Previously stored value consulted during replay.
/// - `expected_version: u32`: Stable version used to reject incompatible encoded or replayed data.
/// - `encoded_exit: []const u8`: Borrowed encoded terminal outcome.
/// Returns (`ReadBack`): Result of confirming a stored activity against its encoded Exit.
pub fn confirm(recorded: ?*const types.RecordedActivity, expected_version: u32, encoded_exit: []const u8) ReadBack {
    const value = recorded orelse return .missing;
    if (value.activity_version != expected_version) return .conflicting_version;
    return if (std.mem.eql(u8, value.outcome.items, encoded_exit)) .confirmed else .conflicting_outcome;
}
