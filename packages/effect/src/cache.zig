//! Scoped cache with caller-selected capacity, expiry, and in-flight sharing.
const diagnostics = @import("internal/diagnostics.zig");
const std = @import("std");
const outcomes = @import("outcome.zig");
const providers = @import("services.zig");
const execution = @import("executor.zig");
const primitives = @import("primitives.zig");
const Cancellation = @import("internal/runtime.zig").Cancellation;

/// Lookup selection, fixed before waiting for a generation.
pub const CacheObservation = enum {
    /// Selected a still-retained settled entry.
    retained,
    /// Selected a generation already being loaded by another observer.
    shared,
    /// Admitted the generation whose loader supplies this value.
    loaded,
};
/// Observed A plus selection provenance fixed before waiting.
/// When A is marked owning, deinit/clone release/copy its ownership independently.
pub fn CacheLookup(comptime A: type) type {
    return struct {
        pub const effect_owned_payload = outcomes.ownsPayload(A);
        /// Independent observation of the selected cache value.
        value: A,
        /// Provenance chosen before waiting; settlement does not relabel it.
        observation: CacheObservation,
        /// Discard a marked owning observed value once.
        pub fn deinit(self: *@This()) void {
            outcomes.discardPayload(&self.value);
            self.* = undefined;
        }
        /// Clone the observed value without changing its generation-selection provenance.
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            return .{ .value = try outcomes.clonePayload(self.value), .observation = self.observation };
        }
    };
}

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
        capacity_error: ?*const fn () E = null,
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
        const dependency_aware = switch (@typeInfo(A)) {
            .@"struct", .@"union" => @hasDecl(A, "cache_dependencies"),
            else => false,
        };
        const DependencySnapshot = if (dependency_aware) A else void;
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
            dependency_snapshot: ?DependencySnapshot = null,
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
                if (comptime dependency_aware) if (self.dependency_snapshot) |*value| value.deinit();
                if (!retained) if (self.metadata) |metadata| cache.allocator.destroy(metadata);
                if (self.aggregation) |aggregation| cache.allocator.destroy(aggregation);
                cache.config.keys.release(cache.config.keys.userdata, &self.key);
                cache.allocator.destroy(self);
            }
            fn load(data: *anyopaque) void {
                const self: *Entry = @ptrCast(@alignCast(data));
                self.outcome = self.cache.loader.run(self.cache.loader.userdata, self.key, &self.cancellation);
                if (comptime dependency_aware) {
                    if (self.outcome == .success) {
                        self.dependency_snapshot = self.outcome.success.clone() catch {
                            self.outcome.deinit();
                            self.outcome = .{ .die = outcomes.Defect.out_of_memory };
                            self.ready.store(true, .release);
                            self.cache.wake.set(self.cache.io);
                            return;
                        };
                    }
                }
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
        users_idle: std.Io.Condition = .init,
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
        /// Returns (`outcomes.Exit(*Self, E)`): Carries `*Self` on `.success` or `E` on `.fail`; `.die`, `.interrupted`, and `.finalization_failed` preserve other terminal causes.
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

        fn beginUse(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            self.users += 1;
            self.mutex.unlock(self.io);
        }
        fn endUse(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            self.users -= 1;
            if (self.users == 0) self.users_idle.broadcast(self.io);
            self.mutex.unlock(self.io);
        }

        const Snapshot = struct { entry: *Entry, retained: bool, invalidated: bool, expires: ?std.Io.Timestamp, settled: bool };
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
                item.* = .{ .entry = entry, .retained = entry.retained, .invalidated = entry.invalidated, .expires = entry.expires, .settled = entry.settled };
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
        /// Returns (`outcomes.Exit(A, E)`): Carries `A` on `.success` or `E` on `.fail`; `.die`, `.interrupted`, and `.finalization_failed` preserve other terminal causes.
        pub fn getToken(self: *Self, key: Key, token: ?*const Cancellation) outcomes.Exit(A, E) {
            const selected = self.getObservedToken(key, token);
            return switch (selected) {
                .success => |lookup| .{ .success = lookup.value },
                else => outcomes.castFailure(A, selected),
            };
        }
        /// Select a retained/shared/new generation before waiting and return an independent value observation.
        /// Cancellation removes this observer; the scoped shared load and other observers keep their own lifetimes.
        pub fn getObservedToken(self: *Self, key: Key, token: ?*const Cancellation) outcomes.Exit(CacheLookup(A), E) {
            self.beginUse();
            defer self.endUse();
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
                    const observation: CacheObservation = if (entry.settled) .retained else .shared;
                    entry.retain();
                    entry.accessed = now;
                    self.changed();
                    self.mutex.unlock(self.io);
                    self.releaseSnapshot(entries);
                    defer entry.release();
                    return self.observeLookup(entry, token, observation);
                }
                const saturated = if (self.config.maximum_inflight) |limit| self.inflight >= limit else false;
                self.mutex.unlock(self.io);
                self.releaseSnapshot(entries);
                if (saturated) {
                    if (self.config.capacity_error) |make_error| return .{ .fail = make_error() };
                    return .{ .die = .{ .kind = .provider_unavailable, .message = diagnostics.cache_in_flight_admission_capacity_is_exhausted } };
                }
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
                entry.* = .{ .cache = self, .key = owned_key, .accessed = .{ .nanoseconds = 0 }, .completion = .{ .io = self.io }, .metadata = metadata, .aggregation = aggregation };
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
                return self.observeLookup(entry, token, .loaded);
            }
        }

        fn observeLookup(self: *Self, entry: *Entry, token: ?*const Cancellation, observation: CacheObservation) outcomes.Exit(CacheLookup(A), E) {
            const result = self.observe(entry, token);
            return switch (result) {
                .success => |value| .{ .success = .{ .value = value, .observation = observation } },
                else => outcomes.castFailure(CacheLookup(A), result),
            };
        }

        /// Inspect settled successful entries only; never starts or waits for a load.
        pub fn tryGet(self: *Self, key: Key) outcomes.Exit(?A, E) {
            self.beginUse();
            defer self.endUse();
            while (true) {
                self.mutex.lockUncancelable(self.io);
                if (self.closed) {
                    self.mutex.unlock(self.io);
                    return .{ .success = null };
                }
                const revision = self.revision;
                const entries = self.snapshot() orelse {
                    self.mutex.unlock(self.io);
                    return .{ .die = outcomes.Defect.out_of_memory };
                };
                self.mutex.unlock(self.io);
                const now = self.clock.now();
                var selected: ?*Entry = null;
                for (entries) |item| {
                    if (item.settled and item.retained and !item.invalidated and
                        (item.expires == null or now.nanoseconds < item.expires.?.nanoseconds) and
                        self.config.keys.equal(self.config.keys.userdata, item.entry.key, key))
                    {
                        selected = item.entry;
                        break;
                    }
                }
                self.mutex.lockUncancelable(self.io);
                if (revision != self.revision) {
                    self.mutex.unlock(self.io);
                    self.releaseSnapshot(entries);
                    continue;
                }
                if (selected) |entry| entry.retain();
                self.mutex.unlock(self.io);
                self.releaseSnapshot(entries);
                const entry = selected orelse return .{ .success = null };
                defer entry.release();
                // Deferred.tryGet clones outside the cache lock and cannot suspend.
                const value = entry.completion.poll();
                if (value) |result| {
                    return switch (result) {
                        .success => |success| .{ .success = success },
                        .die => |defect| .{ .die = defect },
                        else => blk: {
                            var discarded = result;
                            discarded.deinit();
                            break :blk .{ .success = null };
                        },
                    };
                }
                return .{ .success = null };
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
        /// Returns (`outcomes.Exit(void, E)`): Carries no value on `.success` or `E` on `.fail`; `.die`, `.interrupted`, and `.finalization_failed` preserve other terminal causes.
        pub fn invalidate(self: *Self, key: Key) outcomes.Exit(void, E) {
            return self.invalidateMatching(Key, key, struct {
                fn matches(cache: *Self, candidate: Key, _: ?*const DependencySnapshot, target: Key) bool {
                    return cache.config.keys.equal(cache.config.keys.userdata, candidate, target);
                }
            }.matches, null);
        }
        /// Exclude every selected generation, release settled entries and cancel/join excluded loaders outside the mutex.
        pub fn clear(self: *Self) outcomes.Exit(void, E) {
            return self.invalidateMatching(void, {}, struct {
                fn matches(_: *Self, _: Key, _: ?*const DependencySnapshot, _: void) bool {
                    return true;
                }
            }.matches, null);
        }
        /// Invalidate matching settled dependency owners and conservatively exclude unfinished generations with unknown dependencies.
        pub fn invalidateDependency(self: *Self, dependency: A.Dependency) outcomes.Exit(void, E) {
            if (!dependency_aware) @compileError("dependency invalidation requires a dependency cache");
            return self.invalidateMatching(A.Dependency, dependency, struct {
                fn matches(_: *Self, _: Key, metadata: ?*const DependencySnapshot, target: A.Dependency) bool {
                    // Unknown dependencies include active generations and retained failures.
                    const value = metadata orelse return true;
                    return value.dependsOn(target);
                }
            }.matches, null);
        }
        fn invalidateMatching(self: *Self, comptime Input: type, input: Input, comptime matchesEntry: fn (*Self, Key, ?*const DependencySnapshot, Input) bool, replacement: ?*Entry) outcomes.Exit(void, E) {
            self.beginUse();
            defer self.endUse();
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
                for (entries, matches) |item, *match| {
                    const metadata: ?*const DependencySnapshot = if (comptime dependency_aware)
                        (if (item.settled and item.entry.dependency_snapshot != null) &item.entry.dependency_snapshot.? else null)
                    else
                        null;
                    match.* = matchesEntry(self, item.entry.key, metadata, input);
                }
                // Time is sampled after user equality and all replacement copies.
                // Retry the sample when a competing mutation invalidates the snapshot.
                const settled_at: ?std.Io.Timestamp = if (replacement != null) self.clock.now() else null;
                const expires: ?std.Io.Timestamp = if (settled_at != null and self.config.time_to_live != null) blk: {
                    const end = std.math.add(i96, settled_at.?.nanoseconds, self.config.time_to_live.?.nanoseconds) catch {
                        self.allocator.free(matches);
                        self.releaseSnapshot(entries);
                        return .{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.cache_expiration_timestamp_overflowed } };
                    };
                    break :blk .{ .nanoseconds = end };
                } else null;
                self.mutex.lockUncancelable(self.io);
                if (self.revision != revision) {
                    self.mutex.unlock(self.io);
                    self.allocator.free(matches);
                    self.releaseSnapshot(entries);
                    continue;
                }
                if (replacement != null and self.closed) {
                    self.mutex.unlock(self.io);
                    self.allocator.free(matches);
                    self.releaseSnapshot(entries);
                    return .{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.cache_is_closed } };
                }
                for (entries, matches) |item, match| if (match) {
                    item.entry.invalidated = true;
                    item.entry.retained = false;
                };
                if (replacement) |entry| {
                    if (self.closed) {
                        self.mutex.unlock(self.io);
                        self.allocator.free(matches);
                        self.releaseSnapshot(entries);
                        return .{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.cache_is_closed } };
                    }
                    entry.accessed = settled_at.?;
                    entry.expires = expires;
                    entry.next = self.entries;
                    self.entries = entry;
                    entry.retain(); // cache list ownership; caller keeps its construction reference
                    self.trim();
                }
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
                    }
                    self.changed();
                    self.mutex.unlock(self.io);
                    if (completed == .success or completed == .fail) {
                        var discarded = completed;
                        discarded.deinit();
                    }
                };
                self.allocator.free(matches);
                self.releaseSnapshot(entries);
                return result;
            }
        }

        /// Copy replacement data before atomically excluding earlier equal-key generations.
        pub fn set(self: *Self, key: Key, value: *const A) outcomes.Exit(void, E) {
            self.beginUse();
            defer self.endUse();
            const owned_key = self.config.keys.retain(self.config.keys.userdata, key) catch return .{ .die = outcomes.Defect.out_of_memory };
            const entry = self.allocator.create(Entry) catch {
                var discarded = owned_key;
                self.config.keys.release(self.config.keys.userdata, &discarded);
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            entry.* = .{ .cache = self, .key = owned_key, .accessed = .{ .nanoseconds = 0 }, .completion = .{ .io = self.io }, .metadata = null, .aggregation = null };
            defer entry.release();
            const copied = outcomes.clonePayload(value.*) catch return .{ .die = outcomes.Defect.out_of_memory };
            entry.outcome = .{ .success = copied };
            entry.settled = true;
            if (comptime dependency_aware) entry.dependency_snapshot = copied.clone() catch return .{ .die = outcomes.Defect.out_of_memory };
            if (!entry.completion.completeCopy(entry.outcome) or entry.completion.copy_failed) return .{ .die = outcomes.Defect.out_of_memory };
            return self.invalidateMatching(Key, key, struct {
                fn matches(cache: *Self, candidate: Key, _: ?*const DependencySnapshot, target: Key) bool {
                    return cache.config.keys.equal(cache.config.keys.userdata, candidate, target);
                }
            }.matches, entry);
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
        /// Returns (`outcomes.Exit(void, E)`): Carries no value on `.success` or `E` on `.fail`; `.die`, `.interrupted`, and `.finalization_failed` preserve other terminal causes.
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
            std.debug.assert(self.manager_joined);
            // Shutdown joins provider generations. Borrowers can still be
            // returning from Deferred observations or user policy callbacks;
            // keep entries, the loader context and scope inputs alive until
            // those already active calls have released their cache access.
            self.mutex.lockUncancelable(self.io);
            while (self.users != 0) self.users_idle.waitUncancelable(self.io, &self.mutex);
            self.mutex.unlock(self.io);
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
