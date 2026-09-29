//! Bounded, cancellation-safe admission with a scoped Effect entry point.
//! A successful acquisition is owned by acquireRelease before user work runs.
const std = @import("std");
const outcomes = @import("outcome.zig");
const effects = @import("effect.zig");
const Cancellation = @import("internal/runtime.zig").Cancellation;
const Waiter = @import("internal/waiter.zig").Waiter;
const keys = @import("primary_key.zig");

/// Permit-gate failure for invalid capacity, shutdown, or invalid release.
pub const Error = enum { invalid_capacity, shutdown, invalid_release };

/// I/O-aware permit gate with a fixed limit and synchronized availability.
pub const Semaphore = struct {
    const Self = @This();
    const Admission = struct {
        waiter: Waiter,
        token: ?*const Cancellation,
        queued: bool = true,
        granted: bool = false,
    };

    io: std.Io,
    limit: usize,
    available: usize,
    mutex: std.Io.Mutex = .init,
    waiters: ?*Waiter = null,
    waiter_tail: ?*Waiter = null,
    closed: bool = false,

    /// Capacity is an explicit, positive caller policy; there is no default.
    ///
    /// Parameters:
    /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
    /// - `capacity: usize`: initial positive maximum number of concurrent permits.
    /// Returns (`outcomes.Result(Self, Error)`): An ok/err Result; owned payloads must be released according to their type contract.
    pub fn init(env: anytype, capacity: usize) outcomes.Result(Self, Error) {
        if (capacity == 0) return .{ .err = .invalid_capacity };
        return .{ .ok = .{ .io = env.io, .limit = capacity, .available = capacity } };
    }

    fn acquireToken(self: *Self, token: ?*const Cancellation) outcomes.Exit(void, Error) {
        self.mutex.lockUncancelable(self.io);
        if (if (token) |cancellation| cancellation.isRequested() else false) {
            self.mutex.unlock(self.io);
            return .interrupted;
        }
        if (self.closed) {
            self.mutex.unlock(self.io);
            return .{ .fail = .shutdown };
        }
        if (self.available != 0) {
            self.available -= 1;
            self.mutex.unlock(self.io);
            return .{ .success = {} };
        }

        var admission: Admission = .{ .waiter = .{ .io = self.io }, .token = token };
        if (self.waiters == null) self.waiter_tail = &admission.waiter;
        admission.waiter.register(&self.waiters, token);
        self.mutex.unlock(self.io);
        _ = admission.waiter.event.wait(self.io) catch {};
        admission.waiter.registration.unregister();

        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        if (admission.queued) {
            if (self.waiter_tail == &admission.waiter) self.waiter_tail = admission.waiter.previous;
            admission.waiter.remove(&self.waiters);
        }
        // An already committed handoff wins over a later cancellation.
        if (admission.granted) return .{ .success = {} };
        if (self.closed) return .{ .fail = .shutdown };
        return .interrupted;
    }

    fn release(self: *Self) outcomes.Exit(void, Error) {
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        if (!self.closed) {
            // Waiters register at the head; the tail is the oldest owner.
            while (self.waiter_tail) |oldest| {
                const admission: *Admission = @alignCast(@fieldParentPtr("waiter", oldest));
                self.waiter_tail = oldest.previous;
                oldest.remove(&self.waiters);
                admission.queued = false;
                if (if (admission.token) |token| token.isRequested() else false) {
                    oldest.event.set(self.io);
                    continue;
                }
                admission.granted = true;
                oldest.event.set(self.io);
                return .{ .success = {} };
            }
        }
        if (self.available == self.limit) return .{ .fail = .invalid_release };
        self.available += 1;
        return .{ .success = {} };
    }

    /// The only public acquisition path owns the permit through a structured
    /// finalizer. The body keeps its own typed E; admission errors are mapped
    /// explicitly instead of silently becoming defects or native error unions.
    ///
    /// Parameters:
    /// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
    /// - `A: comptime type`: Value type produced by a successful computation.
    /// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
    /// - `select: comptime fn (*const R) *Self`: Compile-time typed function selected at comptime; no runtime callback lookup is
    ///   required.
    /// - `map_error: comptime fn (Error) E`: Compile-time typed function selected at comptime; no runtime callback lookup
    ///   is required.
    /// - `use: comptime fn (void) effects.Effect(A, E, R)`: Compile-time typed function selected at comptime; no runtime callback lookup is
    ///   required.
    /// Returns (`effects.Effect(A, E, R)`): body result after a permit is acquired and released even on failure.
    pub fn withPermit(
        comptime R: type,
        comptime A: type,
        comptime E: type,
        comptime select: fn (*const R) *Self,
        comptime map_error: fn (Error) E,
        comptime use: fn (void) effects.Effect(A, E, R),
    ) effects.Effect(A, E, R) {
        const Acquire = effects.Effect(void, Error, R);
        const acquire = Acquire.fromNativeCancellableBorrowed(void, &{}, struct {
            fn run(_: *const void, env: *const R, token: ?*const Cancellation) outcomes.Exit(void, Error) {
                return select(env).acquireToken(token);
            }
        }.run).mapError(map_error);
        return acquire.acquireRelease(use, struct {
            fn finish(_: void, _: outcomes.PrimaryOutcome(E)) effects.Effect(void, E, R) {
                return effects.Effect(void, Error, R).fromOutcome(struct {
                    fn run(env: *const R) outcomes.Exit(void, Error) {
                        return select(env).release();
                    }
                }.run).mapError(map_error);
            }
        }.finish);
    }

    /// Stop new admission and wake queued callers. Already owned permits must
    /// still return before deinitialization; shutdown does not revoke them.
    ///
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn shutdown(self: *Self) void {
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        self.closed = true;
        Waiter.signalAll(self.waiters);
    }

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *Self) void {
        self.mutex.lockUncancelable(self.io);
        const drained = self.waiters == null and self.waiter_tail == null and self.available == self.limit;
        self.mutex.unlock(self.io);
        std.debug.assert(drained);
        self.* = undefined;
    }
};

