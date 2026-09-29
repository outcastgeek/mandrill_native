//! Ordered batch storage capability. A provider may stop at an item boundary;
//! every request retains its original index, and unattempted items remain
//! distinguishable from missing records and failed operations.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const storage = @import("key_value_store.zig");
const PersistenceError = @import("persistence_error.zig").Error;

pub fn Item(comptime A: type, comptime E: type) type {
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = true;
        not_attempted,
        attempted: outcomes.Exit(A, E),

        pub fn deinit(self: *Self) void {
            if (self.* == .attempted) self.attempted.deinit();
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .not_attempted => .not_attempted,
                .attempted => |value| .{ .attempted = value.clone() },
            };
        }
    };
}

pub fn Report(comptime A: type, comptime E: type) type {
    return struct {
        const Self = @This();
        pub const effect_owned_payload = true;
        items: []Item(A, E),
        allocator: std.mem.Allocator,

        /// A conforming provider returns exactly one result per input, in
        /// request order. Once it stops, all following entries are unattempted.
        pub fn deinit(self: *Self) void {
            for (self.items) |*item| item.deinit();
            self.allocator.free(self.items);
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            const copied = try self.allocator.alloc(Item(A, E), self.items.len);
            var count: usize = 0;
            errdefer {
                for (copied[0..count]) |*item| item.deinit();
                self.allocator.free(copied);
            }
            for (self.items) |*item| {
                copied[count] = try item.clone();
                count += 1;
            }
            return .{ .items = copied, .allocator = self.allocator };
        }
    };
}

pub const Write = struct {
    key: []const u8,
    bytes: []const u8,
    expiry: storage.Expiry,
};

/// This is an ordered partial-progress capability, not an atomic multi-key
/// transaction. Failure of one item cannot be represented as whole-batch
/// success; the report records the failure at its exact index. Borrowed input
/// slices remain valid until the returned Effect completes.
pub fn BatchStore(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        pub const Failure = PersistenceError(ProviderError);
        pub const ReadReport = Report(storage.Read, Failure);
        pub const WriteReport = Report(storage.WriteAcknowledgement, Failure);
        pub const ReadEffect = effects.Effect(ReadReport, Failure, R);
        pub const WriteEffect = effects.Effect(WriteReport, Failure, R);

        userdata: ?*anyopaque,
        vtable: *const VTable,

        pub const VTable = struct {
            read_many: *const fn (?*anyopaque, *const R, []const []const u8) ReadEffect,
            write_many: *const fn (?*anyopaque, *const R, []const Write) WriteEffect,
            remove_many: *const fn (?*anyopaque, *const R, []const []const u8) WriteEffect,
        };

        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }

        pub fn read(comptime select_store: fn (*const R) *const Self, comptime select_keys: fn (*const R) []const []const u8) ReadEffect {
            return ReadEffect.deferEffect(struct {
                fn run(env: *const R) ReadEffect {
                    const store = select_store(env);
                    return store.vtable.read_many(store.userdata, env, select_keys(env));
                }
            }.run);
        }

        pub fn write(comptime select_store: fn (*const R) *const Self, comptime select_writes: fn (*const R) []const Write) WriteEffect {
            return WriteEffect.deferEffect(struct {
                fn run(env: *const R) WriteEffect {
                    const store = select_store(env);
                    return store.vtable.write_many(store.userdata, env, select_writes(env));
                }
            }.run);
        }

        pub fn remove(comptime select_store: fn (*const R) *const Self, comptime select_keys: fn (*const R) []const []const u8) WriteEffect {
            return WriteEffect.deferEffect(struct {
                fn run(env: *const R) WriteEffect {
                    const store = select_store(env);
                    return store.vtable.remove_many(store.userdata, env, select_keys(env));
                }
            }.run);
        }
    };
}
