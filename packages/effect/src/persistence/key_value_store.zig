//! Portable encoded storage contracts. Conditional updates are a stronger
//! capability, never inferred from ordinary get/set behavior.
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const deadlines = @import("../deadline.zig");
const PersistenceError = @import("persistence_error.zig").Error;

pub const Bytes = outcomes.OwnedSlice(u8);
pub const Expiry = deadlines.Deadline;

pub const Read = union(enum) {
    pub const effect_owned_payload = true;
    found: Bytes,
    missing,
    expired,

    pub fn deinit(self: *@This()) void {
        if (self.* == .found) self.found.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .found => |value| .{ .found = try value.clone() },
            else => self.*,
        };
    }
};

pub const Revision = union(enum) { absent, present: u64 };
pub const Versioned = struct {
    pub const effect_owned_payload = true;
    bytes: Bytes,
    revision: Revision,

    pub fn deinit(self: *@This()) void {
        self.bytes.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{ .bytes = try self.bytes.clone(), .revision = self.revision };
    }
};

pub const ReadVersioned = union(enum) {
    pub const effect_owned_payload = true;
    found: Versioned,
    missing,
    expired,

    pub fn deinit(self: *@This()) void {
        if (self.* == .found) self.found.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .found => |value| .{ .found = try value.clone() },
            else => self.*,
        };
    }
};

pub const WriteAcknowledgement = enum { committed, possibly_committed };
pub const ConditionalAcknowledgement = union(enum) {
    applied: Revision,
    conflict: Revision,
    possibly_applied,
};

/// Providers return owned read buffers allocated from R. They copy borrowed
/// write buffers before committing or retaining them. The callback constructs
/// an Effect; the existing kernel then owns any suspension/cancellation.
pub fn KeyValueStore(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        pub const Failure = PersistenceError(ProviderError);
        pub const ReadEffect = effects.Effect(Read, Failure, R);
        pub const WriteEffect = effects.Effect(WriteAcknowledgement, Failure, R);
        userdata: ?*anyopaque,
        vtable: *const VTable,

        pub const VTable = struct {
            read: *const fn (?*anyopaque, *const R, []const u8) ReadEffect,
            write: *const fn (?*anyopaque, *const R, []const u8, []const u8, Expiry) WriteEffect,
            remove: *const fn (?*anyopaque, *const R, []const u8) WriteEffect,
        };

        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }

        pub fn read(comptime select_store: fn (*const R) *const Self, comptime select_key: fn (*const R) []const u8) ReadEffect {
            return ReadEffect.deferEffect(struct {
                fn run(env: *const R) ReadEffect {
                    const store = select_store(env);
                    return store.vtable.read(store.userdata, env, select_key(env));
                }
            }.run);
        }

        pub fn write(
            comptime select_store: fn (*const R) *const Self,
            comptime select_key: fn (*const R) []const u8,
            comptime select_value: fn (*const R) []const u8,
            comptime select_expiry: fn (*const R) Expiry,
        ) WriteEffect {
            return WriteEffect.deferEffect(struct {
                fn run(env: *const R) WriteEffect {
                    const store = select_store(env);
                    return store.vtable.write(store.userdata, env, select_key(env), select_value(env), select_expiry(env));
                }
            }.run);
        }

        pub fn remove(comptime select_store: fn (*const R) *const Self, comptime select_key: fn (*const R) []const u8) WriteEffect {
            return WriteEffect.deferEffect(struct {
                fn run(env: *const R) WriteEffect {
                    const store = select_store(env);
                    return store.vtable.remove(store.userdata, env, select_key(env));
                }
            }.run);
        }
    };
}

/// A successful compareExchange is linearizable for this key, including its
/// revision and expiry update. Implementations unable to uphold that contract
/// cannot construct a conforming conditional capability.
pub fn ConditionalStore(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        pub const Failure = PersistenceError(ProviderError);
        pub const ReadEffect = effects.Effect(ReadVersioned, Failure, R);
        pub const CompareEffect = effects.Effect(ConditionalAcknowledgement, Failure, R);
        userdata: ?*anyopaque,
        vtable: *const VTable,

        pub const VTable = struct {
            read_versioned: *const fn (?*anyopaque, *const R, []const u8) ReadEffect,
            compare_exchange: *const fn (?*anyopaque, *const R, []const u8, Revision, ?[]const u8, Expiry) CompareEffect,
        };

        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }

        pub fn read(comptime select_store: fn (*const R) *const Self, comptime select_key: fn (*const R) []const u8) ReadEffect {
            return ReadEffect.deferEffect(struct {
                fn run(env: *const R) ReadEffect {
                    const store = select_store(env);
                    return store.vtable.read_versioned(store.userdata, env, select_key(env));
                }
            }.run);
        }

        pub fn compareExchange(
            comptime select_store: fn (*const R) *const Self,
            comptime select_key: fn (*const R) []const u8,
            comptime select_expected: fn (*const R) Revision,
            comptime select_replacement: fn (*const R) ?[]const u8,
            comptime select_expiry: fn (*const R) Expiry,
        ) CompareEffect {
            return CompareEffect.deferEffect(struct {
                fn run(env: *const R) CompareEffect {
                    const store = select_store(env);
                    return store.vtable.compare_exchange(store.userdata, env, select_key(env), select_expected(env), select_replacement(env), select_expiry(env));
                }
            }.run);
        }
    };
}
