//! Scoped local atom state with an atomic value/subscription boundary.
//! Invalidation follows successful publication; a failed notification is
//! visible as a committed write requiring generation-based reconciliation.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const keys = @import("../primary_key.zig");
const pubsub = @import("../pub_sub.zig");
const subscription_ref = @import("../subscription_ref.zig");
const atom_registry = @import("atom_registry.zig");

pub const Error = union(enum) {
    invalid_key,
    publication: pubsub.Error,
};

pub const InvalidationFailure = union(enum) {
    error_value: atom_registry.Error,
    defect: outcomes.Defect,
    interrupted,
    unchanged,
};

pub const Write = union(enum) {
    committed: u64,
    committed_notification_lost: u64,
    committed_without_invalidation: InvalidationFailure,
};

pub fn AtomRef(comptime T: type) type {
    return struct {
        const Self = @This();
        const Ref = subscription_ref.SubscriptionRef(T);
        pub const Observation = Ref.Observation;

        values: Ref,
        registry: *atom_registry.Registry,
        key: outcomes.OwnedSlice(u8),

        /// `initial` is cloned into the subscription ref; its caller retains
        /// the original ownership. The key is copied from the caller's R.
        pub fn init(env: anytype, registry: *atom_registry.Registry, key: []const u8, initial: T, capacity: usize) outcomes.Exit(Self, Error) {
            if (keys.inspectEncoded(key) == .err) return .{ .fail = .invalid_key };
            const owned_key = env.allocator.dupe(u8, key) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            const prepared = Ref.init(env, initial, capacity);
            return switch (prepared) {
                .success => |values| .{ .success = .{
                    .values = values,
                    .registry = registry,
                    .key = .{ .items = owned_key, .allocator = env.allocator },
                } },
                .fail => |problem| blk: {
                    env.allocator.free(owned_key);
                    break :blk .{ .fail = .{ .publication = problem } };
                },
                .die => |defect| blk: {
                    env.allocator.free(owned_key);
                    break :blk .{ .die = defect };
                },
                .interrupted => blk: {
                    env.allocator.free(owned_key);
                    break :blk .interrupted;
                },
                .finalization_failed => unreachable,
            };
        }

        pub fn get(self: *Self) outcomes.Exit(T, Error) {
            return switch (self.values.get()) {
                .success => |value| .{ .success = value },
                .fail => |problem| .{ .fail = .{ .publication = problem } },
                .die => |defect| .{ .die = defect },
                .interrupted => .interrupted,
                .finalization_failed => unreachable,
            };
        }

        pub fn observe(self: *Self) outcomes.Exit(*Observation, Error) {
            return switch (self.values.observe()) {
                .success => |value| .{ .success = value },
                .fail => |problem| .{ .fail = .{ .publication = problem } },
                .die => |defect| .{ .die = defect },
                .interrupted => .interrupted,
                .finalization_failed => unreachable,
            };
        }

        pub fn setNow(self: *Self, value: T) outcomes.Exit(Write, Error) {
            const published = self.values.bus.publishImmediate(value);
            switch (published) {
                .success => |result| if (result == .dropped)
                    return .{ .fail = .{ .publication = .full } },
                .fail => |problem| return .{ .fail = .{ .publication = problem } },
                .die => |defect| return .{ .die = defect },
                .interrupted => return .interrupted,
                .finalization_failed => unreachable,
            }
            const changed = self.registry.invalidateBatch(&.{self.key.items});
            return switch (changed) {
                .success => |result| switch (result) {
                    .committed => |generation| .{ .success = .{ .committed = generation } },
                    .committed_notification_lost => |generation| .{ .success = .{ .committed_notification_lost = generation } },
                    .unchanged => .{ .success = .{ .committed_without_invalidation = .unchanged } },
                },
                .fail => |problem| .{ .success = .{ .committed_without_invalidation = .{ .error_value = problem } } },
                .die => |defect| .{ .success = .{ .committed_without_invalidation = .{ .defect = defect } } },
                .interrupted => .{ .success = .{ .committed_without_invalidation = .interrupted } },
                .finalization_failed => unreachable,
            };
        }

        pub fn set(comptime R: type, comptime select_ref: fn (*const R) *Self, comptime select_value: fn (*const R) T) effects.Effect(Write, Error, R) {
            return effects.Effect(Write, Error, R).fromOutcome(struct {
                fn run(env: *const R) outcomes.Exit(Write, Error) {
                    return select_ref(env).setNow(select_value(env));
                }
            }.run);
        }

        pub fn shutdown(self: *Self) void {
            self.values.shutdown();
        }

        pub fn deinit(self: *Self) void {
            self.values.deinit();
            self.key.deinit();
            self.* = undefined;
        }
    };
}
