//! Reentrant ownership is a caller-provided logical execution identity.
//! Native thread IDs are deliberately absent because execution can migrate.
const std = @import("std");
const tx = @import("transaction.zig");

/// Specialize a transactional reentrant lock by owner identity type.
/// Parameters:
/// - `Owner: comptime type`: Compile-time owner identity type used by the reentrant lock.
/// Returns (`type`): journal-backed lock keyed by the selected owner identity type.
pub fn ReentrantLock(comptime Owner: type) type {
    if (!tx.immutableValue(Owner)) @compileError("lock owners must be stable immutable values");
    return struct {
        const Self = @This();
        const State = struct { owner: ?Owner = null, depth: usize = 0 };
        /// `Acquire` cases: `acquired`, `reentered`, `busy`, `depth_exhausted`.
        pub const Acquire = enum { acquired, reentered, busy, depth_exhausted };
        /// `Release` cases: `released`, `still_owned`, `not_owner`.
        pub const Release = enum { released, still_owned, not_owner };

        reference: tx.Ref(State),

        /// Create a transactional reentrant lock in the supplied domain.
        /// Parameters:
        /// - `domain: *tx.Domain`: Transaction domain containing the cells and journal for this operation.
        /// Returns (`Self`): a lock whose ownership changes commit with the journal.
        pub fn init(domain: *tx.Domain) Self {
            return .{ .reference = tx.Ref(State).init(domain, .{}) };
        }

        /// Acquire a resource from the selected capability.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// - `owner: Owner`: Owner identity used for claim and fencing checks.
        /// Returns (`?Acquire`): lock acquisition status; `null` means the journal read failed.
        pub fn acquire(self: *Self, journal: *tx.Domain.Journal, owner: Owner) ?Acquire {
            var state = switch (journal.read(State, &self.reference)) {
                .ok => |current| current,
                .err => return null,
            };
            if (state.owner) |current| {
                if (!std.meta.eql(current, owner)) return .busy;
                if (state.depth == std.math.maxInt(usize)) return .depth_exhausted;
                state.depth += 1;
                journal.write(State, &self.reference, state);
                return .reentered;
            }
            state.owner = owner;
            state.depth = 1;
            journal.write(State, &self.reference, state);
            return .acquired;
        }

        /// Return or release the owned resource.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// - `owner: Owner`: Owner identity used for claim and fencing checks.
        /// Returns (`?Release`): lock release status; `null` means the journal read failed.
        pub fn release(self: *Self, journal: *tx.Domain.Journal, owner: Owner) ?Release {
            var state = switch (journal.read(State, &self.reference)) {
                .ok => |current| current,
                .err => return null,
            };
            const current = state.owner orelse return .not_owner;
            if (!std.meta.eql(current, owner)) return .not_owner;
            state.depth -= 1;
            if (state.depth == 0) {
                state.owner = null;
                journal.write(State, &self.reference, state);
                return .released;
            }
            journal.write(State, &self.reference, state);
            return .still_owned;
        }
    };
}
