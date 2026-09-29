//! Accessing the engine can fail without changing a completed workflow Exit.
const outcomes = @import("../outcome.zig");

/// `OperationError` is a tagged choice.
pub const OperationError = union(enum) {
    unavailable,
    definition_conflict,
    incompatible_definition,
    incompatible_schema,
    malformed_record,
    execution_missing,
    ownership_lost,
    uncertain_activity_record,
    uncertain_control_request,
    deadline_invalid,
    queue_dead_letter,
    generation_exhausted,
};

/// Specialize the typed completed outcome for workflow operations.
/// Parameters:
/// - `Success: comptime type`: Success payload type represented by the returned component.
/// - `DomainError: comptime type`: Application failure type produced by domain-level operations.
/// Returns (`type`): typed completed workflow outcome that preserves success or expected failure.
pub fn Completed(comptime Success: type, comptime DomainError: type) type {
    return struct {
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        execution_version: u32,
        outcome: outcomes.Exit(Success, DomainError),

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
            const copy = self.outcome.clone();
            if (copy == .die and copy.die.kind == .out_of_memory and !(self.outcome == .die and self.outcome.die.kind == .out_of_memory)) return error.OutOfMemory;
            return .{ .execution_version = self.execution_version, .outcome = copy };
        }
    };
}
