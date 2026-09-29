//! Caller-sized transactional binary heap. Comparator and capacity are
//! compile-time choices; no hidden allocation or scheduler is used.
const std = @import("std");
const tx = @import("transaction.zig");

/// Specialize a bounded transactional priority queue by item type, capacity, and comparator.
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// - `capacity: comptime usize`: Compile-time caller-selected bound or count; zero/overflow handling follows this
///   API’s validation.
/// - `compare: comptime fn (T, T) std.math.Order`: Compile-time typed function selected at comptime; no runtime callback lookup is
///   required.
/// Returns (`type`): bounded journal-backed priority queue using the supplied comparator.
pub fn PriorityQueue(comptime T: type, comptime capacity: usize, comptime compare: fn (T, T) std.math.Order) type {
    if (capacity == 0) @compileError("transactional priority queue capacity must be positive");
    if (!tx.immutableValue(T)) @compileError("priority queue values must be immutable");
    return struct {
        const Self = @This();
        const children_per_node: usize = 2;
        const State = struct { items: [capacity]?T = [_]?T{null} ** capacity, length: usize = 0 };
        /// `Offered` cases: `accepted`, `full`.
        pub const Offered = enum { accepted, full };
        /// Priority-queue removal result: selected value or empty queue.
        pub const Taken = union(enum) { value: T, empty };
        reference: tx.Ref(State),

        /// Create an empty transactional priority queue in the supplied domain.
        /// Parameters:
        /// - `domain: *tx.Domain`: Transaction domain containing the cells and journal for this operation.
        /// Returns (`Self`): a queue whose ordering and mutations participate in that domain.
        pub fn init(domain: *tx.Domain) Self {
            return .{ .reference = tx.Ref(State).init(domain, .{}) };
        }

        /// Read the number of elements.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// Returns (`?usize`): staged queue length; `null` means the journal read failed.
        pub fn len(self: *Self, journal: *tx.Domain.Journal) ?usize {
            return switch (journal.read(State, &self.reference)) {
                .ok => |state| state.length,
                .err => null,
            };
        }

        /// Offer the value to the selected queue or store.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// - `value: T`: immutable item staged in comparator order when capacity permits.
        /// Returns (`?Offered`): accepted/full status; `null` means the journal read failed.
        pub fn offer(self: *Self, journal: *tx.Domain.Journal, value: T) ?Offered {
            var state = switch (journal.read(State, &self.reference)) {
                .ok => |current| current,
                .err => return null,
            };
            if (state.length == capacity) return .full;
            var index = state.length;
            state.items[index] = value;
            state.length += 1;
            while (index > 0) {
                const parent = (index - 1) / children_per_node;
                if (compare(state.items[index].?, state.items[parent].?) != .lt) break;
                std.mem.swap(?T, &state.items[index], &state.items[parent]);
                index = parent;
            }
            journal.write(State, &self.reference, state);
            return .accepted;
        }

        /// Observe the current completion state.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// Returns (`?Taken`): highest-priority value or empty status; `null` means the journal read failed.
        pub fn poll(self: *Self, journal: *tx.Domain.Journal) ?Taken {
            var state = switch (journal.read(State, &self.reference)) {
                .ok => |current| current,
                .err => return null,
            };
            if (state.length == 0) return .empty;
            const first = state.items[0].?;
            state.length -= 1;
            const last = state.items[state.length];
            state.items[state.length] = null;
            if (state.length > 0) {
                state.items[0] = last;
                var index: usize = 0;
                while (true) {
                    const left = std.math.mul(usize, index, children_per_node) catch break;
                    const child = std.math.add(usize, left, 1) catch break;
                    if (child >= state.length) break;
                    const right = std.math.add(usize, child, 1) catch state.length;
                    var selected = child;
                    if (right < state.length and compare(state.items[right].?, state.items[child].?) == .lt)
                        selected = right;
                    if (compare(state.items[selected].?, state.items[index].?) != .lt) break;
                    std.mem.swap(?T, &state.items[selected], &state.items[index]);
                    index = selected;
                }
            }
            journal.write(State, &self.reference, state);
            return .{ .value = first };
        }
    };
}
