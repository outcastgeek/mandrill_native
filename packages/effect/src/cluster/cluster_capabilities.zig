//! Logical ownership capability, independent of membership, placement and
//! transport. MessageStore remains the separate authority for requests and
//! replies; an expired or replaced lease cannot complete its old claims.
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const deadlines = @import("../deadline.zig");
const messages = @import("message_store.zig");
const types = @import("cluster_types.zig");

pub const EntityAddress = types.EntityAddress;

pub const LeasePolicy = struct {
    until: deadlines.Deadline,

    pub fn valid(self: LeasePolicy) bool {
        return self.until != .never;
    }
};

pub const Token = struct {
    address: EntityAddress,
    owner_id: []const u8,
    generation: u64,
};

/// A provider can hold its ownership linearization gate across a message
/// claim/completion transition. The message authority must release this guard
/// on every exit path. A cross-process provider supplies an equivalent atomic
/// proof; a read followed by an unguarded write is insufficient.
pub const Fence = struct {
    userdata: ?*anyopaque,
    enter: *const fn (?*anyopaque, Token, deadlines.WallInstant) Enter,

    pub const Guard = struct {
        userdata: ?*anyopaque,
        until: deadlines.Deadline,
        release_fn: *const fn (?*anyopaque) void,

        pub fn release(self: Guard) void {
            self.release_fn(self.userdata);
        }
    };

    pub const Enter = union(enum) { held: Guard, stale, unavailable };
};

pub const Lease = struct {
    pub const effect_owned_payload = true;
    group: messages.Bytes,
    entity_key: messages.Bytes,
    owner_id: messages.Bytes,
    generation: u64,
    until: deadlines.Deadline,

    pub fn token(self: *const @This()) Token {
        return .{ .address = .{ .group = self.group.items, .entity_key = self.entity_key.items }, .owner_id = self.owner_id.items, .generation = self.generation };
    }

    pub fn deinit(self: *@This()) void {
        self.group.deinit();
        self.entity_key.deinit();
        self.owner_id.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        var group = try self.group.clone();
        errdefer group.deinit();
        var entity_key = try self.entity_key.clone();
        errdefer entity_key.deinit();
        return .{ .group = group, .entity_key = entity_key, .owner_id = try self.owner_id.clone(), .generation = self.generation, .until = self.until };
    }
};

pub const Acquisition = union(enum) {
    pub const effect_owned_payload = true;
    acquired: Lease,
    unavailable,

    pub fn deinit(self: *@This()) void {
        if (self.* == .acquired) self.acquired.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .acquired => |value| .{ .acquired = try value.clone() },
            .unavailable => .unavailable,
        };
    }
};

pub const Renewal = enum { renewed, stale, possibly_renewed };
pub const Release = enum { released, already_released, stale, possibly_released };
pub const Error = enum { invalid_identity, invalid_deadline, unavailable, generation_exhausted };
pub const Guarantee = enum { @"volatile", recoverable };

/// A conforming implementation linearizes acquire, renew and release for
/// each entity; generations never repeat. It must not execute user handlers
/// while holding its ownership gate. An ambiguous acknowledgement is surfaced
/// for read-back rather than assumed successful.
pub fn Ownership(comptime R: type, comptime ProviderError: type, comptime guarantee: Guarantee) type {
    return struct {
        const Self = @This();
        pub const storage_guarantee = guarantee;
        pub const Failure = union(enum) {
            const Own = @This();
            pub const effect_owned_payload = outcomes.ownsPayload(ProviderError);
            operation: Error,
            provider: ProviderError,

            pub fn deinit(self: *Own) void {
                if (self.* == .provider) outcomes.discardPayload(&self.provider);
                self.* = undefined;
            }

            pub fn clone(self: *const Own) error{OutOfMemory}!Own {
                return switch (self.*) {
                    .provider => |value| .{ .provider = try outcomes.clonePayload(value) },
                    else => self.*,
                };
            }
        };
        pub const AcquireEffect = effects.Effect(Acquisition, Failure, R);
        pub const RenewEffect = effects.Effect(Renewal, Failure, R);
        pub const ReleaseEffect = effects.Effect(Release, Failure, R);
        pub const ReadEffect = effects.Effect(?Lease, Failure, R);

        userdata: ?*anyopaque,
        vtable: *const VTable,

        pub const VTable = struct {
            acquire: *const fn (?*anyopaque, *const R, EntityAddress, []const u8, LeasePolicy) AcquireEffect,
            renew: *const fn (?*anyopaque, *const R, Token, LeasePolicy) RenewEffect,
            release: *const fn (?*anyopaque, *const R, Token) ReleaseEffect,
            read: *const fn (?*anyopaque, *const R, EntityAddress) ReadEffect,
        };

        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }
    };
}
