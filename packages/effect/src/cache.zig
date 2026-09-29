//! Scoped cache with caller-selected capacity, expiry, and in-flight sharing.
const diagnostics = @import("internal/diagnostics.zig");
const std = @import("std");
const outcomes = @import("outcome.zig");
const providers = @import("services.zig");
const execution = @import("executor.zig");
const primitives = @import("primitives.zig");
const Cancellation = @import("internal/runtime.zig").Cancellation;

/// Specialize cache key hashing, equality, and retention callbacks.
/// Parameters:
/// - `Key: comptime type`: Key type used to identify or look up entries in this component.
/// Returns (`type`): key hashing, equality, retain, and release callbacks required by the cache.
pub fn KeyPolicy(comptime Key: type) type {
    return struct {
        userdata: ?*anyopaque,
        equal: *const fn (?*anyopaque, Key, Key) bool,
        retain: *const fn (?*anyopaque, Key) error{OutOfMemory}!Key,
        release: *const fn (?*anyopaque, *Key) void,
    };
}

/// Specialize capacity, TTL, and admission settings for a typed cache.
/// Parameters:
/// - `Key: comptime type`: Key type used to identify or look up entries in this component.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): cache policy type for capacity, time-to-live, and in-flight admission.
pub fn Config(comptime Key: type, comptime E: type) type {
    return struct {
        keys: KeyPolicy(Key),
        time_to_live: ?std.Io.Duration,
        capacity: ?usize,
        // Retention capacity concerns settled entries. In-flight admission has
        // its own explicit consumer policy; null deliberately permits growth.
        maximum_inflight: ?usize,
        cache_failure: *const fn (E) bool,
    };
}

