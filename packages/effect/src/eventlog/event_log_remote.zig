//! Replaceable replication, cursor, session and protection boundaries. None
//! constructs a transport, clock, crypto backend or persistence engine.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const journal = @import("event_journal.zig");
const message = @import("event_log_message.zig");

pub fn Error(comptime ProviderError: type) type {
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = outcomes.ownsPayload(ProviderError);
        provider: ProviderError,
        malformed,
        disconnected,
        rejected_session,
        rejected_protection,
        uncertain_delivery,
        unavailable,

        pub fn deinit(self: *Self) void {
            if (self.* == .provider) outcomes.discardPayload(&self.provider);
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .provider => |value| .{ .provider = try outcomes.clonePayload(value) },
                else => self.*,
            };
        }
    };
}

pub const Delivery = enum { accepted, possibly_accepted, rejected };
pub const Received = union(enum) {
    pub const effect_owned_payload = true;
    batch: message.Batch,
    disconnected,

    pub fn deinit(self: *@This()) void {
        if (self.* == .batch) self.batch.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .batch => |value| .{ .batch = try value.clone() },
            .disconnected => .disconnected,
        };
    }
};

/// The provider retains no borrowed batch fields after send completes.
pub fn Transport(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        pub const Failure = Error(ProviderError);
        pub const SendEffect = effects.Effect(Delivery, Failure, R);
        pub const ReceiveEffect = effects.Effect(Received, Failure, R);
        pub const AckEffect = effects.Effect(Delivery, Failure, R);
        userdata: ?*anyopaque,
        vtable: *const VTable,

        pub const VTable = struct {
            send_batch: *const fn (?*anyopaque, *const R, *const message.Batch) SendEffect,
            receive_batch: *const fn (?*anyopaque, *const R) ReceiveEffect,
            acknowledge: *const fn (?*anyopaque, *const R, message.Acknowledgement) AckEffect,
        };

        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }
    };
}

pub const Advance = union(enum) {
    committed: journal.Cursor,
    already_advanced: journal.Cursor,
    conflict: journal.Cursor,
    possibly_committed,
};

/// Progress is keyed by source replica. Implementations must compare and
/// advance atomically. Import may append before advancing, but stable event
/// IDs make a retry deduplicate the append after an uncertain cursor update.
pub fn CursorStore(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        pub const Failure = Error(ProviderError);
        pub const ReadEffect = effects.Effect(journal.Cursor, Failure, R);
        pub const AdvanceEffect = effects.Effect(Advance, Failure, R);
        userdata: ?*anyopaque,
        vtable: *const VTable,

        pub const VTable = struct {
            read: *const fn (?*anyopaque, *const R, []const u8) ReadEffect,
            compare_and_advance: *const fn (?*anyopaque, *const R, []const u8, journal.Cursor, journal.Cursor) AdvanceEffect,
        };

        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }
    };
}

pub fn SessionAuthority(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        pub const ValidateEffect = effects.Effect(bool, Error(ProviderError), R);
        userdata: ?*anyopaque,
        validate: *const fn (?*anyopaque, *const R, message.Identity) ValidateEffect,

        pub fn fromProvider(data: ?*anyopaque, callback: *const fn (?*anyopaque, *const R, message.Identity) ValidateEffect) Self {
            return .{ .userdata = data, .validate = callback };
        }
    };
}

pub fn PayloadProtection(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        pub const TransformEffect = effects.Effect(outcomes.OwnedSlice(u8), Error(ProviderError), R);
        userdata: ?*anyopaque,
        seal: *const fn (?*anyopaque, *const R, message.Identity, []const u8) TransformEffect,
        open: *const fn (?*anyopaque, *const R, message.Identity, []const u8) TransformEffect,

        pub fn fromProvider(
            data: ?*anyopaque,
            seal: *const fn (?*anyopaque, *const R, message.Identity, []const u8) TransformEffect,
            open: *const fn (?*anyopaque, *const R, message.Identity, []const u8) TransformEffect,
        ) Self {
            return .{ .userdata = data, .seal = seal, .open = open };
        }
    };
}

/// Compaction requires a provider-defined replay proof. The core never
/// assumes a projection checkpoint can replace history for every consumer.
pub fn Compaction(comptime R: type, comptime ProviderError: type, comptime Proof: type) type {
    return struct {
        const Self = @This();
        pub const CompactEffect = effects.Effect(journal.Cursor, Error(ProviderError), R);
        userdata: ?*anyopaque,
        compact_through: *const fn (?*anyopaque, *const R, journal.Cursor, *const Proof) CompactEffect,
        oldest_available: *const fn (?*anyopaque, *const R) CompactEffect,

        pub fn fromProvider(
            data: ?*anyopaque,
            compact: *const fn (?*anyopaque, *const R, journal.Cursor, *const Proof) CompactEffect,
            oldest: *const fn (?*anyopaque, *const R) CompactEffect,
        ) Self {
            return .{ .userdata = data, .compact_through = compact, .oldest_available = oldest };
        }
    };
}

/// Conflict facts are borrowed for the duration of a handler Effect. A
/// handler may consult application history or request a snapshot, but the
/// importer never fabricates a universal event merge from timestamps.
pub const Conflict = struct {
    source: []const u8,
    destination: []const u8,
    source_cursor: journal.Cursor,
    observed_cursor: journal.Cursor,
    event_id: ?[]const u8,
    reason: enum { cursor_diverged, missing_event, incompatible_event, concurrent_append },

    pub fn valid(self: Conflict) bool {
        return self.source.len > 0 and self.destination.len > 0 and
            std.unicode.utf8ValidateSlice(self.source) and
            std.unicode.utf8ValidateSlice(self.destination);
    }
};

/// Resolution is an application-defined typed value. The core deliberately
/// does not interpret it as permission to rewrite journal history; any
/// replacement must pass the same authoritative journal contracts as append.
pub fn ConflictHandler(comptime R: type, comptime Resolution: type, comptime HandlerError: type) type {
    return struct {
        const Self = @This();
        pub const ProviderEffect = effects.Effect(Resolution, HandlerError, R);
        pub const ResolveEffect = effects.Effect(Resolution, Error(HandlerError), R);
        userdata: ?*anyopaque,
        resolve_fn: *const fn (?*anyopaque, *const R, Conflict) ProviderEffect,

        pub fn fromProvider(data: ?*anyopaque, callback: *const fn (?*anyopaque, *const R, Conflict) ProviderEffect) Self {
            return .{ .userdata = data, .resolve_fn = callback };
        }

        pub fn resolve(self: Self, env: *const R, conflict: Conflict) ResolveEffect {
            if (!conflict.valid()) return ResolveEffect.fail(.malformed);
            var delegated = self.resolve_fn(self.userdata, env, conflict);
            return delegated.mapErrorOwned(env, struct {
                fn provider(problem: HandlerError) Error(HandlerError) {
                    return .{ .provider = problem };
                }
            }.provider);
        }
    };
}
