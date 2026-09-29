//! Bounded transactional sequence backed by the shared mixed-type journal.
//! Capacity is a caller-selected comptime type parameter, not a hidden limit.
const tx = @import("transaction.zig");

/// Specialize a bounded transactional chunk by item type and capacity.
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// - `capacity: comptime usize`: Compile-time caller-selected bound or count; zero/overflow handling follows this
///   API’s validation.
/// Returns (`type`): bounded transactional sequence of `T` values with journaled updates.
pub fn Chunk(comptime T: type, comptime capacity: usize) type {
    if (capacity == 0) @compileError("transactional chunk capacity must be positive");
    if (!tx.immutableValue(T)) @compileError("transactional chunk elements must be immutable values");
    return struct {
        const Self = @This();
        const State = struct { items: [capacity]?T = [_]?T{null} ** capacity, length: usize = 0 };
        /// `Append` cases: `appended`, `full`.
        pub const Append = enum { appended, full };
        /// Indexed chunk read yielding a value or out-of-range state.
        pub const At = union(enum) { value: T, out_of_bounds };

        reference: tx.Ref(State),

        /// Create an empty transactional chunk in the supplied domain.
        /// Parameters:
        /// - `domain: *tx.Domain`: Transaction domain containing the cells and journal for this operation.
        /// Returns (`Self`): a chunk whose changes commit with that domain’s journal.
        pub fn init(domain: *tx.Domain) Self {
            return .{ .reference = tx.Ref(State).init(domain, .{}) };
        }

        /// Read the number of elements.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// Returns (`?usize`): staged chunk length; `null` means the journal read failed.
        pub fn len(self: *Self, journal: *tx.Domain.Journal) ?usize {
            return switch (journal.read(State, &self.reference)) {
                .ok => |state| state.length,
                .err => null,
            };
        }

        /// Construct or access the value at the selected position.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// - `index: usize`: Position in the collection or definition table.
        /// Returns (`?At`): indexed value or out-of-range result; `null` means the journal read failed.
        pub fn at(self: *Self, journal: *tx.Domain.Journal, index: usize) ?At {
            return switch (journal.read(State, &self.reference)) {
                .ok => |state| if (index < state.length) .{ .value = state.items[index].? } else .out_of_bounds,
                .err => null,
            };
        }

        /// Append an event to the journal.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// - `value: T`: immutable item staged at the end of the chunk.
        /// Returns (`?Append`): append status; `null` means the journal read failed.
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

        /// Replace the current value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// - `index: usize`: Position in the collection or definition table.
        /// - `value: T`: immutable replacement staged at the selected index.
        /// Returns (`?bool`): replacement status; `null` means the journal read failed.
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

        /// Remove the item at the selected transactional index.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// - `index: usize`: Position in the collection or definition table.
        /// Returns (`?At`): removed value or out-of-range result; `null` means the journal read failed.
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
