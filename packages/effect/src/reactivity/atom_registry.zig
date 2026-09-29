//! Per-scope atom dependency and invalidation authority. Graph mutations and
//! transitive invalidation are serialized here; bounded wakeups remain hints
//! and generation reads remain the authoritative local reconciliation path.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const invalidation = @import("invalidation_registry.zig");
const graph_module = @import("internal/dependency_graph.zig");

/// Typed atom-registry failure during identity, dependency, or subscription operations.
pub const Error = union(enum) {
    graph: graph_module.Error,
    invalidation: invalidation.Error,
    closed,
};

/// Change in an atom’s declared dependency relationships.
pub const DependencyUpdate = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    graph_generation: u64,
    invalidation: outcomes.Exit(invalidation.InvalidateResult, Error),

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        self.invalidation.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        const copied = self.invalidation.clone();
        if (copied == .die and copied.die.kind == .out_of_memory and
            !(self.invalidation == .die and self.invalidation.die.kind == .out_of_memory))
            return error.OutOfMemory;
        return .{ .graph_generation = self.graph_generation, .invalidation = copied };
    }
};

/// Typed atom definitions, revisions, and subscriptions owned by the caller.
pub const Registry = struct {
    const Self = @This();
    allocator: std.mem.Allocator,
    io: std.Io,
    mutex: std.Io.Mutex = .init,
    graph: graph_module.Graph,
    invalidations: invalidation.Registry,
    closed: bool = false,

    /// Construct the selected resource or definition.
    /// Parameters:
    /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
    /// - `graph_limits: graph_module.Limits`: Caller-supplied graph traversal bounds.
    /// - `notification_capacity: usize`: Maximum buffered invalidation notices for atom observers.
    /// Returns (`outcomes.Exit(*Self, Error)`): Carries `*Self` on `.success` or `Error` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
    pub fn create(env: anytype, graph_limits: graph_module.Limits, notification_capacity: usize) outcomes.Exit(*Self, Error) {
        const owner = env.allocator.create(Self) catch return .{ .die = outcomes.Defect.out_of_memory };
        const prepared_invalidation = invalidation.Registry.init(env, notification_capacity);
        var invalidations = switch (prepared_invalidation) {
            .success => |value| value,
            .fail => |problem| {
                env.allocator.destroy(owner);
                return .{ .fail = .{ .invalidation = problem } };
            },
            .die => |defect| {
                env.allocator.destroy(owner);
                return .{ .die = defect };
            },
            .interrupted => {
                env.allocator.destroy(owner);
                return .interrupted;
            },
            .finalization_failed => unreachable,
        };
        const prepared_graph = graph_module.Graph.init(env, graph_limits);
        const graph = switch (prepared_graph) {
            .success => |value| value,
            .fail => |problem| {
                invalidations.deinit();
                env.allocator.destroy(owner);
                return .{ .fail = .{ .graph = problem } };
            },
            .die => |defect| {
                invalidations.deinit();
                env.allocator.destroy(owner);
                return .{ .die = defect };
            },
            .interrupted => {
                invalidations.deinit();
                env.allocator.destroy(owner);
                return .interrupted;
            },
            .finalization_failed => unreachable,
        };
        owner.* = .{ .allocator = env.allocator, .io = env.io, .graph = graph, .invalidations = invalidations };
        return .{ .success = owner };
    }

    fn invalidateLocked(self: *Self, key: []const u8) outcomes.Exit(invalidation.InvalidateResult, Error) {
        const discovered = self.graph.dependents(key);
        var affected = switch (discovered) {
            .success => |value| value,
            .fail => |problem| return .{ .fail = .{ .graph = problem } },
            .die => |defect| return .{ .die = defect },
            .interrupted => return .interrupted,
            .finalization_failed => unreachable,
        };
        defer affected.deinit();
        const count = std.math.add(usize, affected.items.len, 1) catch
            return .{ .die = outcomes.Defect.out_of_memory };
        const keys = self.allocator.alloc([]const u8, count) catch
            return .{ .die = outcomes.Defect.out_of_memory };
        defer self.allocator.free(keys);
        keys[0] = key;
        for (affected.items, keys[1..]) |owned, *slot| slot.* = owned.items;
        return switch (self.invalidations.invalidateBatch(keys)) {
            .success => |result| .{ .success = result },
            .fail => |problem| .{ .fail = .{ .invalidation = problem } },
            .die => |defect| .{ .die = defect },
            .interrupted => .interrupted,
            .finalization_failed => unreachable,
        };
    }

    /// Invalidate the selected cached or observable value.
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `key: []const u8`: Stable identity used for matching, deduplication, or replay.
    /// Returns (`outcomes.Exit(invalidation.InvalidateResult, Error)`): Carries `invalidation.InvalidateResult` on `.success` or `Error` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
    pub fn invalidate(self: *Self, key: []const u8) outcomes.Exit(invalidation.InvalidateResult, Error) {
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        if (self.closed) return .{ .fail = .closed };
        return self.invalidateLocked(key);
    }

    /// Collect dependent keys before publishing one versioned notification
    /// batch. The graph and publication boundary are scoped to this registry.
    ///
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `input: []const []const u8`: Borrowed input; keep its owner alive until the operation has copied or joined
    ///   it.
    /// Returns (`outcomes.Exit(invalidation.InvalidateResult, Error)`): Carries `invalidation.InvalidateResult` on `.success` or `Error` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
    pub fn invalidateBatch(self: *Self, input: []const []const u8) outcomes.Exit(invalidation.InvalidateResult, Error) {
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        if (self.closed) return .{ .fail = .closed };
        var keys = std.array_list.Managed([]const u8).init(self.allocator);
        defer keys.deinit();
        var retained = std.array_list.Managed(outcomes.OwnedSlice(outcomes.OwnedSlice(u8))).init(self.allocator);
        defer {
            for (retained.items) |*batch| batch.deinit();
            retained.deinit();
        }
        for (input) |key| {
            keys.append(key) catch return .{ .die = outcomes.Defect.out_of_memory };
            const found = self.graph.dependents(key);
            const dependents = switch (found) {
                .success => |value| value,
                .fail => |problem| return .{ .fail = .{ .graph = problem } },
                .die => |defect| return .{ .die = defect },
                .interrupted => return .interrupted,
                .finalization_failed => unreachable,
            };
            retained.append(dependents) catch {
                var abandoned = dependents;
                abandoned.deinit();
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            for (retained.items[retained.items.len - 1].items) |item|
                keys.append(item.items) catch return .{ .die = outcomes.Defect.out_of_memory };
        }
        return switch (self.invalidations.invalidateBatch(keys.items)) {
            .success => |value| .{ .success = value },
            .fail => |problem| .{ .fail = .{ .invalidation = problem } },
            .die => |defect| .{ .die = defect },
            .interrupted => .interrupted,
            .finalization_failed => unreachable,
        };
    }

    /// The graph transition is committed before notification. The nested
    /// invalidation Exit preserves a later notification failure without
    /// pretending that the dependency replacement was rolled back.
    ///
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `key: []const u8`: Stable identity used for matching, deduplication, or replay.
    /// - `dependencies: []const []const u8`: Dependency definitions required before constructing this component.
    /// Returns (`outcomes.Exit(DependencyUpdate, Error)`): Carries `DependencyUpdate` on `.success` or `Error` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
    pub fn replaceDependencies(self: *Self, key: []const u8, dependencies: []const []const u8) outcomes.Exit(DependencyUpdate, Error) {
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        if (self.closed) return .{ .fail = .closed };
        const replaced = self.graph.replace(key, dependencies);
        const generation = switch (replaced) {
            .success => |value| value,
            .fail => |problem| return .{ .fail = .{ .graph = problem } },
            .die => |defect| return .{ .die = defect },
            .interrupted => return .interrupted,
            .finalization_failed => unreachable,
        };
        const notice = self.invalidateLocked(key);
        return .{ .success = .{ .graph_generation = generation, .invalidation = notice } };
    }

    /// Subscribe to invalidation or atom changes.
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `key: []const u8`: Stable identity used for matching, deduplication, or replay.
    /// Returns (`outcomes.Exit(*invalidation.Registry.Watch, Error)`): Carries `*invalidation.Registry.Watch` on `.success` or `Error` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
    pub fn watch(self: *Self, key: []const u8) outcomes.Exit(*invalidation.Registry.Watch, Error) {
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        if (self.closed) return .{ .fail = .closed };
        return switch (self.invalidations.watch(key)) {
            .success => |value| .{ .success = value },
            .fail => |problem| .{ .fail = .{ .invalidation = problem } },
            .die => |defect| .{ .die = defect },
            .interrupted => .interrupted,
            .finalization_failed => unreachable,
        };
    }

    /// Expose the subscription source for atom invalidations.
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`*invalidation.Registry`): A borrowed pointer; keep its owner alive while using it.
    pub fn invalidationSource(self: *Self) *invalidation.Registry {
        return &self.invalidations;
    }

    /// Stop the capability and release its retained resources.
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn shutdown(self: *Self) void {
        self.mutex.lockUncancelable(self.io);
        self.closed = true;
        self.mutex.unlock(self.io);
        self.graph.shutdown();
        self.invalidations.shutdown();
    }

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *Self) void {
        self.shutdown();
        self.graph.deinit();
        self.invalidations.deinit();
        const allocator = self.allocator;
        self.* = undefined;
        allocator.destroy(self);
    }
};
