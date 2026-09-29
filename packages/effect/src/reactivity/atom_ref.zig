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

/// Typed atom-reference failure during write and notification operations.
pub const Error = union(enum) {
    invalid_key,
    publication: pubsub.Error,
};

/// Typed reason a committed atom update could not notify observers.
pub const InvalidationFailure = union(enum) {
    error_value: atom_registry.Error,
    defect: outcomes.Defect,
    interrupted,
    unchanged,
};

/// Atom write result distinguishing committed delivery from notification loss.
pub const Write = union(enum) {
    committed: u64,
    committed_notification_lost: u64,
    committed_without_invalidation: InvalidationFailure,
};

/// Specialize a reactive atom reference by value and environment types.
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// Returns (`type`): reactive reference bound to an atom definition, registry, and environment.
pub fn AtomRef(comptime T: type) type {
    return struct {
        const Self = @This();
        const Ref = subscription_ref.SubscriptionRef(T);
        /// Subscription snapshot type emitted by this reference.
        pub const Observation = Ref.Observation;

        values: Ref,
        registry: *atom_registry.Registry,
        key: outcomes.OwnedSlice(u8),

        /// `initial` is cloned into the subscription ref; its caller retains
        /// the original ownership. The key is copied from the caller's R.
        ///
        /// Parameters:
        /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `registry: *atom_registry.Registry`: Caller-supplied capability or scoped state; no concrete backend is
        ///   selected here.
        /// - `key: []const u8`: Stable identity used for matching, deduplication, or replay.
        /// - `initial: T`: Initial atom value, owned by the created reference.
        /// - `capacity: usize`: buffer capacity for atom change notifications.
        /// Returns (`outcomes.Exit(Self, Error)`): Carries `Self` on `.success` or `Error` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
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

        /// Look up the selected value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`outcomes.Exit(T, Error)`): Carries `T` on `.success` or `Error` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn get(self: *Self) outcomes.Exit(T, Error) {
            return switch (self.values.get()) {
                .success => |value| .{ .success = value },
                .fail => |problem| .{ .fail = .{ .publication = problem } },
                .die => |defect| .{ .die = defect },
                .interrupted => .interrupted,
                .finalization_failed => unreachable,
            };
        }

        /// Read the current observable state.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`outcomes.Exit(*Observation, Error)`): Carries `*Observation` on `.success` or `Error` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn observe(self: *Self) outcomes.Exit(*Observation, Error) {
            return switch (self.values.observe()) {
                .success => |value| .{ .success = value },
                .fail => |problem| .{ .fail = .{ .publication = problem } },
                .die => |defect| .{ .die = defect },
                .interrupted => .interrupted,
                .finalization_failed => unreachable,
            };
        }

        /// Set the atom value immediately and publish the change.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `value: T`: new atom value committed before observer notification.
        /// Returns (`outcomes.Exit(Write, Error)`): Carries `Write` on `.success` or `Error` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
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

        /// Replace the current value.
        /// Parameters:
        /// - `R: comptime type`: Caller-defined environment type supplying the dependencies needed by this operation.
        /// - `select_ref: comptime fn (*const R) *Self`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// - `select_value: comptime fn (*const R) T`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`effects.Effect(Write, Error, R)`): committed atom revision with explicit notification-delivery status.
        pub fn set(comptime R: type, comptime select_ref: fn (*const R) *Self, comptime select_value: fn (*const R) T) effects.Effect(Write, Error, R) {
            return effects.Effect(Write, Error, R).fromOutcome(struct {
                fn run(env: *const R) outcomes.Exit(Write, Error) {
                    return select_ref(env).setNow(select_value(env));
                }
            }.run);
        }

        /// Stop the capability and release its retained resources.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn shutdown(self: *Self) void {
            self.values.shutdown();
        }

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            self.values.deinit();
            self.key.deinit();
            self.* = undefined;
        }
    };
}
