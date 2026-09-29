//! Accessing the engine can fail without changing a completed workflow Exit.
const outcomes = @import("../outcome.zig");

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

pub fn Completed(comptime Success: type, comptime DomainError: type) type {
    return struct {
        pub const effect_owned_payload = true;
        execution_version: u32,
        outcome: outcomes.Exit(Success, DomainError),

        pub fn deinit(self: *@This()) void {
            self.outcome.deinit();
            self.* = undefined;
        }

        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            const copy = self.outcome.clone();
            if (copy == .die and copy.die.kind == .out_of_memory and !(self.outcome == .die and self.outcome.die.kind == .out_of_memory)) return error.OutOfMemory;
            return .{ .execution_version = self.execution_version, .outcome = copy };
        }
    };
}
