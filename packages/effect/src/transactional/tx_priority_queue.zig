//! Caller-sized transactional binary heap. Comparator and capacity are
//! compile-time choices; no hidden allocation or scheduler is used.
const std = @import("std");
const tx = @import("transaction.zig");

pub fn PriorityQueue(comptime T: type, comptime capacity: usize, comptime compare: fn (T, T) std.math.Order) type {
    if (capacity == 0) @compileError("transactional priority queue capacity must be positive");
    if (!tx.immutableValue(T)) @compileError("priority queue values must be immutable");
    return struct {
        const Self = @This();
        const children_per_node: usize = 2;
        const State = struct { items: [capacity]?T = [_]?T{null} ** capacity, length: usize = 0 };
        pub const Offered = enum { accepted, full };
        pub const Taken = union(enum) { value: T, empty };
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