/// Typed failure from validating a partition key or acquiring its semaphore.
pub const PartitionError = union(enum) {
    invalid_capacity,
    invalid_key: keys.Error,
    partition_limit,
    shutdown,
    resource_limit,
    semaphore: Error,
};

/// Partitions exist only while an acquisition or waiter owns them. Unrelated
/// keys use independent semaphores; the map gate protects only lifecycle and
/// never remains held while a permit waits or user work runs.
pub const PartitionedSemaphore = struct {
    const Self = @This();
    const Entry = struct {
        owner: *Self,
        key: []u8,
        semaphore: Semaphore,
        users: usize,
    };

    allocator: std.mem.Allocator,
    io: std.Io,
    mutex: std.Io.Mutex = .init,
    max_partitions: usize,
    permits_per_partition: usize,
    entries: std.StringHashMap(*Entry),
    closed: bool = false,

    /// Create keyed semaphores with explicit partition and permit limits.
    /// Parameters:
    /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
    /// - `max_partitions: usize`: Caller-supplied upper bound on partition state.
    /// - `permits_per_partition: usize`: The permits per partition input required by this typed
    ///   operation.
    /// Returns (`outcomes.Result(Self, PartitionError)`): An ok/err Result; owned payloads must be released according to their type contract.
    pub fn init(env: anytype, max_partitions: usize, permits_per_partition: usize) outcomes.Result(Self, PartitionError) {
        if (max_partitions == 0 or permits_per_partition == 0) return .{ .err = .invalid_capacity };
        return .{ .ok = .{
            .allocator = env.allocator,
            .io = env.io,
            .max_partitions = max_partitions,
            .permits_per_partition = permits_per_partition,
            .entries = std.StringHashMap(*Entry).init(env.allocator),
        } };
    }

    fn retainEntry(self: *Self, key: []const u8) outcomes.Exit(*Entry, PartitionError) {
        switch (keys.inspectEncoded(key)) {
            .ok => {},
            .err => |problem| return .{ .fail = .{ .invalid_key = problem } },
        }
        self.mutex.lockUncancelable(self.io);
        if (self.closed) {
            self.mutex.unlock(self.io);
            return .{ .fail = .shutdown };
        }
        if (self.entries.get(key)) |entry| {
            entry.users = std.math.add(usize, entry.users, 1) catch {
                self.mutex.unlock(self.io);
                return .{ .fail = .resource_limit };
            };
            self.mutex.unlock(self.io);
            return .{ .success = entry };
        }
        self.mutex.unlock(self.io);

        const owned_key = self.allocator.dupe(u8, key) catch return .{ .die = outcomes.Defect.out_of_memory };
        const candidate = self.allocator.create(Entry) catch {
            self.allocator.free(owned_key);
            return .{ .die = outcomes.Defect.out_of_memory };
        };
        const sem = Semaphore.init(.{ .io = self.io }, self.permits_per_partition);
        candidate.* = .{ .owner = self, .key = owned_key, .semaphore = switch (sem) {
            .ok => |value| value,
            .err => {
                self.allocator.destroy(candidate);
                self.allocator.free(owned_key);
                return .{ .fail = .invalid_capacity };
            },
        }, .users = 1 };

        self.mutex.lockUncancelable(self.io);
        if (self.closed) {
            self.mutex.unlock(self.io);
            candidate.semaphore.deinit();
            self.allocator.destroy(candidate);
            self.allocator.free(owned_key);
            return .{ .fail = .shutdown };
        }
        if (self.entries.get(key)) |existing| {
            existing.users = std.math.add(usize, existing.users, 1) catch {
                self.mutex.unlock(self.io);
                candidate.semaphore.deinit();
                self.allocator.destroy(candidate);
                self.allocator.free(owned_key);
                return .{ .fail = .resource_limit };
            };
            self.mutex.unlock(self.io);
            candidate.semaphore.deinit();
            self.allocator.destroy(candidate);
            self.allocator.free(owned_key);
            return .{ .success = existing };
        }
        if (self.entries.count() >= self.max_partitions) {
            self.mutex.unlock(self.io);
            candidate.semaphore.deinit();
            self.allocator.destroy(candidate);
            self.allocator.free(owned_key);
            return .{ .fail = .partition_limit };
        }
        self.entries.ensureUnusedCapacity(1) catch {
            self.mutex.unlock(self.io);
            candidate.semaphore.deinit();
            self.allocator.destroy(candidate);
            self.allocator.free(owned_key);
            return .{ .die = outcomes.Defect.out_of_memory };
        };
        self.entries.putAssumeCapacity(owned_key, candidate);
        self.mutex.unlock(self.io);
        return .{ .success = candidate };
    }

    fn releaseEntry(self: *Self, entry: *Entry) void {
        self.mutex.lockUncancelable(self.io);
        std.debug.assert(entry.users > 0);
        entry.users -= 1;
        const destroy = entry.users == 0;
        if (destroy) _ = self.entries.remove(entry.key);
        self.mutex.unlock(self.io);
        if (destroy) {
            entry.semaphore.deinit();
            self.allocator.free(entry.key);
            self.allocator.destroy(entry);
        }
    }

    fn acquireToken(self: *Self, key: []const u8, token: ?*const Cancellation) outcomes.Exit(*Entry, PartitionError) {
        const retained = self.retainEntry(key);
        const entry = switch (retained) {
            .success => |value| value,
            else => return retained,
        };
        const acquired = entry.semaphore.acquireToken(token);
        return switch (acquired) {
            .success => .{ .success = entry },
            .fail => |problem| blk: {
                self.releaseEntry(entry);
                break :blk .{ .fail = .{ .semaphore = problem } };
            },
            .interrupted => blk: {
                self.releaseEntry(entry);
                break :blk .interrupted;
            },
            .die => |defect| blk: {
                self.releaseEntry(entry);
                break :blk .{ .die = defect };
            },
            .finalization_failed => unreachable,
        };
    }

    /// Acquire a semaphore permit for a scoped body.
    /// Parameters:
    /// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
    /// - `A: comptime type`: Value type produced by a successful computation.
    /// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
    /// - `select: comptime fn (*const R) *Self`: Compile-time typed function selected at comptime; no runtime callback lookup is
    ///   required.
    /// - `select_key: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result must
    ///   remain valid through use.
    /// - `map_error: comptime fn (PartitionError) E`: Compile-time typed function selected at comptime; no runtime callback lookup
    ///   is required.
    /// - `use: comptime fn (void) effects.Effect(A, E, R)`: Compile-time typed function selected at comptime; no runtime callback lookup is
    ///   required.
    /// Returns (`effects.Effect(A, E, R)`): body result after a partition-specific permit is acquired and released.
    pub fn withPermit(
        comptime R: type,
        comptime A: type,
        comptime E: type,
        comptime select: fn (*const R) *Self,
        comptime select_key: fn (*const R) []const u8,
        comptime map_error: fn (PartitionError) E,
        comptime use: fn (void) effects.Effect(A, E, R),
    ) effects.Effect(A, E, R) {
        const Acquire = effects.Effect(*Entry, PartitionError, R);
        const acquire = Acquire.fromNativeCancellableBorrowed(void, &{}, struct {
            fn run(_: *const void, env: *const R, token: ?*const Cancellation) outcomes.Exit(*Entry, PartitionError) {
                return select(env).acquireToken(select_key(env), token);
            }
        }.run).mapError(map_error);
        return acquire.acquireRelease(struct {
            fn body(_: *Entry) effects.Effect(A, E, R) {
                return use({});
            }
        }.body, struct {
            fn finish(entry: *Entry, _: outcomes.PrimaryOutcome(E)) effects.Effect(void, E, R) {
                return effects.Effect(void, E, R).fromBorrowed(Entry, entry, struct {
                    fn release(owner: *const Entry, _: *const R) outcomes.Exit(void, E) {
                        const active = @constCast(owner);
                        const released = active.semaphore.release();
                        const map_error_result: outcomes.Exit(void, E) = switch (released) {
                            .success => .{ .success = {} },
                            .fail => |problem| .{ .fail = map_error(.{ .semaphore = problem }) },
                            .die => |defect| .{ .die = defect },
                            .interrupted => .interrupted,
                            .finalization_failed => unreachable,
                        };
                        active.owner.releaseEntry(active);
                        return map_error_result;
                    }
                }.release);
            }
        }.finish);
    }

    /// Stop the capability and release its retained resources.
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn shutdown(self: *Self) void {
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        self.closed = true;
        var iterator = self.entries.iterator();
        while (iterator.next()) |item| item.value_ptr.*.semaphore.shutdown();
    }

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *Self) void {
        self.mutex.lockUncancelable(self.io);
        const drained = self.closed and self.entries.count() == 0;
        self.mutex.unlock(self.io);
        std.debug.assert(drained);
        self.entries.deinit();
        self.* = undefined;
    }
};
