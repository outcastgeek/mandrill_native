//! Data-only replica protocol. A batch carries journal cursors and original
//! event identities; notifications are never treated as history authority.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const keys = @import("../primary_key.zig");
const journal = @import("event_journal.zig");
const Bytes = outcomes.OwnedSlice(u8);

pub const Identity = struct {
    source: []const u8,
    destination: []const u8,
    session: []const u8,

    pub fn valid(self: Identity) bool {
        return keys.inspectEncoded(self.source) == .ok and
            keys.inspectEncoded(self.destination) == .ok and
            self.session.len > 0;
    }
};

pub const Bounds = struct {
    max_entries: usize,
    max_encoded_bytes: usize,

    pub fn valid(self: Bounds) bool {
        return self.max_entries > 0 and self.max_encoded_bytes > 0;
    }
};

pub const Batch = struct {
    pub const effect_owned_payload = true;
    source: Bytes,
    destination: Bytes,
    session: Bytes,
    after: journal.Cursor,
    page: journal.ReadPage,

    pub fn deinit(self: *Batch) void {
        self.source.deinit();
        self.destination.deinit();
        self.session.deinit();
        self.page.deinit();
        self.* = undefined;
    }

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

pub const Acknowledgement = struct {
    source: []const u8,
    destination: []const u8,
    session: []const u8,
    through: journal.Cursor,
};

pub const Gap = struct {
    expected_after: journal.Cursor,
    received_after: journal.Cursor,
};

pub const ImportResult = union(enum) {
    applied: journal.Cursor,
    duplicate: journal.Cursor,
    gap: Gap,
    conflicting_history: journal.Cursor,
    uncertain: journal.Cursor,
};
