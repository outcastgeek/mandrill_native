//! Bounded transactional sequence backed by the shared mixed-type journal.
//! Capacity is a caller-selected comptime type parameter, not a hidden limit.
const tx = @import("transaction.zig");

pub fn Chunk(comptime T: type, comptime capacity: usize) type {
    if (capacity == 0) @compileError("transactional chunk capacity must be positive");
    if (!tx.immutableValue(T)) @compileError("transactional chunk elements must be immutable values");
    return struct {
        const Self = @This();
        const State = struct { items: [capacity]?T = [_]?T{null} ** capacity, length: usize = 0 };
        pub const Append = enum { appended, full };
        pub const At = union(enum) { value: T, out_of_bounds };

        reference: tx.Ref(State),

        pub fn init(domain: *tx.Domain) Self {
            return .{ .reference = tx.Ref(State).init(domain, .{}) };
        }

        pub fn len(self: *Self, journal: *tx.Domain.Journal) ?usize {
            return switch (journal.read(State, &self.reference)) {
                .ok => |state| state.length,
                .err => null,
            };
        }

        pub fn at(self: *Self, journal: *tx.Domain.Journal, index: usize) ?At {
            return switch (journal.read(State, &self.reference)) {
                .ok => |state| if (index < state.length) .{ .value = state.items[index].? } else .out_of_bounds,
                .err => null,
            };
        }

        pub fn append(self: *Self, journal: *tx.Domain.Journal, value: T) ?Append {
            var state = switch (journal.read(State, &self.reference)) {
                .ok => |current| current,
                .err => return null,
            };
            if (state.length == capacity) return .full;
            state.items[state.length] = value;
            state.length += 1;
            journal.write(State, &self.reference, state);
            return .appended;
        }

        pub fn set(self: *Self, journal: *tx.Domain.Journal, index: usize, value: T) ?bool {
            var state = switch (journal.read(State, &self.reference)) {
                .ok => |current| current,
                .err => return null,
            };
            if (index >= state.length) return false;
            state.items[index] = value;
            journal.write(State, &self.reference, state);
            return true;
        }

        pub fn removeAt(self: *Self, journal: *tx.Domain.Journal, index: usize) ?At {
            var state = switch (journal.read(State, &self.reference)) {
                .ok => |current| current,
                .err => return null,
            };
            if (index >= state.length) return .out_of_bounds;
            const removed = state.items[index].?;
            var cursor = index;
            while (cursor + 1 < state.length) : (cursor += 1) state.items[cursor] = state.items[cursor + 1];
            state.length -= 1;
            state.items[state.length] = null;
            journal.write(State, &self.reference, state);
            return .{ .value = removed };
        }
    };
}
