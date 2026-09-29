//! Scoped dependency graph for derived atoms. Dependencies are replaced as
//! one checked transition, and cycles are rejected before publication.
//! Caller limits bound nodes, edges and traversal storage.
const std = @import("std");
const outcomes = @import("../../outcome.zig");
const keys = @import("../../primary_key.zig");

pub const Limits = struct {
    max_nodes: usize,
    max_dependencies_per_node: usize,

    pub fn valid(self: Limits) bool {
        return self.max_nodes > 0 and self.max_dependencies_per_node > 0;
    }
};

pub const Error = enum {
    invalid_policy,
    invalid_key,
    capacity_exhausted,
    dependency_limit,
    duplicate_dependency,
    cycle,
    generation_exhausted,
    closed,
};

pub const Graph = struct {
    const Self = @This();
    const Bytes = outcomes.OwnedSlice(u8);
    const Entry = struct {
        key: Bytes,
        dependencies: outcomes.OwnedSlice(Bytes),

        fn deinit(self: *Entry) void {
            self.key.deinit();
            self.dependencies.deinit();
            self.* = undefined;
        }
    };

    allocator: std.mem.Allocator,
    io: std.Io,
    mutex: std.Io.Mutex = .init,
    limits: Limits,
    entries: std.array_list.Managed(Entry),
    visited: []bool,
    work: []usize,
    generation: u64 = 0,
    closed: bool = false,

    pub fn init(env: anytype, limits: Limits) outcomes.Exit(Self, Error) {
        const R = @typeInfo(@TypeOf(env)).pointer.child;
        if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
            @compileError("dependency graph requires caller allocator in R");
        if (R == void or !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
            @compileError("dependency graph requires caller I/O in R");
        if (!limits.valid()) return .{ .fail = .invalid_policy };
        const visited = env.allocator.alloc(bool, limits.max_nodes) catch
            return .{ .die = outcomes.Defect.out_of_memory };
        const work = env.allocator.alloc(usize, limits.max_nodes) catch {
            env.allocator.free(visited);
            return .{ .die = outcomes.Defect.out_of_memory };
        };
        return .{ .success = .{ .allocator = env.allocator, .io = env.io, .limits = limits, .entries = std.array_list.Managed(Entry).init(env.allocator), .visited = visited, .work = work } };
    }

    fn copy(self: *Self, bytes: []const u8) error{OutOfMemory}!Bytes {
        return .{ .items = try self.allocator.dupe(u8, bytes), .allocator = self.allocator };
    }

    fn indexOf(self: *const Self, key: []const u8) ?usize {
        for (self.entries.items, 0..) |entry, index| {
            if (std.mem.eql(u8, entry.key.items, key)) return index;
        }
        return null;
    }

    fn contains(dependencies: []const Bytes, key: []const u8) bool {
        for (dependencies) |entry| if (std.mem.eql(u8, entry.items, key)) return true;
        return false;
    }

    fn reaches(self: *Self, start: []const u8, target: []const u8) bool {
        if (std.mem.eql(u8, start, target)) return true;
        const first = self.indexOf(start) orelse return false;
        @memset(self.visited, false);
        var head: usize = 0;
        var tail: usize = 1;
        self.visited[first] = true;
        self.work[0] = first;
        while (head < tail) : (head += 1) {
            const entry = &self.entries.items[self.work[head]];
            for (entry.dependencies.items) |dependency| {
                if (std.mem.eql(u8, dependency.items, target)) return true;
                const index = self.indexOf(dependency.items) orelse continue;
                if (self.visited[index]) continue;
                self.visited[index] = true;
                self.work[tail] = index;
                tail += 1;
            }
        }
        return false;
    }

    /// A derived node may change dependencies after evaluation. Copies and
    /// capacity reservation complete before the old edge set is released.
    pub fn replace(self: *Self, key: []const u8, dependencies: []const []const u8) outcomes.Exit(u64, Error) {
        if (keys.inspectEncoded(key) == .err) return .{ .fail = .invalid_key };
        if (dependencies.len > self.limits.max_dependencies_per_node)
            return .{ .fail = .dependency_limit };
        for (dependencies) |dependency| if (keys.inspectEncoded(dependency) == .err)
            return .{ .fail = .invalid_key };
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        if (self.closed) return .{ .fail = .closed };
        const existing = self.indexOf(key);
        if (existing == null and self.entries.items.len == self.limits.max_nodes)
            return .{ .fail = .capacity_exhausted };
        if (self.generation == std.math.maxInt(u64)) return .{ .fail = .generation_exhausted };
        for (dependencies, 0..) |dependency, index| {
            for (dependencies[0..index]) |earlier| if (std.mem.eql(u8, earlier, dependency))
                return .{ .fail = .duplicate_dependency };
            if (self.reaches(dependency, key)) return .{ .fail = .cycle };
        }
        const copied = self.allocator.alloc(Bytes, dependencies.len) catch
            return .{ .die = outcomes.Defect.out_of_memory };
        var completed: usize = 0;
        var installed = false;
        defer if (!installed) {
            for (copied[0..completed]) |*entry| entry.deinit();
            self.allocator.free(copied);
        };
        for (dependencies) |dependency| {
            copied[completed] = self.copy(dependency) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            completed += 1;
        }
        const next = self.generation + 1;
        if (existing) |index| {
            var old = self.entries.items[index].dependencies;
            self.entries.items[index].dependencies = .{ .items = copied, .allocator = self.allocator };
            installed = true;
            old.deinit();
        } else {
            var owned_key = self.copy(key) catch
                return .{ .die = outcomes.Defect.out_of_memory };
            self.entries.append(.{ .key = owned_key, .dependencies = .{ .items = copied, .allocator = self.allocator } }) catch {
                owned_key.deinit();
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            installed = true;
        }
        self.generation = next;
        return .{ .success = next };
    }

    /// Computes transitive dependents without running callbacks under the
    /// graph gate. The returned keys are owned by the caller's allocator.
    pub fn dependents(self: *Self, key: []const u8) outcomes.Exit(outcomes.OwnedSlice(Bytes), Error) {
        if (keys.inspectEncoded(key) == .err) return .{ .fail = .invalid_key };
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        if (self.closed) return .{ .fail = .closed };
        @memset(self.visited, false);
        var head: usize = 0;
        var tail: usize = 0;
        for (self.entries.items, 0..) |entry, index| {
            if (contains(entry.dependencies.items, key)) {
                self.visited[index] = true;
                self.work[tail] = index;
                tail += 1;
            }
        }
        while (head < tail) : (head += 1) {
            const source = self.entries.items[self.work[head]].key.items;
            for (self.entries.items, 0..) |entry, index| {
                if (self.visited[index] or !contains(entry.dependencies.items, source)) continue;
                self.visited[index] = true;
                self.work[tail] = index;
                tail += 1;
            }
        }
        const copied = self.allocator.alloc(Bytes, tail) catch
            return .{ .die = outcomes.Defect.out_of_memory };
        var completed: usize = 0;
        while (completed < tail) : (completed += 1) {
            copied[completed] = self.copy(self.entries.items[self.work[completed]].key.items) catch {
                for (copied[0..completed]) |*entry| entry.deinit();
                self.allocator.free(copied);
                return .{ .die = outcomes.Defect.out_of_memory };
            };
        }
        return .{ .success = .{ .items = copied, .allocator = self.allocator } };
    }

    pub fn shutdown(self: *Self) void {
        self.mutex.lockUncancelable(self.io);
        self.closed = true;
        self.mutex.unlock(self.io);
    }

    pub fn deinit(self: *Self) void {
        self.shutdown();
        for (self.entries.items) |*entry| entry.deinit();
        self.entries.deinit();
        self.allocator.free(self.visited);
        self.allocator.free(self.work);
        self.* = undefined;
    }
};
