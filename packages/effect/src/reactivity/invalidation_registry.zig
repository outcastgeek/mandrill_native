//! Scoped keyed invalidation. The registry's generation map is authoritative
//! for local observation; bounded PubSub events only wake observers. A gap
//! forces a generation resync, so dropping an old wakeup cannot erase a change.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const pubsub = @import("../pub_sub.zig");
const keys = @import("../primary_key.zig");
const Cancellation = @import("../internal/runtime.zig").Cancellation;

/// Typed invalidation-registry failure during revision or subscription operations.
pub const Error = union(enum) {
    bus: pubsub.Error,
    invalid_key,
    closed,
    resource_limit,
    version_exhausted,
};

/// Committed revision or committed state whose notification could not be delivered.
pub const InvalidateResult = union(enum) {
    unchanged,
    committed: u64,
    committed_notification_lost: u64,
};

/// Subscription notice carrying a changed revision or a need to resynchronize.
pub const Notice = union(enum) {
    changed: u64,
    resync_required: u64,
};

/// Set of invalidation notices committed together.
pub const Batch = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    generation: u64,
    keys: outcomes.OwnedSlice(outcomes.OwnedSlice(u8)),

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        self.keys.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{ .generation = self.generation, .keys = try self.keys.clone() };
    }
};

/// Revisioned invalidation state and subscriber management using caller-owned I/O.
pub const Registry = struct {
    const Self = @This();
    const Bus = pubsub.PubSub(Batch);
    /// Subscriber cursor used to observe invalidations.
    pub const Watch = struct {
        registry: *Self,
        subscription: *Bus.Subscription,
        key: outcomes.OwnedSlice(u8),
        initial_generation: u64,

        /// Advance to the next item or state.
        /// Parameters:
        /// - `R: comptime type`: Caller-defined environment type supplying the dependencies needed by this operation.
        /// - `self: *Watch`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`effects.Effect(Notice, Error, R)`): next invalidation notice or resynchronization marker for this watch.
        pub fn next(comptime R: type, self: *Watch) effects.Effect(Notice, Error, R) {
            return effects.Effect(Notice, Error, R).fromNativeCancellableBorrowed(Watch, self, struct {
                fn run(current_watch: *const Watch, _: *const R, token: ?*const Cancellation) outcomes.Exit(Notice, Error) {
                    while (true) {
                        var delivery = switch (current_watch.subscription.takeToken(token)) {
                            .success => |value| value,
                            .fail => |problem| return .{ .fail = .{ .bus = problem } },
                            .die => |defect| return .{ .die = defect },
                            .interrupted => return .interrupted,
                            .finalization_failed => unreachable,
                        };
                        switch (delivery) {
                            .gap => {
                                delivery.deinit();
                                const latest = current_watch.registry.generation(current_watch.key.items);
                                return switch (latest) {
                                    .success => |value| .{ .success = .{ .resync_required = value } },
                                    .fail => |problem| .{ .fail = problem },
                                    .die => |defect| .{ .die = defect },
                                    .interrupted => .interrupted,
                                    .finalization_failed => unreachable,
                                };
                            },
                            .end => {
                                delivery.deinit();
                                return .{ .fail = .closed };
                            },
                            .item => |batch| {
                                var matched = false;
                                for (batch.keys.items) |entry| {
                                    if (std.mem.eql(u8, entry.items, current_watch.key.items)) {
                                        matched = true;
                                        break;
                                    }
                                }
                                const version = batch.generation;
                                delivery.deinit();
                                if (matched) return .{ .success = .{ .changed = version } };
                            },
                        }
                    }
                }
            }.run);
        }

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Watch`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Watch) void {
            const registry = self.registry;
            self.subscription.deinit();
            self.key.deinit();
            registry.mutex.lockUncancelable(registry.io);
            std.debug.assert(registry.active_watches > 0);
            registry.active_watches -= 1;
            registry.mutex.unlock(registry.io);
            registry.allocator.destroy(self);
        }
    };

    allocator: std.mem.Allocator,
    io: std.Io,
    mutex: std.Io.Mutex = .init,
    generations: std.StringHashMap(u64),
    bus: Bus,
    version: u64 = 0,
    active_watches: usize = 0,
    closed: bool = false,

    /// Capacity is caller policy. Old wakeups may be dropped; a watcher then
    /// receives a gap and reads the authoritative generation before rerunning.
    ///
    /// Parameters:
    /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
    /// - `capacity: usize`: maximum retained invalidation notices.
    /// Returns (`outcomes.Exit(Self, Error)`): Carries `Self` on `.success` or `Error` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
    pub fn init(env: anytype, capacity: usize) outcomes.Exit(Self, Error) {
        const R = @typeInfo(@TypeOf(env)).pointer.child;
        if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
            @compileError("invalidation registry requires caller allocator in R");
        if (R == void or !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
            @compileError("invalidation registry requires caller io in R");
        const prepared = Bus.init(env, capacity, 0, .drop_oldest);
        return switch (prepared) {
            .success => |bus| .{ .success = .{
                .allocator = env.allocator,
                .io = env.io,
                .generations = std.StringHashMap(u64).init(env.allocator),
                .bus = bus,
            } },
            .fail => |problem| .{ .fail = .{ .bus = problem } },
            .die => |defect| .{ .die = defect },
            .interrupted => .interrupted,
            .finalization_failed => unreachable,
        };
    }

    /// Read the current invalidation generation.
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `key: []const u8`: Stable identity used for matching, deduplication, or replay.
    /// Returns (`outcomes.Exit(u64, Error)`): Carries `u64` on `.success` or `Error` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
    pub fn generation(self: *Self, key: []const u8) outcomes.Exit(u64, Error) {
        if (keys.inspectEncoded(key) == .err) return .{ .fail = .invalid_key };
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        if (self.closed) return .{ .fail = .closed };
        return .{ .success = self.generations.get(key) orelse 0 };
    }

    /// Subscribe first, then capture the current generation. An invalidation
    /// crossing this boundary is either visible in the snapshot or queued in
    /// the subscriber; a duplicate wakeup is harmless.
    ///
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `key: []const u8`: Stable identity used for matching, deduplication, or replay.
    /// Returns (`outcomes.Exit(*Watch, Error)`): Carries `*Watch` on `.success` or `Error` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
    pub fn watch(self: *Self, key: []const u8) outcomes.Exit(*Watch, Error) {
        if (keys.inspectEncoded(key) == .err) return .{ .fail = .invalid_key };
        const owned_key = self.allocator.dupe(u8, key) catch return .{ .die = outcomes.Defect.out_of_memory };
        const watch_ptr = self.allocator.create(Watch) catch {
            self.allocator.free(owned_key);
            return .{ .die = outcomes.Defect.out_of_memory };
        };
        const subscribed = self.bus.subscribe();
        const subscription = switch (subscribed) {
            .success => |value| value,
            .fail => |problem| {
                self.allocator.destroy(watch_ptr);
                self.allocator.free(owned_key);
                return .{ .fail = .{ .bus = problem } };
            },
            .die => |defect| {
                self.allocator.destroy(watch_ptr);
                self.allocator.free(owned_key);
                return .{ .die = defect };
            },
            else => unreachable,
        };
        self.mutex.lockUncancelable(self.io);
        if (self.closed or self.active_watches == std.math.maxInt(usize)) {
            const problem: Error = if (self.closed) .closed else .resource_limit;
            self.mutex.unlock(self.io);
            subscription.deinit();
            self.allocator.destroy(watch_ptr);
            self.allocator.free(owned_key);
            return .{ .fail = problem };
        }
        const initial = self.generations.get(key) orelse 0;
        self.active_watches += 1;
        self.mutex.unlock(self.io);
        watch_ptr.* = .{
            .registry = self,
            .subscription = subscription,
            .key = .{ .items = owned_key, .allocator = self.allocator },
            .initial_generation = initial,
        };
        return .{ .success = watch_ptr };
    }

    /// Duplicated keys in one call share a single generation and event.
    /// The map commits before publication. If allocation or shutdown loses
    /// the wakeup afterward, the result says so; a later generation read can
    /// still recover the committed local state.
    ///
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `input: []const []const u8`: Borrowed input; keep its owner alive until the operation has copied or joined
    ///   it.
    /// Returns (`outcomes.Exit(InvalidateResult, Error)`): Carries `InvalidateResult` on `.success` or `Error` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
    pub fn invalidateBatch(self: *Self, input: []const []const u8) outcomes.Exit(InvalidateResult, Error) {
        var unique = std.array_list.Managed([]const u8).init(self.allocator);
        defer unique.deinit();
        for (input) |key| {
            if (keys.inspectEncoded(key) == .err) return .{ .fail = .invalid_key };
            var present = false;
            for (unique.items) |existing| {
                if (std.mem.eql(u8, existing, key)) {
                    present = true;
                    break;
                }
            }
            if (!present) unique.append(key) catch return .{ .die = outcomes.Defect.out_of_memory };
        }
        if (unique.items.len == 0) return .{ .success = .unchanged };
        const event_keys = self.allocator.alloc(outcomes.OwnedSlice(u8), unique.items.len) catch return .{ .die = outcomes.Defect.out_of_memory };
        var copied: usize = 0;
        var event_transferred = false;
        defer if (!event_transferred) {
            for (event_keys[0..copied]) |*item| item.deinit();
            self.allocator.free(event_keys);
        };
        const map_keys = self.allocator.alloc(?outcomes.OwnedSlice(u8), unique.items.len) catch return .{ .die = outcomes.Defect.out_of_memory };
        defer {
            for (map_keys[0..copied]) |*item| {
                if (item.*) |*owned| owned.deinit();
            }
            self.allocator.free(map_keys);
        }
        for (unique.items) |key| {
            event_keys[copied] = .{ .items = self.allocator.dupe(u8, key) catch return .{ .die = outcomes.Defect.out_of_memory }, .allocator = self.allocator };
            map_keys[copied] = .{ .items = self.allocator.dupe(u8, key) catch {
                event_keys[copied].deinit();
                return .{ .die = outcomes.Defect.out_of_memory };
            }, .allocator = self.allocator };
            copied += 1;
        }
        self.mutex.lockUncancelable(self.io);
        if (self.closed) {
            self.mutex.unlock(self.io);
            return .{ .fail = .closed };
        }
        if (self.version == std.math.maxInt(u64)) {
            self.mutex.unlock(self.io);
            return .{ .fail = .version_exhausted };
        }
        const additional = std.math.cast(u32, unique.items.len) orelse
            return .{ .fail = .resource_limit };
        self.generations.ensureUnusedCapacity(additional) catch {
            self.mutex.unlock(self.io);
            return .{ .die = outcomes.Defect.out_of_memory };
        };
        self.version += 1;
        const version = self.version;
        for (unique.items, 0..) |key, index| {
            if (self.generations.getPtr(key)) |existing| {
                existing.* = version;
            } else {
                self.generations.putAssumeCapacity(map_keys[index].?.items, version);
                map_keys[index] = null;
            }
        }
        self.mutex.unlock(self.io);
        var event: Batch = .{ .generation = version, .keys = .{ .items = event_keys, .allocator = self.allocator } };
        event_transferred = true;
        defer event.deinit();
        const published = self.bus.publishImmediate(event);
        return switch (published) {
            .success => |result| switch (result) {
                .published => .{ .success = .{ .committed = version } },
                .dropped => .{ .success = .{ .committed_notification_lost = version } },
            },
            .fail, .die, .interrupted, .finalization_failed => .{ .success = .{ .committed_notification_lost = version } },
        };
    }

    /// Invalidate the selected cached or observable value.
    /// Parameters:
    /// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
    /// - `select_registry: comptime fn (*const R) *Self`: Compile-time typed selector from the current environment; its result
    ///   must remain valid through use.
    /// - `select_keys: comptime fn (*const R) []const []const u8`: Compile-time typed selector from the current environment; its result must
    ///   remain valid through use.
    /// Returns (`effects.Effect(InvalidateResult, Error, R)`): commit report for invalidating the selected set of dependencies.
    pub fn invalidate(
        comptime R: type,
        comptime select_registry: fn (*const R) *Self,
        comptime select_keys: fn (*const R) []const []const u8,
    ) effects.Effect(InvalidateResult, Error, R) {
        return effects.Effect(InvalidateResult, Error, R).fromNativeCancellableBorrowed(void, &{}, struct {
            fn run(_: *const void, env: *const R, token: ?*const Cancellation) outcomes.Exit(InvalidateResult, Error) {
                if (token) |cancellation| if (cancellation.isRequested()) return .interrupted;
                return select_registry(env).invalidateBatch(select_keys(env));
            }
        }.run);
    }

    /// Invalidate one registered reactive key.
    /// Parameters:
    /// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
    /// - `select_registry: comptime fn (*const R) *Self`: Compile-time typed selector from the current environment; its result
    ///   must remain valid through use.
    /// - `select_key: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result must
    ///   remain valid through use.
    /// Returns (`effects.Effect(InvalidateResult, Error, R)`): commit report for invalidating the selected dependency.
    pub fn invalidateOne(
        comptime R: type,
        comptime select_registry: fn (*const R) *Self,
        comptime select_key: fn (*const R) []const u8,
    ) effects.Effect(InvalidateResult, Error, R) {
        return effects.Effect(InvalidateResult, Error, R).fromNativeCancellableBorrowed(void, &{}, struct {
            fn run(_: *const void, env: *const R, token: ?*const Cancellation) outcomes.Exit(InvalidateResult, Error) {
                if (token) |cancellation| if (cancellation.isRequested()) return .interrupted;
                const one = [_][]const u8{select_key(env)};
                return select_registry(env).invalidateBatch(&one);
            }
        }.run);
    }

    /// Run a scoped body while observing invalidations.
    /// Parameters:
    /// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
    /// - `A: comptime type`: Value type produced by a successful computation.
    /// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
    /// - `select_registry: comptime fn (*const R) *Self`: Compile-time typed selector from the current environment; its result
    ///   must remain valid through use.
    /// - `select_key: comptime fn (*const R) []const u8`: Compile-time typed selector from the current environment; its result must
    ///   remain valid through use.
    /// - `map_error: comptime fn (Error) E`: Compile-time typed function selected at comptime; no runtime callback lookup
    ///   is required.
    /// - `use: comptime fn (*Watch) effects.Effect(A, E, R)`: Compile-time typed function selected at comptime; no runtime callback lookup is
    ///   required.
    /// Returns (`effects.Effect(A, E, R)`): body result while a watch remains registered and later released.
    pub fn withWatch(
        comptime R: type,
        comptime A: type,
        comptime E: type,
        comptime select_registry: fn (*const R) *Self,
        comptime select_key: fn (*const R) []const u8,
        comptime map_error: fn (Error) E,
        comptime use: fn (*Watch) effects.Effect(A, E, R),
    ) effects.Effect(A, E, R) {
        const Acquire = effects.Effect(*Watch, Error, R);
        const acquire = Acquire.fromOutcome(struct {
            fn run(env: *const R) outcomes.Exit(*Watch, Error) {
                return select_registry(env).watch(select_key(env));
            }
        }.run).mapError(map_error);
        return acquire.acquireRelease(use, struct {
            fn release(current_watch: *Watch, _: outcomes.PrimaryOutcome(E)) effects.Effect(void, E, R) {
                current_watch.deinit();
                return effects.Effect(void, E, R).succeed({});
            }
        }.release);
    }

    /// Stop the capability and release its retained resources.
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn shutdown(self: *Self) void {
        self.mutex.lockUncancelable(self.io);
        self.closed = true;
        self.mutex.unlock(self.io);
        self.bus.shutdown();
    }

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *Self) void {
        self.shutdown();
        self.mutex.lockUncancelable(self.io);
        std.debug.assert(self.active_watches == 0);
        self.mutex.unlock(self.io);
        var iterator = self.generations.iterator();
        while (iterator.next()) |entry| self.allocator.free(entry.key_ptr.*);
        self.generations.deinit();
        self.bus.deinit();
        self.* = undefined;
    }
};
