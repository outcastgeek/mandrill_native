//! Transactional one-shot value. A pending read is part of the journal's
//! retry set, so completion wakes blocked transactions after commit.
const tx = @import("transaction.zig");

/// Specialize a transactional one-shot deferred by value type.
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// Returns (`type`): one-shot cell completed through the domain journal.
pub fn Deferred(comptime T: type) type {
    if (!tx.immutableValue(T)) @compileError("transactional deferred values must be immutable");
    return struct {
        const Self = @This();
        /// Deferred-cell observation: completed value or pending state.
        pub const Poll = union(enum) { ready: T, pending };
        reference: tx.Ref(?T),

        /// Create an empty transactional deferred cell in the supplied domain.
        /// Parameters:
        /// - `domain: *tx.Domain`: Transaction domain containing the cells and journal for this operation.
        /// Returns (`Self`): a one-shot cell whose completion is staged in that domain.
        pub fn init(domain: *tx.Domain) Self {
            return .{ .reference = tx.Ref(?T).init(domain, null) };
        }

        /// Observe the current completion state.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// Returns (`?Poll`): ready or pending status; `null` means the journal read failed.
        pub fn poll(self: *Self, journal: *tx.Domain.Journal) ?Poll {
            return switch (journal.read(?T, &self.reference)) {
                .ok => |stored| if (stored) |value| .{ .ready = value } else .pending,
                .err => null,
            };
        }

        /// Complete  and publish its typed outcome.
        ///
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// - `value: T`: one-shot value staged only if the cell is still pending.
        /// Returns (`?bool`): completion status; `null` means the journal read failed.
        pub fn complete(self: *Self, journal: *tx.Domain.Journal, value: T) ?bool {
            const stored = switch (journal.read(?T, &self.reference)) {
                .ok => |current| current,
                .err => return null,
            };
            if (stored != null) return false;
            journal.write(?T, &self.reference, value);
            return true;
        }
    };
}
