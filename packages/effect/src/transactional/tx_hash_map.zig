//! Bounded open-addressed transactional map. Each bucket is a separate
//! reference in the shared journal, so unrelated keys need not copy a table.
const tx = @import("transaction.zig");

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
        pub const Lookup = union(enum) { found: Value, missing };
        pub const Put = enum { inserted, replaced, full };
        pub const Remove = union(enum) { removed: Value, missing };

        buckets: [capacity]tx.Ref(Slot),

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
