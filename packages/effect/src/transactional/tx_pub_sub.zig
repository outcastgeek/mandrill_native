//! Bounded transactional broadcast history. Subscribers hold cursors, not
//! pointers into mutable storage; overload is explicit as reject or gap.
const std = @import("std");
const tx = @import("transaction.zig");

/// Specialize bounded transactional publication by item type and capacity.
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// - `capacity: comptime usize`: Compile-time caller-selected bound or count; zero/overflow handling follows this
///   API’s validation.
/// Returns (`type`): bounded transactional publication bus whose state commits atomically.
pub fn PubSub(comptime T: type, comptime capacity: usize) type {
    if (capacity == 0) @compileError("transactional PubSub capacity must be positive");
    if (!tx.immutableValue(T)) @compileError("transactional PubSub values must be immutable");
    return struct {
        const Self = @This();
        const retention_cap: u64 = @intCast(capacity);
        /// `OverflowPolicy` cases: `reject`, `drop_oldest`.
        pub const OverflowPolicy = enum { reject, drop_oldest };
        /// `Start` cases: `oldest_retained`, `next_committed`.
        pub const Start = enum { oldest_retained, next_committed };
        /// `Published` cases: `committed`, `full`, `sequence_exhausted`.
        pub const Published = enum { committed, full, sequence_exhausted };
        /// Subscriber delivery with sequence and item, or a gap/closed state.
        pub const Received = union(enum) {
            value: struct { sequence: u64, item: T },
            gap: struct { requested: u64, oldest_retained: u64 },
            pending,
        };
        const Entry = struct { sequence: u64, value: T };
        const State = struct {
            entries: [capacity]?Entry = [_]?Entry{null} ** capacity,
            oldest: u64 = 0,
            next: u64 = 0,
        };

        reference: tx.Ref(State),
        overflow: OverflowPolicy,

        /// Create a transactional publication bus with the selected overflow policy.
        /// Parameters:
        /// - `domain: *tx.Domain`: Transaction domain containing the cells and journal for this operation.
        /// - `overflow: OverflowPolicy`: Behavior when the bounded buffer is full.
        /// Returns (`Self`): a bus whose publication state commits with the domain journal.
        pub fn init(domain: *tx.Domain, overflow: OverflowPolicy) Self {
            return .{ .reference = tx.Ref(State).init(domain, .{}), .overflow = overflow };
        }

        /// Open a subscription to future publications.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// - `start: Start`: Async start outcome, either completed or pending.
        /// Returns (`?u64`): initial subscriber cursor; `null` means the journal read failed.
        pub fn subscribe(self: *Self, journal: *tx.Domain.Journal, start: Start) ?u64 {
            const state = switch (journal.read(State, &self.reference)) {
                .ok => |value| value,
                .err => return null,
            };
            return if (start == .oldest_retained) state.oldest else state.next;
        }

        /// Publish a value to active subscribers.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// - `value: T`: immutable item staged for publication at the next sequence.
        /// Returns (`?Published`): commit/full/sequence status; `null` means the journal read failed.
        pub fn publish(self: *Self, journal: *tx.Domain.Journal, value: T) ?Published {
            var state = switch (journal.read(State, &self.reference)) {
                .ok => |current| current,
                .err => return null,
            };
            if (state.next == std.math.maxInt(u64)) return .sequence_exhausted;
            const retained = state.next - state.oldest;
            if (retained == retention_cap) {
                if (self.overflow == .reject) return .full;
                state.oldest += 1;
            }
            const index: usize = @intCast(state.next % retention_cap);
            state.entries[index] = .{ .sequence = state.next, .value = value };
            state.next += 1;
            journal.write(State, &self.reference, state);
            return .committed;
        }

        /// Observe the current completion state.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// - `cursor: u64`: Caller-supplied time or cursor boundary used for ordering and retention.
        /// Returns (`?Received`): item, gap, or pending status; `null` means the journal read failed.
        pub fn poll(self: *Self, journal: *tx.Domain.Journal, cursor: u64) ?Received {
            const state = switch (journal.read(State, &self.reference)) {
                .ok => |current| current,
                .err => return null,
            };
            if (cursor < state.oldest) return .{ .gap = .{
                .requested = cursor,
                .oldest_retained = state.oldest,
            } };
            if (cursor >= state.next) return .pending;
            const index: usize = @intCast(cursor % retention_cap);
            const entry = state.entries[index].?;
            return .{ .value = .{ .sequence = entry.sequence, .item = entry.value } };
        }
    };
}