/// Specialize a cache by its key, value, and typed error channels.
/// Parameters:
/// - `Key: comptime type`: Key type used to identify or look up entries in this component.
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): shared cache type with typed keys, values, failures, and caller-selected providers.
pub fn Cache(comptime Key: type, comptime A: type, comptime E: type) type {
    return struct {
        const Self = @This();
        /// `Loader` is a callback interface with `userdata: *anyopaque`, `run: *const fn (*anyopaque,
        /// Key, *const Cancellation) outcomes.Exit(A, E)`, `deinit: *const fn (*anyopaque) void`.
        pub const Loader = struct {
            userdata: *anyopaque,
            run: *const fn (*anyopaque, Key, *const Cancellation) outcomes.Exit(A, E),
            deinit: *const fn (*anyopaque) void,
        };
        const Entry = struct {
            cache: *Self,
            key: Key,
            next: ?*Entry = null,
            references: std.atomic.Value(usize) = .init(1),
            cancellation: Cancellation = .{},
            handle: execution.Registered = undefined,
            accepted: bool = false,
            submitted: bool = false,
            claimed: bool = false,
            settled: bool = false,
            retained: bool = true,
            invalidated: bool = false,
            diagnostic: bool = false,
            expires: ?std.Io.Timestamp = null,
            accessed: std.Io.Timestamp,
            outcome: outcomes.Exit(A, E) = undefined,
            completion: primitives.Deferred(A, E),
            metadata: ?*Aggregate,
            aggregation: ?*Aggregate,
            ready: std.atomic.Value(bool) = .init(false),

            fn retain(self: *Entry) void {
                _ = self.references.fetchAdd(1, .monotonic);
            }
            fn release(self: *Entry) void {
                if (self.references.fetchSub(1, .acq_rel) != 1) return;
                const cache = self.cache;
                self.completion.deinit();
                const retained = if (self.metadata) |metadata| metadata.storage.retained() else false;
                if (self.settled) self.outcome.deinit();
                if (!retained) if (self.metadata) |metadata| cache.allocator.destroy(metadata);
                if (self.aggregation) |aggregation| cache.allocator.destroy(aggregation);
                cache.config.keys.release(cache.config.keys.userdata, &self.key);
                cache.allocator.destroy(self);
            }
            fn load(data: *anyopaque) void {
                const self: *Entry = @ptrCast(@alignCast(data));
                self.outcome = self.cache.loader.run(self.cache.loader.userdata, self.key, &self.cancellation);
                self.ready.store(true, .release);
                self.cache.wake.set(self.cache.io);
            }
        };

        allocator: std.mem.Allocator,
        io: std.Io,
        clock: providers.Clock,
        config: Config(Key, E),
        loader: Loader,
        executor: execution.Executor,
        mutex: std.Io.Mutex = .init,
        closing_mutex: std.Io.Mutex = .init,
        wake: std.Io.Event = .unset,
        entries: ?*Entry = null,
        revision: u128 = 0,
        fault: ?outcomes.Defect = null,
        inflight: usize = 0,
        users: usize = 0,
        closed: bool = false,
        owner_cancelled: std.atomic.Value(bool) = .init(false),
        owner_registration: Cancellation.Registration = .{ .data = null, .forward = ownerCancelled },
        manager: execution.Executor.Handle = undefined,
        manager_joined: bool = false,
        manager_metadata: *Aggregate,
        closing: outcomes.Exit(void, E) = .{ .success = {} },

        /// Construct the selected resource or definition.
        /// Parameters:
        /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `config: Config(Key, E)`: Caller-selected, validated behavior or resource bounds; no hidden default
        ///   is installed.
        /// - `loader: Loader`: Typed loader callback used when the cache misses.
        /// Returns (`outcomes.Exit(*Self, E)`): Carries `*Self` on `.success` or `E` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn create(env: anytype, config: Config(Key, E), loader: Loader) outcomes.Exit(*Self, E) {
            if (config.time_to_live) |ttl| if (ttl.nanoseconds < 0) return .{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.cache_ttl_must_be_non_negative } };
            const cache = env.allocator.create(Self) catch return .{ .die = outcomes.Defect.out_of_memory };
            const manager_metadata = env.allocator.create(Aggregate) catch {
                env.allocator.destroy(cache);
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            manager_metadata.* = .{ .allocator = env.allocator, .storage = .{ .owner = manager_metadata, .release = Aggregate.destroy, .allocator = env.allocator } };
            cache.* = .{ .allocator = env.allocator, .io = env.io, .clock = undefined, .config = config, .loader = loader, .executor = env.async_engine, .manager_metadata = manager_metadata };
            if (comptime @hasField(@typeInfo(@TypeOf(env)).pointer.child, "clock")) cache.clock = env.clock else cache.clock = providers.Clock.fromIo(&cache.io);
            cache.owner_registration.data = cache;
            if (comptime @hasField(@typeInfo(@TypeOf(env)).pointer.child, "cancellation")) env.cancellation.register(&cache.owner_registration);
            const submitted = cache.executor.submit(.independent, .{ .data = cache, .run = manage });
            if (submitted == .rejected) {
                cache.owner_registration.unregister();
                cache.allocator.destroy(manager_metadata);
                cache.allocator.destroy(cache);
                return .{ .die = submitted.rejected };
            }
            cache.manager = submitted.accepted;
            return .{ .success = cache };
        }

        fn ownerCancelled(data: ?*anyopaque) void {
            const self: *Self = @ptrCast(@alignCast(data.?));
            self.owner_cancelled.store(true, .release);
            self.wake.set(self.io);
        }

        fn changed(self: *Self) void {
            // An exhausted generation cannot safely validate stale snapshots.
            if (self.revision == std.math.maxInt(u128)) {
                self.fault = .{ .kind = .invalid_configuration, .message = diagnostics.cache_snapshot_generation_is_exhausted };
                self.closed = true;
            } else self.revision += 1;
            self.wake.set(self.io);
        }

        const Snapshot = struct { entry: *Entry, retained: bool, invalidated: bool, expires: ?std.Io.Timestamp };
        fn snapshot(self: *Self) ?[]Snapshot {
            var count: usize = 0;
            var current = self.entries;
            while (current) |entry| {
                count += 1;
                current = entry.next;
            }
            const entries = self.allocator.alloc(Snapshot, count) catch return null;
            current = self.entries;
            for (entries) |*item| {
                const entry = current.?;
                entry.retain();
                item.* = .{ .entry = entry, .retained = entry.retained, .invalidated = entry.invalidated, .expires = entry.expires };
                current = entry.next;
            }
            return entries;
        }
        fn releaseSnapshot(self: *Self, entries: []Snapshot) void {
            for (entries) |item| item.entry.release();
            self.allocator.free(entries);
        }

        /// Look up a cache entry while observing cancellation.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `key: Key`: Stable identity used for matching, deduplication, or replay.
        /// - `token: ?*const Cancellation`: Cancellation token observed by this operation; its owner must outlive
        ///   pending work.
        /// Returns (`outcomes.Exit(A, E)`): Carries `A` on `.success` or `E` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn getToken(self: *Self, key: Key, token: ?*const Cancellation) outcomes.Exit(A, E) {
            self.mutex.lockUncancelable(self.io);
            self.users += 1;
            self.mutex.unlock(self.io);
            defer {
                self.mutex.lockUncancelable(self.io);
                self.users -= 1;
                self.mutex.unlock(self.io);
            }
            while (true) {
                if (if (token) |cancellation| cancellation.isRequested() else false) return .interrupted;
                self.mutex.lockUncancelable(self.io);
                if (self.closed) {
                    const fault = self.fault orelse outcomes.Defect{ .kind = .invalid_configuration, .message = diagnostics.cache_is_closed };
                    self.mutex.unlock(self.io);
                    return .{ .die = fault };
                }
                const revision = self.revision;
                const entries = self.snapshot() orelse {
                    self.mutex.unlock(self.io);
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
                self.mutex.unlock(self.io);
                // Clock and equality callbacks can reenter. Snapshot references
                // keep entries alive; version validation prevents stale commits.
                const now = self.clock.now();
                var selected: ?*Entry = null;
                for (entries) |item| {
                    if (item.retained and !item.invalidated and (item.expires == null or now.nanoseconds < item.expires.?.nanoseconds) and self.config.keys.equal(self.config.keys.userdata, item.entry.key, key)) {
                        selected = item.entry;
                        break;
                    }
                }
                self.mutex.lockUncancelable(self.io);
                if (self.closed or self.revision != revision) {
                    self.mutex.unlock(self.io);
                    self.releaseSnapshot(entries);
                    continue;
                }
                var expired = false;
                for (entries) |item| if (item.retained and item.expires != null and now.nanoseconds >= item.expires.?.nanoseconds) {
                    item.entry.retained = false;
                    expired = true;
                };
                if (expired) {
                    self.changed();
                    self.mutex.unlock(self.io);
                    self.releaseSnapshot(entries);
                    continue;
                }
                if (selected) |entry| {
                    entry.retain();
                    entry.accessed = now;
                    self.changed();
                    self.mutex.unlock(self.io);
                    self.releaseSnapshot(entries);
                    defer entry.release();
                    return self.observe(entry, token);
                }
                const saturated = if (self.config.maximum_inflight) |limit| self.inflight >= limit else false;
                self.mutex.unlock(self.io);
                self.releaseSnapshot(entries);
                if (saturated) return .{ .die = .{ .kind = .provider_unavailable, .message = diagnostics.cache_in_flight_admission_capacity_is_exhausted } };
                const owned_key = self.config.keys.retain(self.config.keys.userdata, key) catch return .{ .die = outcomes.Defect.out_of_memory };
                const entry = self.allocator.create(Entry) catch {
                    var discarded = owned_key;
                    self.config.keys.release(self.config.keys.userdata, &discarded);
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
                const metadata = self.allocator.create(Aggregate) catch {
                    var discarded = owned_key;
                    self.config.keys.release(self.config.keys.userdata, &discarded);
                    self.allocator.destroy(entry);
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
                const aggregation = self.allocator.create(Aggregate) catch {
                    var discarded = owned_key;
                    self.config.keys.release(self.config.keys.userdata, &discarded);
                    self.allocator.destroy(metadata);
                    self.allocator.destroy(entry);
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
                aggregation.* = .{ .allocator = self.allocator, .storage = .{ .owner = aggregation, .release = Aggregate.destroy, .allocator = self.allocator } };
                metadata.* = .{ .allocator = self.allocator, .storage = .{ .owner = metadata, .release = Aggregate.destroy, .allocator = self.allocator } };
                entry.* = .{ .cache = self, .key = owned_key, .accessed = now, .completion = .{ .io = self.io }, .metadata = metadata, .aggregation = aggregation };
                self.mutex.lockUncancelable(self.io);
                if (self.closed or self.revision != revision) {
                    self.mutex.unlock(self.io);
                    entry.release();
                    continue;
                }
                entry.next = self.entries;
                self.entries = entry;
                self.inflight += 1;
                entry.retain(); // this observer
                self.changed();
                self.mutex.unlock(self.io);
                const submitted = self.executor.submit(.independent, .{ .data = entry, .run = Entry.load, .cancellation = &entry.cancellation });
                if (submitted == .accepted) entry.handle.init(submitted.accepted, &entry.cancellation) else {
                    entry.outcome = .{ .die = submitted.rejected };
                    entry.ready.store(true, .release);
                }
                self.mutex.lockUncancelable(self.io);
                entry.accepted = submitted == .accepted;
                entry.submitted = true;
                self.changed();
                self.mutex.unlock(self.io);
                defer entry.release();
                return self.observe(entry, token);
            }
        }

        fn observe(self: *Self, entry: *Entry, token: ?*const Cancellation) outcomes.Exit(A, E) {
            const observation = entry.completion.waitObserved(token);
            var value = observation.exit;
            self.mutex.lockUncancelable(self.io);
            const cancelled = if (token) |cancellation| cancellation.isRequested() else false;
            const invalidated = self.closed or entry.invalidated;
            if (!cancelled and !observation.observer_interrupted and !observation.clone_failed) {
                entry.diagnostic = false;
                self.changed();
            }
            self.mutex.unlock(self.io);
            if (cancelled or invalidated) {
                if (value == .finalization_failed) {
                    if (value.finalization_failed.primary == .failed) outcomes.discardPayload(&value.finalization_failed.primary.failed);
                    value.finalization_failed.primary = .was_interrupted;
                    return value;
                }
                value.deinit();
                return .interrupted;
            }
            return value;
        }

        fn trim(self: *Self) void {
            const capacity = self.config.capacity orelse return;
            var count: usize = 0;
            var current = self.entries;
            while (current) |entry| {
                if (entry.retained and entry.settled) count += 1;
                current = entry.next;
            }
            while (count > capacity) {
                var oldest: ?*Entry = null;
                current = self.entries;
                while (current) |entry| {
                    if (entry.retained and entry.settled and (oldest == null or entry.accessed.nanoseconds <= oldest.?.accessed.nanoseconds)) oldest = entry;
                    current = entry.next;
                }
                oldest.?.retained = false;
                count -= 1;
            }
        }

        fn manage(data: *anyopaque) void {
            const self: *Self = @ptrCast(@alignCast(data));
            const protection = self.io.swapCancelProtection(.blocked);
            defer _ = self.io.swapCancelProtection(protection);
            while (true) {
                self.mutex.lockUncancelable(self.io);
                var selected: ?*Entry = null;
                var cancel: ?*Entry = null;
                var retired: ?*Entry = null;
                var link = &self.entries;
                while (link.*) |entry| {
                    if ((self.closed or self.owner_cancelled.load(.acquire)) and !entry.cancellation.isRequested()) {
                        entry.retain();
                        cancel = entry;
                        break;
                    }
                    if (entry.submitted and entry.ready.load(.acquire) and !entry.claimed and !entry.settled) {
                        entry.claimed = true;
                        entry.retain();
                        selected = entry;
                        break;
                    }
                    if (entry.settled and !entry.retained and !entry.diagnostic) {
                        link.* = entry.next;
                        retired = entry;
                        self.changed();
                        break;
                    }
                    link = &entry.next;
                }
                const finished = self.closed and self.inflight == 0;
                if (selected == null and cancel == null and retired == null) self.wake.reset();
                self.mutex.unlock(self.io);
                if (cancel) |entry| {
                    entry.cancellation.request();
                    entry.release();
                    continue;
                }
                if (retired) |entry| {
                    entry.release();
                    continue;
                }
                if (selected) |entry| {
                    if (entry.accepted) {
                        const joined = entry.handle.joinProtected(true);
                        if (joined.forwarding) |defect| entry.metadata.?.storage.append(&entry.outcome, .{ .release_defect = defect });
                        if (joined.joining) |defect| entry.metadata.?.storage.append(&entry.outcome, .{ .release_defect = defect });
                    }
                    var keep = switch (entry.outcome) {
                        .success => true,
                        .fail => |err| self.config.cache_failure(err),
                        else => false,
                    };
                    const now = self.clock.now();
                    var expires: ?std.Io.Timestamp = null;
                    blk: {
                        if (keep) if (self.config.time_to_live) |ttl| {
                            const end = std.math.add(i96, now.nanoseconds, ttl.nanoseconds) catch {
                                var failed: outcomes.Exit(A, E) = .{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.cache_expiration_timestamp_overflowed } };
                                if (entry.outcome == .success) entry.outcome.deinit() else entry.metadata.?.storage.appendFailures(&failed, entry.outcome);
                                entry.outcome = failed;
                                keep = false;
                                break :blk;
                            };
                            expires = .{ .nanoseconds = end };
                        };
                    }
                    // Publish only after native ownership has been joined. The
                    // manager settles whichever loader completes first.
                    self.mutex.lockUncancelable(self.io);
                    entry.settled = true;
                    entry.retained = keep and !entry.invalidated and !self.closed;
                    entry.diagnostic = entry.outcome == .finalization_failed;
                    entry.expires = expires;
                    self.inflight -= 1;
                    self.trim();
                    self.changed();
                    self.mutex.unlock(self.io);
                    _ = entry.completion.completeCopy(entry.outcome);
                    entry.release();
                    continue;
                }
                if (finished) return;
                self.wake.waitUncancelable(self.io);
            }
        }

        /// Invalidate the selected cached or observable value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `key: Key`: Stable identity used for matching, deduplication, or replay.
        /// Returns (`outcomes.Exit(void, E)`): Carries no value on `.success` or `E` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn invalidate(self: *Self, key: Key) outcomes.Exit(void, E) {
            self.mutex.lockUncancelable(self.io);
            self.users += 1;
            self.mutex.unlock(self.io);
            defer {
                self.mutex.lockUncancelable(self.io);
                self.users -= 1;
                self.mutex.unlock(self.io);
            }
            while (true) {
                self.mutex.lockUncancelable(self.io);
                const revision = self.revision;
                const entries = self.snapshot() orelse {
                    self.mutex.unlock(self.io);
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
                self.mutex.unlock(self.io);
                const matches = self.allocator.alloc(bool, entries.len) catch {
                    self.releaseSnapshot(entries);
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
                for (entries, matches) |item, *match| match.* = self.config.keys.equal(self.config.keys.userdata, item.entry.key, key);
                self.mutex.lockUncancelable(self.io);
                if (self.revision != revision) {
                    self.mutex.unlock(self.io);
                    self.allocator.free(matches);
                    self.releaseSnapshot(entries);
                    continue;
                }
                for (entries, matches) |item, match| if (match) {
                    item.entry.invalidated = true;
                    item.entry.retained = false;
                };
                self.changed();
                self.mutex.unlock(self.io);
                var result: outcomes.Exit(void, E) = .{ .success = {} };
                const protection = self.io.swapCancelProtection(.blocked);
                defer _ = self.io.swapCancelProtection(protection);
                for (entries, matches) |item, match| if (match) {
                    const entry = item.entry;
                    entry.cancellation.request();
                    _ = entry.completion.waitReadyToken(null);
                    self.mutex.lockUncancelable(self.io);
                    // Entries retain an independently owned raw outcome. Move
                    // its cleanup once; observers use the Deferred's copy.
                    const completed = entry.outcome;
                    entry.outcome = .interrupted;
                    entry.diagnostic = false;
                    if (completed == .finalization_failed) {
                        entry.aggregation.?.storage.appendFailures(&result, completed);
                        if (entry.aggregation.?.storage.retained()) entry.aggregation = null;
                        if (entry.metadata.?.storage.retained()) entry.metadata = null;
                    } else if (completed == .fail) {
                        var discarded = completed;
                        discarded.deinit();
                    }
                    if (completed == .success) {
                        var discarded = completed;
                        discarded.deinit();
                    }
                    self.changed();
                    self.mutex.unlock(self.io);
                };
                self.allocator.free(matches);
                self.releaseSnapshot(entries);
                return result;
            }
        }

        const Aggregate = struct {
            allocator: std.mem.Allocator,
            storage: outcomes.CleanupStorage(E),
            fn destroy(data: *anyopaque) void {
                const self: *@This() = @ptrCast(@alignCast(data));
                self.allocator.destroy(self);
            }
        };

        /// Stop the capability and release its retained resources.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`outcomes.Exit(void, E)`): Carries no value on `.success` or `E` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn shutdown(self: *Self) outcomes.Exit(void, E) {
            self.closing_mutex.lockUncancelable(self.io);
            defer self.closing_mutex.unlock(self.io);
            if (self.manager_joined) return self.closing.clone();
            self.mutex.lockUncancelable(self.io);
            self.closed = true;
            self.changed();
            self.mutex.unlock(self.io);
            const joined = self.manager.joinProtected(true);
            self.manager.deinit();
            self.owner_registration.unregister();
            self.manager_joined = true;
            if (self.fault) |fault| self.closing = .{ .die = fault };
            if (joined.defect) |defect| self.manager_metadata.storage.append(&self.closing, .{ .release_defect = defect });
            var current = self.entries;
            while (current) |entry| {
                if (entry.diagnostic) {
                    entry.aggregation.?.storage.appendFailures(&self.closing, entry.outcome);
                    if (entry.aggregation.?.storage.retained()) entry.aggregation = null;
                    if (entry.metadata.?.storage.retained()) entry.metadata = null;
                    entry.outcome = .interrupted;
                    entry.diagnostic = false;
                }
                current = entry.next;
            }
            return self.closing.clone();
        }

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            std.debug.assert(self.manager_joined and self.users == 0);
            var current = self.entries;
            self.entries = null;
            while (current) |entry| {
                current = entry.next;
                entry.release();
            }
            const manager_retained = self.manager_metadata.storage.retained();
            self.closing.deinit();
            if (!manager_retained) self.allocator.destroy(self.manager_metadata);
            self.loader.deinit(self.loader.userdata);
            self.allocator.destroy(self);
        }
    };
}
