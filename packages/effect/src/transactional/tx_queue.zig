//! Bounded FIFO queue whose state commits with every other journal write.
const tx = @import("transaction.zig");

/// Build a FIFO queue whose contents are staged in a transaction journal and
/// become visible only when that journal commits. The queue stores immutable
/// `T` values; a zero capacity or mutable `T` fails at compilation.
/// Parameters:
/// - `T: comptime type`: immutable type of each queued item.
/// - `capacity: comptime usize`: positive maximum number of buffered items.
/// Returns (`type`): queue type with `offer`, `poll`, `take`, and `len`
/// operations; specializing the type does not allocate or start a transaction.
pub fn Queue(comptime T: type, comptime capacity: usize) type {
    if (capacity == 0) @compileError("transactional queue capacity must be positive");
    if (!tx.immutableValue(T)) @compileError("transactional queue elements must be immutable values");
    return struct {
        const Self = @This();
        const State = struct { items: [capacity]?T = [_]?T{null} ** capacity, head: usize = 0, length: usize = 0 };
        /// Offer result: `accepted` stages the item in the journal; `full`
        /// leaves the queue unchanged when it has reached `capacity`.
        pub const Offered = enum { accepted, full };
        /// Poll result: `value` holds the removed oldest item; `empty` means
        /// there was nothing to remove.
        pub const Taken = union(enum) { value: T, empty };

        reference: tx.Ref(State),

        /// Create an empty queue in the supplied transaction domain.
        /// Parameters:
        /// - `domain: *tx.Domain`: domain that owns the queue's transactional reference;
        ///   it must outlive the queue.
        /// Returns (`Self`): queue handle whose state participates in that domain.
        pub fn init(domain: *tx.Domain) Self {
            return .{ .reference = tx.Ref(State).init(domain, .{}) };
        }

        fn advance(index: usize, distance: usize) usize {
            const remaining = capacity - index;
            return if (distance < remaining) index + distance else distance - remaining;
        }

        /// Read the queue length as seen by this journal, including its staged writes.
        /// Parameters:
        /// - `self: *Self`: queue whose current transactional length is read.
        /// - `journal: *tx.Domain.Journal`: active journal for the queue's domain.
        /// Returns (`?usize`): staged length, or `null` if the journal read failed.
        pub fn len(self: *Self, journal: *tx.Domain.Journal) ?usize {
            return switch (journal.read(State, &self.reference)) {
                .ok => |state| state.length,
                .err => null,
            };
        }

        /// Stage an item at the tail without overwriting a full queue.
        /// Parameters:
        /// - `self: *Self`: queue to update in the journal.
        /// - `journal: *tx.Domain.Journal`: active journal for the queue's domain.
        /// - `value: T`: immutable item staged only when space is available.
        /// Returns (`?Offered`): `accepted` if staged, `full` if unchanged,
        /// or `null` if the journal read failed.
        pub fn offer(self: *Self, journal: *tx.Domain.Journal, value: T) ?Offered {
            var state = switch (journal.read(State, &self.reference)) {
                .ok => |current| current,
                .err => return null,
            };
            if (state.length == capacity) return .full;
            state.items[advance(state.head, state.length)] = value;
            state.length += 1;
            journal.write(State, &self.reference, state);
            return .accepted;
        }

        /// Stage removal of the oldest item, if any.
        /// Parameters:
        /// - `self: *Self`: queue to update in the journal.
        /// - `journal: *tx.Domain.Journal`: active journal for the queue's domain.
        /// Returns (`?Taken`): `.value` for the removed oldest item, `.empty`
        /// when unchanged, or `null` if the journal read failed.
        pub fn poll(self: *Self, journal: *tx.Domain.Journal) ?Taken {
            var state = switch (journal.read(State, &self.reference)) {
                .ok => |current| current,
                .err => return null,
            };
            if (state.length == 0) return .empty;
            const value = state.items[state.head].?;
            state.items[state.head] = null;
            state.head = advance(state.head, 1);
            state.length -= 1;
            journal.write(State, &self.reference, state);
            return .{ .value = value };
        }

        /// Attempt to take the oldest item. This is the same immediate operation
        /// as `poll`; a transaction callback can turn `.empty` into `retry` so
        /// `Domain.atomically` waits for a queue change without a lost wakeup.
        ///
        /// Parameters:
        /// - `self: *Self`: queue to read or update in the journal.
        /// - `journal: *tx.Domain.Journal`: active journal for the queue's domain.
        /// Returns (`?Taken`): the same `.value`, `.empty`, or `null` result as `poll`.
        pub fn take(self: *Self, journal: *tx.Domain.Journal) ?Taken {
            return self.poll(journal);
        }
    };
}
