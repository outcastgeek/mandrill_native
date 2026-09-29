//! Transactional set sharing the hash map's bucket and commit semantics.
const map = @import("tx_hash_map.zig");
const tx = @import("transaction.zig");

pub fn HashSet(
    comptime Key: type,
    comptime capacity: usize,
    comptime hash: fn (Key) u64,
    comptime equal: fn (Key, Key) bool,
) type {
    return struct {
        const Self = @This();
        const Backing = map.HashMap(Key, void, capacity, hash, equal);
        pub const Added = Backing.Put;
        pub const Removed = enum { removed, missing };
        backing: Backing,

        pub fn init(domain: *tx.Domain) Self {
            return .{ .backing = Backing.init(domain) };
        }

        pub fn contains(self: *Self, journal: *tx.Domain.Journal, key: Key) ?bool {
            const found = self.backing.get(journal, key) orelse return null;
            return found == .found;
        }

        pub fn add(self: *Self, journal: *tx.Domain.Journal, key: Key) ?Added {
            return self.backing.put(journal, key, {});
        }

        pub fn remove(self: *Self, journal: *tx.Domain.Journal, key: Key) ?Removed {
            const removed = self.backing.remove(journal, key) orelse return null;
            return if (removed == .removed) .removed else .missing;
        }
    };
}
