//! Data-only workflow identities and states. Code is registered separately;
//! execution and activity histories never contain live functions or handles.
const std = @import("std");
const keys = @import("../primary_key.zig");
const outcomes = @import("../outcome.zig");
const deadlines = @import("../deadline.zig");

pub const Bytes = outcomes.OwnedSlice(u8);

pub const Definition = struct {
    name: []const u8,
    version: u32,
    payload_schema_version: u32,
    success_schema_version: u32,
    error_schema_version: u32,

    pub fn valid(self: Definition) bool {
        return self.name.len > 0 and std.unicode.utf8ValidateSlice(self.name) and
            self.version > 0 and self.payload_schema_version > 0 and
            self.success_schema_version > 0 and self.error_schema_version > 0;
    }
};

pub const ExecutionId = struct {
    encoded: []const u8,

    pub fn validate(self: ExecutionId) outcomes.Result(void, keys.Error) {
        return switch (keys.inspectEncoded(self.encoded)) {
            .ok => .{ .ok = {} },
            .err => |problem| .{ .err = problem },
        };
    }
};

pub const ActivityId = struct {
    encoded: []const u8,
    attempt: u64,

    pub fn validate(self: ActivityId) outcomes.Result(void, keys.Error) {
        if (self.attempt == 0) return .{ .err = .malformed };
        return switch (keys.inspectEncoded(self.encoded)) {
            .ok => .{ .ok = {} },
            .err => |problem| .{ .err = problem },
        };
    }
};

/// `generation` fences stale activations. It is engine contract data, not a
/// prescribed owner election or storage implementation.
pub const Activation = struct {
    execution: ExecutionId,
    owner_id: []const u8,
    generation: u64,

    pub fn valid(self: Activation) bool {
        return self.execution.validate() == .ok and self.owner_id.len > 0 and self.generation > 0;
    }
};

pub const ActivationLease = struct {
    pub const effect_owned_payload = true;
    execution_key: Bytes,
    owner_id: Bytes,
    generation: u64,

    pub fn activation(self: *const @This()) Activation {
        return .{
            .execution = .{ .encoded = self.execution_key.items },
            .owner_id = self.owner_id.items,
            .generation = self.generation,
        };
    }

    pub fn deinit(self: *@This()) void {
        self.execution_key.deinit();
        self.owner_id.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        var execution_key = try self.execution_key.clone();
        errdefer execution_key.deinit();
        return .{
            .execution_key = execution_key,
            .owner_id = try self.owner_id.clone(),
            .generation = self.generation,
        };
    }
};

pub const ActivationResult = union(enum) {
    pub const effect_owned_payload = true;
    activated: ActivationLease,
    unavailable,
    already_terminal,

    pub fn deinit(self: *@This()) void {
        if (self.* == .activated) self.activated.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .activated => |value| .{ .activated = try value.clone() },
            else => self.*,
        };
    }
};

pub const ActivationPolicy = struct {
    lease_until: deadlines.Deadline,

    pub fn valid(self: ActivationPolicy) bool {
        return self.lease_until != .never;
    }
};

/// A completed activity stores a full versioned Exit encoding, not merely a
/// success value. An activity that suspended has no completed record and may
/// run again. New retries use a new attempt; replay reads the same attempt.
pub const RecordedActivity = struct {
    pub const effect_owned_payload = true;
    outcome: Bytes,
    activity_version: u32,

    pub fn deinit(self: *@This()) void {
        self.outcome.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{ .outcome = try self.outcome.clone(), .activity_version = self.activity_version };
    }
};

pub const ExecutionState = union(enum) {
    pub const effect_owned_payload = true;
    absent,
    queued,
    running,
    cancellation_requested,
    suspended,
    completed: Bytes,
    terminal_cancelled,

    pub fn deinit(self: *@This()) void {
        if (self.* == .completed) self.completed.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .completed => |data| .{ .completed = try data.clone() },
            else => self.*,
        };
    }
};

pub const Submission = struct {
    definition: Definition,
    execution: ExecutionId,
    encoded_payload: []const u8,
};

pub const SubmitResult = enum { accepted, already_present, possibly_accepted };
pub const ControlResult = enum { recorded, already_recorded, absent, possibly_recorded };
pub const RecordResult = enum { recorded, already_recorded, conflicting_record, stale_activation, possibly_recorded };
pub const CompletionResult = enum { committed, already_committed, conflicting_completion, stale_activation, possibly_recorded };
pub const SuspensionResult = enum { recorded, already_recorded, stale_activation, possibly_recorded };
