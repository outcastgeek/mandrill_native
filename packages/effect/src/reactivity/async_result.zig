//! Reactive state retains typed failure classification and previous success.
const outcomes = @import("../outcome.zig");

/// Specialize the reactivity failure variants from their typed channels.
/// Parameters:
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): typed failure state for an asynchronous observable result.
pub fn Failure(comptime E: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        domain: E,
        defect: outcomes.Defect,
        interrupted,
        finalization_failed: outcomes.Finalization(E),

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            switch (self.*) {
                .domain => outcomes.discardPayload(&self.domain),
                .finalization_failed => {
                    var owned: outcomes.Exit(void, E) = .{ .finalization_failed = self.finalization_failed };
                    owned.deinit();
                },
                else => {},
            }
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .domain => |value| .{ .domain = try outcomes.clonePayload(value) },
                .finalization_failed => |value| blk: {
                    const wrapped: outcomes.Exit(void, E) = .{ .finalization_failed = value };
                    const copy = wrapped.clone();
                    if (copy == .die) return error.OutOfMemory;
                    break :blk .{ .finalization_failed = copy.finalization_failed };
                },
                else => self.*,
            };
        }
    };
}

/// Specialize observable pending, successful, and failed async query states.
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): observable pending, successful, or failed result retaining prior value where applicable.
pub fn AsyncResult(comptime A: type, comptime E: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;

        initial,
        waiting: ?A,
        success: A,
        refreshing: A,
        failure: struct { cause: Failure(E), previous: ?A },

        /// Moves the completed Exit into this state. Previous success is
        /// retained only for failures; a new success releases the old owner.
        ///
        /// Parameters:
        /// - `result: outcomes.Exit(A, E)`: completed Exit moved into the observable state; do not release another copy.
        /// - `previous: ?A`: Optional previously observed value.
        /// Returns (`Self`): async-result state owning the supplied terminal outcome and retaining the previous value where appropriate.
        pub fn fromExit(result: outcomes.Exit(A, E), previous: ?A) Self {
            return switch (result) {
                .success => |value| blk: {
                    var old = previous;
                    outcomes.discardPayload(&old);
                    break :blk .{ .success = value };
                },
                .fail => |err| .{ .failure = .{ .cause = .{ .domain = err }, .previous = previous } },
                .die => |defect| .{ .failure = .{ .cause = .{ .defect = defect }, .previous = previous } },
                .interrupted => .{ .failure = .{ .cause = .interrupted, .previous = previous } },
                .finalization_failed => |details| .{ .failure = .{ .cause = .{ .finalization_failed = details }, .previous = previous } },
            };
        }

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            switch (self.*) {
                .waiting => outcomes.discardPayload(&self.waiting),
                .success => outcomes.discardPayload(&self.success),
                .refreshing => outcomes.discardPayload(&self.refreshing),
                .failure => {
                    self.failure.cause.deinit();
                    outcomes.discardPayload(&self.failure.previous);
                },
                .initial => {},
            }
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .initial => .initial,
                .waiting => |value| .{ .waiting = try outcomes.clonePayload(value) },
                .success => |value| .{ .success = try outcomes.clonePayload(value) },
                .refreshing => |value| .{ .refreshing = try outcomes.clonePayload(value) },
                .failure => |value| blk: {
                    var cause = try value.cause.clone();
                    errdefer cause.deinit();
                    break :blk .{ .failure = .{ .cause = cause, .previous = try outcomes.clonePayload(value.previous) } };
                },
            };
        }
    };
}
