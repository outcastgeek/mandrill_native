//! Versioned observation of committed transactional values. Initial read and
//! the next-change wait use the same journal/commit synchronization boundary.
const effects = @import("../effect.zig");
const tx = @import("transaction.zig");
const TransactionError = @import("transaction_error.zig").Error;

pub fn SubscriptionRef(comptime T: type) type {
    if (!tx.immutableValue(T)) @compileError("observable transactional values must be immutable");
    return struct {
        const Self = @This();
        pub const AwaitCapture = struct { reference: *Self, version: u64 };
        reference: tx.Ref(T),

        pub fn init(domain: *tx.Domain, value: T) Self {
            return .{ .reference = tx.Ref(T).init(domain, value) };
        }

        pub fn read(self: *Self, journal: *tx.Domain.Journal) ?tx.Observed(T) {
            return switch (journal.readObserved(T, &self.reference)) {
                .ok => |value| value,
                .err => null,
            };
        }

        pub fn write(self: *Self, journal: *tx.Domain.Journal, value: T) void {
            journal.write(T, &self.reference, value);
        }

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
