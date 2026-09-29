//! Data-only workflow identities and states. Code is registered separately;
//! execution and activity histories never contain live functions or handles.
const std = @import("std");
const keys = @import("../primary_key.zig");
const outcomes = @import("../outcome.zig");
const deadlines = @import("../deadline.zig");

/// Owned bytes buffer paired with its allocator; release it with `deinit`.
pub const Bytes = outcomes.OwnedSlice(u8);

/// Stable workflow identity and the versions of its payload, success, and error schemas. `valid`
/// rejects missing names and zero versions.
pub const Definition = struct {
    name: []const u8,
    version: u32,
    payload_schema_version: u32,
    success_schema_version: u32,
    error_schema_version: u32,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Definition`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` for nonempty UTF-8 name and positive workflow/schema versions.
    pub fn valid(self: Definition) bool {
        return self.name.len > 0 and std.unicode.utf8ValidateSlice(self.name) and
            self.version > 0 and self.payload_schema_version > 0 and
            self.success_schema_version > 0 and self.error_schema_version > 0;
    }
};

/// Borrowed encoded PrimaryKey identifying one workflow execution. Keep `encoded` alive while the
/// identity is in use; `validate` checks its wire form.
pub const ExecutionId = struct {
    encoded: []const u8,

    /// Validate the value against its declared contract.
    /// Parameters:
    /// - `self: ExecutionId`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`outcomes.Result(void, keys.Error)`): An ok/err Result; owned payloads must be released according to their type contract.
    pub fn validate(self: ExecutionId) outcomes.Result(void, keys.Error) {
        return switch (keys.inspectEncoded(self.encoded)) {
            .ok => .{ .ok = {} },
            .err => |problem| .{ .err = problem },
        };
    }
};

/// Borrowed encoded activity key plus a nonzero attempt number. Both identify the record used for
/// replay and deduplication.
pub const ActivityId = struct {
    encoded: []const u8,
    attempt: u64,

    /// Validate the value against its declared contract.
    /// Parameters:
    /// - `self: ActivityId`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`outcomes.Result(void, keys.Error)`): An ok/err Result; owned payloads must be released according to their type contract.
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

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Activation`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` when execution identity, owner, and generation are valid.
    pub fn valid(self: Activation) bool {
        return self.execution.validate() == .ok and self.owner_id.len > 0 and self.generation > 0;
    }
};

/// Owned activation identity returned by the engine. It retains the execution key and owner bytes;
/// release them with `deinit`.
pub const ActivationLease = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    execution_key: Bytes,
    owner_id: Bytes,
    generation: u64,

    /// Borrow the activation identity from this lease.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`Activation`): Borrowed activation identity derived from this lease.
    pub fn activation(self: *const @This()) Activation {
        return .{
            .execution = .{ .encoded = self.execution_key.items },
            .owner_id = self.owner_id.items,
            .generation = self.generation,
        };
    }

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        self.execution_key.deinit();
        self.owner_id.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
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

/// Result of claiming an execution: an owned activation lease, unavailable
/// execution, or an execution that is already terminal.
pub const ActivationResult = union(enum) {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    activated: ActivationLease,
    unavailable,
    already_terminal,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        if (self.* == .activated) self.activated.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .activated => |value| .{ .activated = try value.clone() },
            else => self.*,
        };
    }
};

/// Caller-selected bound and lease policy governing how an execution is activated.
pub const ActivationPolicy = struct {
    lease_until: deadlines.Deadline,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: ActivationPolicy`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` when activation lease has a finite deadline.
    pub fn valid(self: ActivationPolicy) bool {
        return self.lease_until != .never;
    }
};

/// A completed activity stores a full versioned Exit encoding, not merely a
/// success value. An activity that suspended has no completed record and may
/// run again. New retries use a new attempt; replay reads the same attempt.
pub const RecordedActivity = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    outcome: Bytes,
    activity_version: u32,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        self.outcome.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{ .outcome = try self.outcome.clone(), .activity_version = self.activity_version };
    }
};

/// Observed lifecycle state of a workflow execution, including pending, running, suspended, and
/// terminal forms.
pub const ExecutionState = union(enum) {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    absent,
    queued,
    running,
    cancellation_requested,
    suspended,
    completed: Bytes,
    terminal_cancelled,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        if (self.* == .completed) self.completed.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .completed => |data| .{ .completed = try data.clone() },
            else => self.*,
        };
    }
};

/// Borrowed workflow identity and encoded payload submitted to an engine; providers copy buffers they
/// retain.
pub const Submission = struct {
    definition: Definition,
    execution: ExecutionId,
    encoded_payload: []const u8,
};

/// Submission acknowledgement; `possibly_accepted` requires lookup before retrying an external
/// submission.
pub const SubmitResult = enum { accepted, already_present, possibly_accepted };
/// Outcome of a workflow control command. `possibly_recorded` requires reconciliation before retry.
pub const ControlResult = enum { recorded, already_recorded, absent, possibly_recorded };
/// Outcome of recording an activity under its activation fence, including conflict and ambiguous-write
/// states.
pub const RecordResult = enum { recorded, already_recorded, conflicting_record, stale_activation, possibly_recorded };
/// Outcome of completing an execution under its activation fence, including conflicts and ambiguous
/// writes.
pub const CompletionResult = enum { committed, already_committed, conflicting_completion, stale_activation, possibly_recorded };
/// Outcome of suspending an execution under its activation fence, including stale and ambiguous
/// transitions.
pub const SuspensionResult = enum { recorded, already_recorded, stale_activation, possibly_recorded };
