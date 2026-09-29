//! Transactional set sharing the hash map's bucket and commit semantics.
const map = @import("tx_hash_map.zig");
const tx = @import("transaction.zig");

/// Specialize a transactional set by element type.
/// Parameters:
/// - `Key: comptime type`: Key type used to identify or look up entries in this component.
/// - `capacity: comptime usize`: Compile-time caller-selected bound or count; zero/overflow handling follows this
///   API’s validation.
/// - `hash: comptime fn (Key) u64`: Compile-time typed function selected at comptime; no runtime callback lookup is
///   required.
/// - `equal: comptime fn (Key, Key) bool`: Compile-time typed function selected at comptime; no runtime callback lookup is
///   required.
/// Returns (`type`): journal-backed set of immutable keys.
pub fn HashSet(
    comptime Key: type,
    comptime capacity: usize,
    comptime hash: fn (Key) u64,
    comptime equal: fn (Key, Key) bool,
) type {
    return struct {
        const Self = @This();
        const Backing = map.HashMap(Key, void, capacity, hash, equal);
        /// Result of inserting a value into the transactional set.
        pub const Added = Backing.Put;
        /// `Removed` cases: `removed`, `missing`.
        pub const Removed = enum { removed, missing };
        backing: Backing,

        /// Create an empty transactional hash set in the supplied domain.
        /// Parameters:
        /// - `domain: *tx.Domain`: Transaction domain containing the cells and journal for this operation.
        /// Returns (`Self`): a set whose changes commit with the domain journal.
        pub fn init(domain: *tx.Domain) Self {
            return .{ .backing = Backing.init(domain) };
        }

        /// Check whether the transactional set contains a value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// - `key: Key`: Stable identity used for matching, deduplication, or replay.
        /// Returns (`?bool`): membership result; `null` means the journal read failed.
        pub fn contains(self: *Self, journal: *tx.Domain.Journal, key: Key) ?bool {
            const found = self.backing.get(journal, key) orelse return null;
            return found == .found;
        }

        /// Add a value to the transactional set.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// - `key: Key`: Stable identity used for matching, deduplication, or replay.
        /// Returns (`?Added`): added/already-present status; `null` means the journal read failed.
        pub fn add(self: *Self, journal: *tx.Domain.Journal, key: Key) ?Added {
            return self.backing.put(journal, key, {});
        }

        /// Remove the selected entry.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `journal: *tx.Domain.Journal`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// - `key: Key`: Stable identity used for matching, deduplication, or replay.
        /// Returns (`?Removed`): removed/missing status; `null` means the journal read failed.
        pub fn remove(self: *Self, journal: *tx.Domain.Journal, key: Key) ?Removed {
            const removed = self.backing.remove(journal, key) orelse return null;
            return if (removed == .removed) .removed else .missing;
        }
    };
}
