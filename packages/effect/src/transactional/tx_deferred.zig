//! Transactional one-shot value. A pending read is part of the journal's
//! retry set, so completion wakes blocked transactions after commit.
const tx = @import("transaction.zig");

pub fn Deferred(comptime T: type) type {
    if (!tx.immutableValue(T)) @compileError("transactional deferred values must be immutable");
    return struct {
        const Self = @This();
        pub const Poll = union(enum) { ready: T, pending };
        reference: tx.Ref(?T),

        pub fn init(domain: *tx.Domain) Self {
            return .{ .reference = tx.Ref(?T).init(domain, null) };
        }

        pub fn poll(self: *Self, journal: *tx.Domain.Journal) ?Poll {
            return switch (journal.read(?T, &self.reference)) {
                .ok => |stored| if (stored) |value| .{ .ready = value } else .pending,
                .err => null,
            };
        }

        /// Returns false when another completion was already staged/committed.
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
