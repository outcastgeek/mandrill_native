//! Data-only replica protocol. A batch carries journal cursors and original
//! event identities; notifications are never treated as history authority.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const keys = @import("../primary_key.zig");
const journal = @import("event_journal.zig");
const Bytes = outcomes.OwnedSlice(u8);

/// Stable event and replica identifiers used across replication.
pub const Identity = struct {
    source: []const u8,
    destination: []const u8,
    session: []const u8,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Identity`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` for valid encoded source/destination and nonempty session.
    pub fn valid(self: Identity) bool {
        return keys.inspectEncoded(self.source) == .ok and
            keys.inspectEncoded(self.destination) == .ok and
            self.session.len > 0;
    }
};

/// Caller-selected limits for encoded event and batch size.
pub const Bounds = struct {
    max_entries: usize,
    max_encoded_bytes: usize,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Bounds`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` when entry and encoded-byte bounds are positive.
    pub fn valid(self: Bounds) bool {
        return self.max_entries > 0 and self.max_encoded_bytes > 0;
    }
};

/// Ordered set of encoded events sent to a replica.
pub const Batch = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    source: Bytes,
    destination: Bytes,
    session: Bytes,
    after: journal.Cursor,
    page: journal.ReadPage,

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *Batch`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *Batch) void {
        self.source.deinit();
        self.destination.deinit();
        self.session.deinit();
        self.page.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const Batch`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!Batch`): A `Batch` on success; `OutOfMemory` if required allocation fails.
    pub fn clone(self: *const Batch) error{OutOfMemory}!Batch {
        var source = try self.source.clone();
        errdefer source.deinit();
        var destination = try self.destination.clone();
        errdefer destination.deinit();
        var session = try self.session.clone();
        errdefer session.deinit();
        return .{
            .source = source,
            .destination = destination,
            .session = session,
            .after = self.after,
            .page = try self.page.clone(),
        };
    }

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: *const Batch`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// - `bounds: Bounds`: Caller-supplied size or traversal bounds.
    /// Returns (`bool`): `true` when identity, bounds, page size, and ordered cursors all validate.
    pub fn valid(self: *const Batch, bounds: Bounds) bool {
        if (!bounds.valid() or !(Identity{ .source = self.source.items, .destination = self.destination.items, .session = self.session.items }).valid() or
            self.page.entries.items.len > bounds.max_entries) return false;
        var cursor = self.after.sequence;
        var total: usize = 0;
        for (self.page.entries.items) |entry| {
            cursor = std.math.add(u64, cursor, 1) catch return false;
            if (entry.cursor.sequence != cursor or entry.tag.items.len == 0 or
                keys.inspectEncoded(entry.event_id.items) == .err) return false;
            total = std.math.add(usize, total, entry.encoded.items.len) catch return false;
            if (total > bounds.max_encoded_bytes) return false;
        }
        return self.page.next.sequence == cursor;
    }
};

/// Replica acknowledgement with cursor and accepted progress.
pub const Acknowledgement = struct {
    source: []const u8,
    destination: []const u8,
    session: []const u8,
    through: journal.Cursor,
};

/// Missing sequence range that prevents contiguous replay.
pub const Gap = struct {
    expected_after: journal.Cursor,
    received_after: journal.Cursor,
};

/// Result of importing a replica batch, including progress or conflict.
pub const ImportResult = union(enum) {
    applied: journal.Cursor,
    duplicate: journal.Cursor,
    gap: Gap,
    conflicting_history: journal.Cursor,
    uncertain: journal.Cursor,
};
