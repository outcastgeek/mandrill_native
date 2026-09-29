//! Versioned observation of committed transactional values. Initial read and
//! the next-change wait use the same journal/commit synchronization boundary.
const effects = @import("../effect.zig");
const tx = @import("transaction.zig");
const TransactionError = @import("transaction_error.zig").Error;

/// Specialize a transactional reference with observable committed changes.
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// Returns (`type`): transactional reference whose committed revisions are observable.
pub fn SubscriptionRef(comptime T: type) type {
    if (!tx.immutableValue(T)) @compileError("observable transactional values must be immutable");
    return struct {
        const Self = @This();
        /// Borrowed reference and revision used while waiting for a later commit.
        pub const AwaitCapture = struct { reference: *Self, version: u64 };
        reference: tx.Ref(T),

        /// Create an observable transactional reference with its initial value.
        /// Parameters:
        /// - `domain: *tx.Domain`: Transaction domain containing the cells and journal for this operation.
        /// - `value: T`: initial committed value and revision source.
        /// Returns (`Self`): a reference whose committed revisions can be observed.
        pub fn init(domain: *tx.Domain, value: T) Self {
            return .{ .reference = tx.Ref(T).init(domain, value) };
        }

        /// Read through the selected capability.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// Returns (`?tx.Observed(T)`): current value and committed revision, or `null` on journal read failure.
        pub fn read(self: *Self, journal: *tx.Domain.Journal) ?tx.Observed(T) {
            return switch (journal.readObserved(T, &self.reference)) {
                .ok => |value| value,
                .err => null,
            };
        }

        /// Write through the selected capability.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// - `value: T`: replacement staged for commit and later observation.
        /// Returns (`void`): no value.
        pub fn write(self: *Self, journal: *tx.Domain.Journal, value: T) void {
            journal.write(T, &self.reference, value);
        }

        /// Wait for a committed change after the supplied revision.
        /// Parameters:
        /// - `R: comptime type`: Caller-defined environment type supplying the dependencies needed by this operation.
        /// - `capture: *const AwaitCapture`: Operation-specific state retained until the callback completes.
        /// - `select_domain: comptime fn (*const R) *tx.Domain`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`effects.Effect(tx.Observed(T), TransactionError(void), R)`): committed value and revision newer than the supplied revision.
        pub fn awaitAfter(
            comptime R: type,
            capture: *const AwaitCapture,
            comptime select_domain: fn (*const R) *tx.Domain,
        ) effects.Effect(tx.Observed(T), TransactionError(void), R) {
            return tx.Domain.atomically(tx.Observed(T), void, R, AwaitCapture, capture, select_domain, struct {
                fn evaluate(input: *const AwaitCapture, journal: *tx.Domain.Journal) tx.Decision(tx.Observed(T), void) {
                    const observed = input.reference.read(journal) orelse return .retry;
                    return if (observed.version > input.version) .{ .commit = observed } else .retry;
                }
            }.evaluate);
        }
    };
}
