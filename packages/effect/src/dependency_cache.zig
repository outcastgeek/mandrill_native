//! Dependency metadata follows the caller's retain/release policy, including
//! pointed-to data. Shared revision-based cache generations own their loads;
//! policy callbacks and exclusion cancel/join run outside the cache mutex.
//! Construction/configuration adds no implicit allocator, I/O or executor.
const std = @import("std");
const outcomes = @import("outcome.zig");
const cache = @import("cache.zig");

/// Generation selection recorded before waiting: retained, shared in-flight or newly loaded.
pub const CacheObservation = cache.CacheObservation;
/// An observed value and its selection provenance; marked value ownership is released or cloned with it.
pub const CacheLookup = cache.CacheLookup;
/// Caller retain/equality/release policy for each dependency, including pointed-to data.
pub const DependencyPolicy = cache.KeyPolicy;

/// Owning loader result combining A with separately retained dependency metadata.
/// Use init, takeValue, clone and deinit to make ownership transfers explicit.
pub fn Loaded(comptime D: type, comptime A: type) type {
    return struct {
        const Self = @This();
        /// Exit/Result cleanup owns this value; release it once or clone it for an independent observation.
        pub const effect_owned_payload = true;
        /// Public evidence used by the core cache’s conservative dependency invalidation.
        pub const cache_dependencies = true;
        /// Dependency identity governed by the supplied retain/equal/release policy.
        pub const Dependency = D;
        /// Owned successful payload transferred by init/takeValue.
        value: A,
        /// Individually retained dependency identities; never a shallow borrow of the loader buffer.
        dependencies: []D,
        /// Originating allocator for the dependency array.
        allocator: std.mem.Allocator,
        /// Retention/equality/release callbacks that remain valid through owner destruction.
        policy: DependencyPolicy(D),

        /// Takes ownership of value only on success; each dependency is retained
        /// separately, including any pointed-to data governed by the policy.
        pub fn init(env: anytype, value: A, dependencies: []const D, policy: DependencyPolicy(D)) error{OutOfMemory}!Self {
            const owned = try env.allocator.alloc(D, dependencies.len);
            var count: usize = 0;
            errdefer {
                for (owned[0..count]) |*dependency| policy.release(policy.userdata, dependency);
                env.allocator.free(owned);
            }
            for (dependencies, owned) |dependency, *slot| {
                slot.* = try policy.retain(policy.userdata, dependency);
                count += 1;
            }
            return .{ .value = value, .dependencies = owned, .allocator = env.allocator, .policy = policy };
        }
        /// Retain dependencies with a replacement allocator/policy, then transfer value and invalidate this owner.
        /// On failure this owner is unchanged.
        pub fn reown(self: *Self, env: anytype, policy: DependencyPolicy(D)) error{OutOfMemory}!Self {
            const replacement = try init(env, self.value, self.dependencies, policy);
            self.releaseDependencies();
            self.* = undefined;
            return replacement;
        }
        /// Compare against retained dependency identities using the caller policy; invokes no loader.
        pub fn dependsOn(self: *const Self, dependency: D) bool {
            for (self.dependencies) |candidate| if (self.policy.equal(self.policy.userdata, candidate, dependency)) return true;
            return false;
        }
        /// Transfer A out, release dependency metadata and invalidate this owner. The caller now owns any marked payload.
        pub fn takeValue(self: *Self) A {
            const value = self.value;
            self.releaseDependencies();
            self.* = undefined;
            return value;
        }
        fn releaseDependencies(self: *Self) void {
            for (self.dependencies) |*dependency| self.policy.release(self.policy.userdata, dependency);
            self.allocator.free(self.dependencies);
        }
        /// Discard the marked owning value and release every dependency with its retained policy.
        pub fn deinit(self: *Self) void {
            outcomes.discardPayload(&self.value);
            self.releaseDependencies();
            self.* = undefined;
        }
        /// Clone A and individually retain dependencies into an independent owner; the source survives allocation failure.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            var value = try outcomes.clonePayload(self.value);
            errdefer outcomes.discardPayload(&value);
            const Env = struct { allocator: std.mem.Allocator };
            return init(&Env{ .allocator = self.allocator }, value, self.dependencies, self.policy);
        }
    };
}

/// Finite dependency-cache retention and explicit positive loader/metadata admission bounds.
pub fn Config(comptime Key: type, comptime D: type, comptime E: type) type {
    return struct {
        /// Core finite capacity, TTL, ownership and clock policy.
        retention: cache.Config(Key, E),
        /// Positive inclusive concurrent load-admission bound.
        maximum_inflight: usize,
        /// Positive inclusive retained dependency count per loaded/replacement value.
        maximum_dependencies_per_entry: usize,
        /// Callbacks governing each retained dependency, including pointed-to storage.
        dependency_policy: DependencyPolicy(D),
        /// Typed failure constructor for admission/metadata bounds.
        capacity_error: *const fn () E,
        /// Check finite positive capacity/admission limits and nonnegative optional TTL; no runtime resources are acquired.
        pub fn valid(self: @This()) bool {
            return self.retention.capacity != null and self.retention.capacity.? > 0 and
                self.maximum_inflight > 0 and self.maximum_dependencies_per_entry > 0 and
                (self.retention.time_to_live == null or self.retention.time_to_live.?.nanoseconds >= 0);
        }
        /// Project dependency-cache admission into the existing core retention policy without changing caller callbacks.
        pub fn core(self: @This()) cache.Config(Key, E) {
            var config = self.retention;
            config.maximum_inflight = self.maximum_inflight;
            config.capacity_error = self.capacity_error;
            return config;
        }
    };
}
