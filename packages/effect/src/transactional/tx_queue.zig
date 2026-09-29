//! Bounded FIFO queue whose state commits with every other journal write.
const tx = @import("transaction.zig");

pub fn Queue(comptime T: type, comptime capacity: usize) type {
    if (capacity == 0) @compileError("transactional queue capacity must be positive");
    if (!tx.immutableValue(T)) @compileError("transactional queue elements must be immutable values");
    return struct {
        const Self = @This();
        const State = struct { items: [capacity]?T = [_]?T{null} ** capacity, head: usize = 0, length: usize = 0 };
        pub const Offered = enum { accepted, full };
        pub const Taken = union(enum) { value: T, empty };

        reference: tx.Ref(State),

        pub fn init(domain: *tx.Domain) Self {
            return .{ .reference = tx.Ref(State).init(domain, .{}) };
        }

        fn advance(index: usize, distance: usize) usize {
            const remaining = capacity - index;
            return if (distance < remaining) index + distance else distance - remaining;
        }

        pub fn len(self: *Self, journal: *tx.Domain.Journal) ?usize {
            return switch (journal.read(State, &self.reference)) {
                .ok => |state| state.length,
                .err => null,
            };
        }

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

        /// A transaction callback returns `retry` when the result is empty;
        /// Domain.atomically registers its read set without a lost wakeup.
        pub fn take(self: *Self, journal: *tx.Domain.Journal) ?Taken {
            return self.poll(journal);
        }
    };
}
