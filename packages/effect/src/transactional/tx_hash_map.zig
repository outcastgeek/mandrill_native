//! Bounded open-addressed transactional map. Each bucket is a separate
//! reference in the shared journal, so unrelated keys need not copy a table.
const tx = @import("transaction.zig");

/// Specialize a transactional map by key and value types.
/// Parameters:
/// - `Key: comptime type`: Key type used to identify or look up entries in this component.
/// - `Value: comptime type`: Value type represented or stored by this component.
/// - `capacity: comptime usize`: Compile-time caller-selected bound or count; zero/overflow handling follows this
///   API’s validation.
/// - `hash: comptime fn (Key) u64`: Compile-time typed function selected at comptime; no runtime callback lookup is
///   required.
/// - `equal: comptime fn (Key, Key) bool`: Compile-time typed function selected at comptime; no runtime callback lookup is
///   required.
/// Returns (`type`): transactional map whose key/value updates commit with one domain journal.
pub fn HashMap(
    comptime Key: type,
    comptime Value: type,
    comptime capacity: usize,
    comptime hash: fn (Key) u64,
    comptime equal: fn (Key, Key) bool,
) type {
    if (capacity == 0) @compileError("transactional map capacity must be positive");
    if (!tx.immutableValue(Key) or !tx.immutableValue(Value))
        @compileError("transactional map keys and values must be immutable and pointer-free");
    return struct {
        const Self = @This();
        const Pair = struct { key: Key, value: Value };
        const Slot = union(enum) { empty, occupied: Pair, tombstone };
        /// Transactional map lookup: found value or missing key.
        pub const Lookup = union(enum) { found: Value, missing };
        /// `Put` cases: `inserted`, `replaced`, `full`.
        pub const Put = enum { inserted, replaced, full };
        /// Removal result distinguishing a removed value from an absent key.
        pub const Remove = union(enum) { removed: Value, missing };

        buckets: [capacity]tx.Ref(Slot),

        /// Create an empty transactional hash map in the supplied domain.
        /// Parameters:
        /// - `domain: *tx.Domain`: Transaction domain containing the cells and journal for this operation.
        /// Returns (`Self`): a map whose mutations participate in that domain’s journal.
        pub fn init(domain: *tx.Domain) Self {
            var result: Self = undefined;
            for (&result.buckets) |*bucket| bucket.* = tx.Ref(Slot).init(domain, .empty);
            return result;
        }

        fn start(key: Key) usize {
            const divisor: u64 = @intCast(capacity);
            return @intCast(hash(key) % divisor);
        }

        fn next(index: usize) usize {
            return if (index == capacity - 1) 0 else index + 1;
        }

        /// Look up the selected value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// - `key: Key`: Stable identity used for matching, deduplication, or replay.
        /// Returns (`?Lookup`): `.found` or `.missing`; `null` means the journal read failed.
        pub fn get(self: *Self, journal: *tx.Domain.Journal, key: Key) ?Lookup {
            var index = start(key);
            for (0..capacity) |_| {
                const slot = switch (journal.read(Slot, &self.buckets[index])) {
                    .ok => |value| value,
                    .err => return null,
                };
                switch (slot) {
                    .empty => return .missing,
                    .occupied => |entry| if (equal(entry.key, key)) return .{ .found = entry.value },
                    .tombstone => {},
                }
                index = next(index);
            }
            return .missing;
        }

        /// Store the supplied value under its typed key.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// - `key: Key`: Stable identity used for matching, deduplication, or replay.
        /// - `value: Value`: immutable map value staged for the key when journal reads succeed.
        /// Returns (`?Put`): insert/replace status; `null` means the journal read failed before staging a write.
        pub fn put(self: *Self, journal: *tx.Domain.Journal, key: Key, value: Value) ?Put {
            var index = start(key);
            var reusable: ?usize = null;
            for (0..capacity) |_| {
                const slot = switch (journal.read(Slot, &self.buckets[index])) {
                    .ok => |current| current,
                    .err => return null,
                };
                switch (slot) {
                    .empty => {
                        journal.write(Slot, &self.buckets[reusable orelse index], .{ .occupied = .{ .key = key, .value = value } });
                        return .inserted;
                    },
                    .occupied => |entry| if (equal(entry.key, key)) {
                        journal.write(Slot, &self.buckets[index], .{ .occupied = .{ .key = key, .value = value } });
                        return .replaced;
                    },
                    .tombstone => if (reusable == null) {
                        reusable = index;
                    },
                }
                index = next(index);
            }
            if (reusable) |vacant| {
                journal.write(Slot, &self.buckets[vacant], .{ .occupied = .{ .key = key, .value = value } });
                return .inserted;
            }
            return .full;
        }

        /// Remove the selected entry.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// - `key: Key`: Stable identity used for matching, deduplication, or replay.
        /// Returns (`?Remove`): `.removed` or `.missing`; `null` means the journal read failed.
        pub fn remove(self: *Self, journal: *tx.Domain.Journal, key: Key) ?Remove {
            var index = start(key);
            for (0..capacity) |_| {
                const slot = switch (journal.read(Slot, &self.buckets[index])) {
                    .ok => |value| value,
                    .err => return null,
                };
                switch (slot) {
                    .empty => return .missing,
                    .occupied => |entry| if (equal(entry.key, key)) {
                        journal.write(Slot, &self.buckets[index], .tombstone);
                        return .{ .removed = entry.value };
                    },
                    .tombstone => {},
                }
                index = next(index);
            }
            return .missing;
        }
    };
}
