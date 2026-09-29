//! Transport-neutral event history contracts. Notifications can wake readers;
//! this journal and its cursors remain the authority for committed progress.
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const EventError = @import("event_log_error.zig").Error;

pub const Bytes = outcomes.OwnedSlice(u8);
/// Sequence zero is the position before the first event; committed events
/// have consecutive positive sequences. `readAfter` excludes this cursor.
pub const Cursor = struct {
    sequence: u64,
    pub const before_first: Cursor = .{ .sequence = 0 };
};

pub const Entry = struct {
    pub const effect_owned_payload = true;
    tag: Bytes,
    event_id: Bytes,
    encoded: Bytes,
    cursor: Cursor,
    schema_version: u32,

    pub fn deinit(self: *@This()) void {
        self.tag.deinit();
        self.event_id.deinit();
        self.encoded.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        var tag = try self.tag.clone();
        errdefer tag.deinit();
        var event_id = try self.event_id.clone();
        errdefer event_id.deinit();
        return .{ .tag = tag, .event_id = event_id, .encoded = try self.encoded.clone(), .cursor = self.cursor, .schema_version = self.schema_version };
    }
};

/// This is data accepted by a journal provider. Both slices remain borrowed
/// until the returned Effect settles; a provider copies what it retains.
pub const Proposal = struct {
    tag: []const u8,
    event_id: []const u8,
    encoded: []const u8,
    schema_version: u32,
};

pub const AppendAcknowledgement = union(enum) {
    committed: Cursor,
    accepted,
    rejected,
    uncertain,
};

pub const ReadPage = struct {
    pub const effect_owned_payload = true;
    entries: outcomes.OwnedSlice(Entry),
    next: Cursor,
    more: bool,

    pub fn deinit(self: *@This()) void {
        self.entries.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{ .entries = try self.entries.clone(), .next = self.next, .more = self.more };
    }
};

pub const ReadPolicy = struct {
    /// The caller selects a strictly positive bound before admission.
    max_entries: usize,
    max_encoded_bytes: usize,

    pub fn valid(self: ReadPolicy) bool {
        return self.max_entries > 0 and self.max_encoded_bytes > 0;
    }
};

/// A journal implementation may be volatile or retained. This interface
/// promises ordered append/read and explicit uncertainty, not physical
/// durability. A stronger projection/checkpoint atomic capability is separate.
pub fn Journal(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        pub const Failure = EventError(ProviderError);
        pub const AppendEffect = effects.Effect(AppendAcknowledgement, Failure, R);
        pub const ReadEffect = effects.Effect(ReadPage, Failure, R);
        pub const LookupEffect = effects.Effect(?Entry, Failure, R);
        userdata: ?*anyopaque,
        vtable: *const VTable,

        pub const VTable = struct {
            append: *const fn (?*anyopaque, *const R, Proposal) AppendEffect,
            read_after: *const fn (?*anyopaque, *const R, Cursor, ReadPolicy) ReadEffect,
            find_event: *const fn (?*anyopaque, *const R, []const u8) LookupEffect,
        };

        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }

        pub fn append(comptime select_journal: fn (*const R) *const Self, comptime select_proposal: fn (*const R) Proposal) AppendEffect {
            return AppendEffect.deferEffect(struct {
                fn start(env: *const R) AppendEffect {
                    const proposal = select_proposal(env);
                    if (proposal.event_id.len == 0 or proposal.tag.len == 0)
                        return AppendEffect.fromOwnedOutcome(env, .{ .fail = .invalid_identity });
                    const journal = select_journal(env);
                    return journal.vtable.append(journal.userdata, env, proposal);
                }
            }.start);
        }

        pub fn readAfter(
            comptime select_journal: fn (*const R) *const Self,
            comptime select_cursor: fn (*const R) Cursor,
            comptime select_policy: fn (*const R) ReadPolicy,
        ) ReadEffect {
            return ReadEffect.deferEffect(struct {
                fn start(env: *const R) ReadEffect {
                    const policy = select_policy(env);
                    if (!policy.valid()) return ReadEffect.fromOwnedOutcome(env, .{ .fail = .invalid_policy });
                    const journal = select_journal(env);
                    return journal.vtable.read_after(journal.userdata, env, select_cursor(env), policy);
                }
            }.start);
        }

        pub fn findEvent(comptime select_journal: fn (*const R) *const Self, comptime select_id: fn (*const R) []const u8) LookupEffect {
            return LookupEffect.deferEffect(struct {
                fn start(env: *const R) LookupEffect {
                    const id = select_id(env);
                    if (id.len == 0) return LookupEffect.fromOwnedOutcome(env, .{ .fail = .invalid_identity });
                    const journal = select_journal(env);
                    return journal.vtable.find_event(journal.userdata, env, id);
                }
            }.start);
        }
    };
}
