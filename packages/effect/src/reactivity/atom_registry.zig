//! Per-scope atom dependency and invalidation authority. Graph mutations and
//! transitive invalidation are serialized here; bounded wakeups remain hints
//! and generation reads remain the authoritative local reconciliation path.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const invalidation = @import("invalidation_registry.zig");
const graph_module = @import("internal/dependency_graph.zig");

pub const Error = union(enum) {
    graph: graph_module.Error,
    invalidation: invalidation.Error,
    closed,
};

pub const DependencyUpdate = struct {
    pub const effect_owned_payload = true;
    graph_generation: u64,
    invalidation: outcomes.Exit(invalidation.InvalidateResult, Error),

    pub fn deinit(self: *@This()) void {
        self.invalidation.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        const copied = self.invalidation.clone();
        if (copied == .die and copied.die.kind == .out_of_memory and
            !(self.invalidation == .die and self.invalidation.die.kind == .out_of_memory))
            return error.OutOfMemory;
        return .{ .graph_generation = self.graph_generation, .invalidation = copied };
    }
};

pub const Registry = struct {
    const Self = @This();
    allocator: std.mem.Allocator,
    io: std.Io,
    mutex: std.Io.Mutex = .init,
    graph: graph_module.Graph,
    invalidations: invalidation.Registry,
    closed: bool = false,

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

    pub fn invalidate(self: *Self, key: []const u8) outcomes.Exit(invalidation.InvalidateResult, Error) {
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        if (self.closed) return .{ .fail = .closed };
        return self.invalidateLocked(key);
    }

    /// Collect dependent keys before publishing one versioned notification
    /// batch. The graph and publication boundary are scoped to this registry.
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

    pub fn invalidationSource(self: *Self) *invalidation.Registry {
        return &self.invalidations;
    }

    pub fn shutdown(self: *Self) void {
        self.mutex.lockUncancelable(self.io);
        self.closed = true;
        self.mutex.unlock(self.io);
        self.graph.shutdown();
        self.invalidations.shutdown();
    }

    pub fn deinit(self: *Self) void {
        self.shutdown();
        self.graph.deinit();
        self.invalidations.deinit();
        const allocator = self.allocator;
        self.* = undefined;
        allocator.destroy(self);
    }
};
