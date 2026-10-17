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
    evaluation_generation_exhausted,
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
    evaluations: std.StringHashMap(u64),

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
        owner.* = .{ .allocator = env.allocator, .io = env.io, .graph = graph, .invalidations = invalidations, .evaluations = std.StringHashMap(u64).init(env.allocator) };
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

    /// Evaluation identity and owner revision captured at admission.
    pub const EvaluationTicket = struct { generation: u64, revision: u64 };
    pub const Observation = struct { key: []const u8, revision: u64 };
    pub const Publication = union(enum) {
        pub const effect_owned_payload = true;
        stale,
        committed: DependencyUpdate,
        pub fn deinit(self: *@This()) void {
            if (self.* == .committed) self.committed.deinit();
        }
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            return switch (self.*) {
                .stale => .stale,
                .committed => |value| .{ .committed = try value.clone() },
            };
        }
    };

    pub fn beginEvaluation(self: *Self, key: []const u8) outcomes.Exit(EvaluationTicket, Error) {
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        if (self.closed) return .{ .fail = .closed };
        const current = self.evaluations.get(key) orelse 0;
        if (current == std.math.maxInt(u64)) return .{ .fail = .evaluation_generation_exhausted };
        if (self.evaluations.getPtr(key)) |generation| generation.* = current + 1 else {
            const copied = self.allocator.dupe(u8, key) catch return .{ .die = outcomes.Defect.out_of_memory };
            self.evaluations.put(copied, current + 1) catch {
                self.allocator.free(copied);
                return .{ .die = outcomes.Defect.out_of_memory };
            };
        }
        const revision = self.invalidations.generation(key);
        return switch (revision) {
            .success => |value| .{ .success = .{ .generation = current + 1, .revision = value } },
            .fail => |e| .{ .fail = .{ .invalidation = e } },
            .die => |e| .{ .die = e },
            .interrupted => .interrupted,
            .finalization_failed => unreachable,
        };
    }

    pub fn observedRevision(self: *Self, key: []const u8) outcomes.Exit(u64, Error) {
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        if (self.closed) return .{ .fail = .closed };
        return switch (self.invalidations.generation(key)) {
            .success => |v| .{ .success = v },
            .fail => |e| .{ .fail = .{ .invalidation = e } },
            .die => |e| .{ .die = e },
            .interrupted => .interrupted,
            .finalization_failed => unreachable,
        };
    }

    /// Generation, dependency revisions and graph replacement share one commit
    /// guard. Notification occurs after that committed transition and retains
    /// its independent outcome, exactly as replaceDependencies does.
    pub fn publishEvaluation(self: *Self, key: []const u8, ticket: EvaluationTicket, observations: []const Observation) outcomes.Exit(Publication, Error) {
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        if (self.closed) return .{ .fail = .closed };
        self.invalidations.mutex.lockUncancelable(self.io);
        var guarded = true;
        defer if (guarded) self.invalidations.mutex.unlock(self.io);
        if (self.invalidations.closed) return .{ .fail = .closed };
        if (self.evaluations.get(key) != ticket.generation or (self.invalidations.generations.get(key) orelse 0) != ticket.revision) return .{ .success = .stale };
        for (observations) |observed| if ((self.invalidations.generations.get(observed.key) orelse 0) != observed.revision) return .{ .success = .stale };
        if (ticket.generation == std.math.maxInt(u64)) return .{ .fail = .evaluation_generation_exhausted };
        const dependencies = self.allocator.alloc([]const u8, observations.len) catch return .{ .die = outcomes.Defect.out_of_memory };
        defer self.allocator.free(dependencies);
        for (observations, dependencies) |observed, *slot| slot.* = observed.key;
        const replaced = self.graph.replace(key, dependencies);
        const generation = switch (replaced) {
            .success => |g| g,
            .fail => |e| return .{ .fail = .{ .graph = e } },
            .die => |e| return .{ .die = e },
            .interrupted => return .interrupted,
            .finalization_failed => unreachable,
        };
        // Consuming this generation makes duplicate/copy publication stale.
        self.evaluations.getPtr(key).?.* = ticket.generation + 1;
        self.invalidations.mutex.unlock(self.io);
        guarded = false;
        return .{ .success = .{ .committed = .{ .graph_generation = generation, .invalidation = self.invalidateLocked(key) } } };
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
        var evaluations = self.evaluations.keyIterator();
        while (evaluations.next()) |key| self.allocator.free(key.*);
        self.evaluations.deinit();
        const allocator = self.allocator;
        self.* = undefined;
        allocator.destroy(self);
    }
};
