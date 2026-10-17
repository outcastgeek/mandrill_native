//! Scoped resource pooling with bounded capacity, lease ownership, and shutdown.
const diagnostics = @import("internal/diagnostics.zig");
const std = @import("std");
const outcomes = @import("outcome.zig");
const providers = @import("services.zig");
const execution = @import("executor.zig");
const primitives = @import("primitives.zig");
const Cancellation = @import("internal/runtime.zig").Cancellation;

/// Typed config record; release owned state with `deinit`.
pub const Config = struct { capacity: ?usize, idle_time_to_live: ?std.Io.Duration };

/// The pool owns resources. Leases borrow them exclusively until their first
/// disposition completes. Scope finalization also destroys lease bookkeeping.
///
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): a bounded resource pool with exclusive leases, caller-selected factory, and typed acquisition failures.
pub fn Pool(comptime A: type, comptime E: type) type {
    return struct {
        const Self = @This();
        const UnitExit = outcomes.Exit(void, E);
        /// `Factory` is a callback interface with `userdata: *anyopaque`, `create: *const fn
        /// (*anyopaque, *const Cancellation) outcomes.Exit(A, E)`, `validate: *const fn (*anyopaque, A,
        /// *const Cancellation) outcomes.Exit(bool, E)`, `dispose: *const fn (*anyopaque, A) UnitExit`
        /// and further declared fields.
        pub const Factory = struct {
            userdata: *anyopaque,
            create: *const fn (*anyopaque, *const Cancellation) outcomes.Exit(A, E),
            validate: *const fn (*anyopaque, A, *const Cancellation) outcomes.Exit(bool, E),
            dispose: *const fn (*anyopaque, A) UnitExit,
            deinit: *const fn (*anyopaque) void,
        };
        const Metadata = struct {
            allocator: std.mem.Allocator,
            storage: outcomes.CleanupStorage(E),
            fn destroy(data: *anyopaque) void {
                const self: *@This() = @ptrCast(@alignCast(data));
                self.allocator.destroy(self);
            }
            fn make(allocator: std.mem.Allocator) ?*@This() {
                const self = allocator.create(@This()) catch return null;
                self.* = .{ .allocator = allocator, .storage = .{ .allocator = allocator, .owner = self, .release = destroy } };
                return self;
            }
            fn finish(self: *@This()) void {
                if (!self.storage.retained()) self.allocator.destroy(self);
            }
        };
        const Resource = struct { value: A = undefined, next: ?*Resource = null, returned: std.Io.Timestamp = .{ .nanoseconds = 0 }, order: u128 = 0, pins: usize = 0, metadata: *Metadata };
        const Waiter = struct {
            next: ?*Waiter = null,
            wake: std.Io.Event = .unset,
            io: std.Io,
            registration: Cancellation.Registration = .{ .data = null, .forward = cancelled },
            fn cancelled(data: ?*anyopaque) void {
                const self: *@This() = @ptrCast(@alignCast(data.?));
                self.wake.set(self.io);
            }
        };
        /// Exclusive, scope-bound loan of one pooled resource. Release or discard it before pool shutdown; never use its resource afterward.
        pub const Lease = struct {
            pool: *Self,
            allocator: std.mem.Allocator,
            resource: *Resource,
            mutex: std.Io.Mutex = .init,
            completion: primitives.Deferred(void, E),
            started: bool = false,
            metadata: ?*Metadata,

            /// Access the value held by this wrapper or lease.
            /// Parameters:
            /// - `self: *const Lease`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
            /// Returns (`A`): The typed stored or computed value; ownership follows the enclosing type contract.
            pub fn value(self: *const Lease) A {
                return self.resource.value;
            }
            /// Finish the current operation and release its active state.
            /// Parameters:
            /// - `self: *Lease`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
            /// - `should_retire: bool`: Whether this lease must be retired instead of returned to the pool.
            /// Returns (`UnitExit`): A terminal Exit with no success payload and a typed failure channel.
            pub fn end(self: *Lease, should_retire: bool) UnitExit {
                const io = self.completion.io;
                const protection = io.swapCancelProtection(.blocked);
                defer _ = io.swapCancelProtection(protection);
                self.mutex.lockUncancelable(io);
                const start = !self.started;
                self.started = true;
                self.mutex.unlock(io);
                if (!start) return self.completion.waitToken(null);
                // An active lease keeps shutdown pending. A completed lease no
                // longer needs the pool and can outlive its resource owner.
                const pool = self.pool;
                const now = pool.clock.now();
                pool.mutex.lockUncancelable(pool.io);
                const dispose = should_retire or pool.closed;
                if (!dispose) {
                    self.resource.returned = now;
                    pool.insertIdle(self.resource);
                }
                pool.mutex.unlock(pool.io);
                var result: UnitExit = .{ .success = {} };
                if (dispose) result = pool.retire(self.resource, false);
                pool.mutex.lockUncancelable(pool.io);
                pool.leases -= 1;
                if (pool.closed) {
                    pool.recordCleanup(&result, self.metadata.?);
                    if (self.metadata.?.storage.retained()) self.metadata = null;
                }
                pool.notify();
                pool.mutex.unlock(pool.io);
                _ = self.completion.complete(result);
                return self.completion.waitToken(null);
            }
            /// Release resources owned by this value.
            /// Parameters:
            /// - `self: *Lease`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
            /// Returns (`void`): no value.
            pub fn deinit(self: *Lease) void {
                std.debug.assert(self.started);
                self.completion.deinit();
                if (self.metadata) |metadata| metadata.finish();
                self.allocator.destroy(self);
            }
        };
        const Population = struct {
            pool: *Self,
            next: ?*Population = null,
            resource: *Resource,
            metadata: *Metadata,
            native_metadata: *Metadata,
            diagnostic_metadata: *Metadata,
            references: std.atomic.Value(usize) = .init(1),
            cancellation: Cancellation = .{},
            registration: Cancellation.Registration = .{ .data = null, .forward = cancel },
            handle: execution.Registered = undefined,
            accepted: bool = false,
            submitted: bool = false,
            ready: std.atomic.Value(bool) = .init(false),
            settled: bool = false,
            observed: bool = false,
            outcome: UnitExit = undefined,
            created: outcomes.Exit(A, E) = undefined,
            fn retain(self: *@This()) void {
                _ = self.references.fetchAdd(1, .monotonic);
            }
            fn release(self: *@This()) void {
                if (self.references.fetchSub(1, .acq_rel) != 1) return;
                const allocator = self.pool.allocator;
                const retained = self.metadata.storage.retained();
                const native_retained = self.native_metadata.storage.retained();
                if (self.settled) self.outcome.deinit() else {
                    std.debug.assert(!self.submitted);
                    self.resource.metadata.finish();
                    allocator.destroy(self.resource);
                }
                if (!retained) allocator.destroy(self.metadata);
                if (!native_retained) allocator.destroy(self.native_metadata);
                self.diagnostic_metadata.finish();
                allocator.destroy(self);
            }
            fn cancel(data: ?*anyopaque) void {
                const self: *@This() = @ptrCast(@alignCast(data.?));
                self.cancellation.request();
            }
            fn run(data: *anyopaque) void {
                const self: *@This() = @ptrCast(@alignCast(data));
                self.created = self.pool.factory.create(self.pool.factory.userdata, &self.cancellation);
                self.ready.store(true, .release);
                self.pool.wake.set(self.pool.io);
            }
        };

        // A factory committed to a borrower also contributes to population.
        // Population observers pin this record until that reservation settles.
        const BorrowerCreation = struct {
            allocator: std.mem.Allocator,
            next: ?*@This() = null,
            references: std.atomic.Value(usize) = .init(1),
            settled: bool = false,
            observed: bool = false,
            outcome: UnitExit = .{ .success = {} },
            storage: outcomes.CleanupStorage(E),
            fn retain(self: *@This()) void {
                _ = self.references.fetchAdd(1, .monotonic);
            }
            fn release(self: *@This()) void {
                if (self.references.fetchSub(1, .acq_rel) == 1) {
                    self.outcome.deinit();
                    self.allocator.destroy(self);
                }
            }
            fn releaseStorage(data: *anyopaque) void {
                const self: *@This() = @ptrCast(@alignCast(data));
                self.release();
            }
        };

        allocator: std.mem.Allocator,
        io: std.Io,
        clock: providers.Clock,
        config: Config,
        factory: Factory,
        executor: execution.Executor,
        mutex: std.Io.Mutex = .init,
        close_mutex: std.Io.Mutex = .init,
        changed: std.Io.Condition = .init,
        wake: std.Io.Event = .unset,
        idle: ?*Resource = null,
        first_waiter: ?*Waiter = null,
        last_waiter: ?*Waiter = null,
        observers: ?*Waiter = null,
        borrower_creations: ?*BorrowerCreation = null,
        allocated: usize = 0,
        live: usize = 0,
        operations: usize = 0,
        leases: usize = 0,
        sequence: u128 = 0,
        closed: bool = false,
        cancellation: Cancellation = .{},
        owner_registration: Cancellation.Registration = .{ .data = null, .forward = cancelOwner },
        populations: ?*Population = null,
        manager: execution.Executor.Handle = undefined,
        manager_joined: bool = false,
        manager_metadata: *Metadata,
        closing: UnitExit = .{ .success = {} },

        /// Construct the selected resource or definition.
        /// Parameters:
        /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `config: Config`: Caller-selected, validated behavior or resource bounds; no hidden default
        ///   is installed.
        /// - `factory: Factory`: Typed callback; its signature is checked at comptime where static.
        /// Returns (`outcomes.Exit(*Self, E)`): Carries `*Self` on `.success` or `E` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn create(env: anytype, config: Config, factory: Factory) outcomes.Exit(*Self, E) {
            if (config.capacity) |capacity| if (capacity == 0) return invalid(diagnostics.pool_capacity_must_be_positive);
            if (config.idle_time_to_live) |ttl| if (ttl.nanoseconds < 0) return invalid(diagnostics.pool_idle_lifetime_must_be_non_negative);
            const pool = env.allocator.create(Self) catch return .{ .die = outcomes.Defect.out_of_memory };
            const manager_metadata = Metadata.make(env.allocator) orelse {
                env.allocator.destroy(pool);
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            pool.* = .{ .allocator = env.allocator, .io = env.io, .clock = undefined, .config = config, .factory = factory, .executor = env.async_engine, .manager_metadata = manager_metadata };
            if (comptime @hasField(@typeInfo(@TypeOf(env)).pointer.child, "clock")) pool.clock = env.clock else pool.clock = providers.Clock.fromIo(&pool.io);
            pool.owner_registration.data = pool;
            if (comptime @hasField(@typeInfo(@TypeOf(env)).pointer.child, "cancellation")) env.cancellation.register(&pool.owner_registration);
            const admitted = pool.executor.submit(.independent, .{ .data = pool, .run = manage });
            if (admitted == .rejected) {
                pool.owner_registration.unregister();
                manager_metadata.finish();
                pool.allocator.destroy(pool);
                return .{ .die = admitted.rejected };
            }
            pool.manager = admitted.accepted;
            return .{ .success = pool };
        }
        fn invalid(message: []const u8) outcomes.Exit(*Self, E) {
            return .{ .die = .{ .kind = .invalid_configuration, .message = message } };
        }
        fn closedExit(comptime T: type) outcomes.Exit(T, E) {
            return .{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.pool_is_closed } };
        }
        fn cancelOwner(data: ?*anyopaque) void {
            const pool: *Self = @ptrCast(@alignCast(data.?));
            pool.cancellation.request();
            pool.wake.set(pool.io);
        }
        fn notify(self: *Self) void {
            var waiter = self.first_waiter;
            while (waiter) |item| {
                item.wake.set(self.io);
                waiter = item.next;
            }
            waiter = self.observers;
            while (waiter) |item| {
                item.wake.set(self.io);
                waiter = item.next;
            }
            self.changed.broadcast(self.io);
            self.wake.set(self.io);
        }
        fn insertIdle(self: *Self, resource: *Resource) void {
            var link = &self.idle;
            while (link.*) |item| {
                if (item.order > resource.order) break;
                link = &item.next;
            }
            resource.next = link.*;
            link.* = resource;
        }
        fn removeWaiter(self: *Self, waiter: *Waiter) void {
            var previous: ?*Waiter = null;
            var link = &self.first_waiter;
            while (link.*) |item| {
                if (item == waiter) {
                    link.* = item.next;
                    if (self.last_waiter == item) self.last_waiter = previous;
                    self.notify();
                    return;
                }
                previous = item;
                link = &item.next;
            }
        }
        fn recordCleanup(self: *Self, value: *UnitExit, metadata: *Metadata) void {
            if (value.* == .finalization_failed) {
                const copied = value.clone();
                metadata.storage.appendFailures(&self.closing, value.*);
                value.* = copied;
            } else if (value.* == .fail or value.* == .die or value.* == .interrupted) {
                const copied = value.clone();
                metadata.storage.appendExit(&self.closing, value.*);
                value.* = copied;
            }
        }
        fn retire(self: *Self, resource: *Resource, keep_metadata: bool) UnitExit {
            self.mutex.lockUncancelable(self.io);
            while (resource.pins != 0) self.changed.waitUncancelable(self.io, &self.mutex);
            self.live -= 1;
            self.mutex.unlock(self.io);
            const protection = self.io.swapCancelProtection(.blocked);
            const result = self.factory.dispose(self.factory.userdata, resource.value);
            _ = self.io.swapCancelProtection(protection);
            if (!keep_metadata) resource.metadata.finish();
            self.allocator.destroy(resource);
            self.mutex.lockUncancelable(self.io);
            self.allocated -= 1;
            self.notify();
            self.mutex.unlock(self.io);
            return result;
        }

        /// Acquire a pool value while observing cancellation.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `parent: ?*const Cancellation`: Cancellation token observed by this operation; its owner must outlive
        ///   pending work.
        /// Returns (`outcomes.Exit(*Lease, E)`): Carries `*Lease` on `.success` or `E` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn acquireToken(self: *Self, parent: ?*const Cancellation) outcomes.Exit(*Lease, E) {
            const lease = self.allocator.create(Lease) catch return .{ .die = outcomes.Defect.out_of_memory };
            const metadata = Metadata.make(self.allocator) orelse {
                self.allocator.destroy(lease);
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            const reserved_resource = self.allocator.create(Resource) catch {
                metadata.finish();
                self.allocator.destroy(lease);
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            const resource_metadata = Metadata.make(self.allocator) orelse {
                metadata.finish();
                self.allocator.destroy(reserved_resource);
                self.allocator.destroy(lease);
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            reserved_resource.* = .{ .metadata = resource_metadata };
            var spare: ?*Resource = reserved_resource;
            defer if (spare) |resource| {
                resource.metadata.finish();
                self.allocator.destroy(resource);
            };
            var token: Cancellation = .{};
            const Link = struct {
                fn cancel(data: ?*anyopaque) void {
                    const cancellation: *Cancellation = @ptrCast(@alignCast(data.?));
                    cancellation.request();
                }
            };
            var owner_link: Cancellation.Registration = .{ .data = &token, .forward = Link.cancel };
            var parent_link: Cancellation.Registration = .{ .data = &token, .forward = Link.cancel };
            self.cancellation.register(&owner_link);
            if (parent) |origin| origin.register(&parent_link);
            defer {
                owner_link.unregister();
                parent_link.unregister();
            }
            var waiter: Waiter = .{ .io = self.io };
            waiter.registration.data = &waiter;
            token.register(&waiter.registration);
            defer waiter.registration.unregister();
            self.mutex.lockUncancelable(self.io);
            if (self.closed) {
                self.mutex.unlock(self.io);
                metadata.finish();
                self.allocator.destroy(lease);
                return closedExit(*Lease);
            }
            self.operations += 1;
            if (self.last_waiter) |last| last.next = &waiter else self.first_waiter = &waiter;
            self.last_waiter = &waiter;
            self.mutex.unlock(self.io);
            defer {
                self.mutex.lockUncancelable(self.io);
                self.removeWaiter(&waiter);
                self.operations -= 1;
                self.notify();
                self.mutex.unlock(self.io);
            }
            while (true) {
                self.mutex.lockUncancelable(self.io);
                if (token.isRequested() or self.closed) {
                    const closed = self.closed;
                    self.mutex.unlock(self.io);
                    metadata.finish();
                    self.allocator.destroy(lease);
                    return if (closed) closedExit(*Lease) else .interrupted;
                }
                var resource: ?*Resource = null;
                var create_new = false;
                var creation: ?*BorrowerCreation = null;
                if (self.first_waiter == &waiter) {
                    if (self.idle) |idle| {
                        self.idle = idle.next;
                        resource = idle;
                    } else if (self.config.capacity == null or self.allocated < self.config.capacity.?) {
                        const reservation = self.allocator.create(BorrowerCreation) catch {
                            self.mutex.unlock(self.io);
                            metadata.finish();
                            self.allocator.destroy(lease);
                            return .{ .die = outcomes.Defect.out_of_memory };
                        };
                        reservation.* = .{ .allocator = self.allocator, .next = self.borrower_creations, .storage = .{ .allocator = self.allocator, .owner = reservation, .release = BorrowerCreation.releaseStorage } };
                        reservation.retain(); // the factory's local ownership
                        self.borrower_creations = reservation;
                        creation = reservation;
                        self.allocated += 1;
                        create_new = true;
                        self.notify();
                    }
                }
                if (resource == null and !create_new) {
                    waiter.wake.reset();
                    self.mutex.unlock(self.io);
                    waiter.wake.wait(self.io) catch token.request();
                    continue;
                }
                self.mutex.unlock(self.io);
                defer if (creation) |reservation| reservation.release();
                if (create_new) {
                    const created = self.factory.create(self.factory.userdata, &token);
                    self.mutex.lockUncancelable(self.io);
                    creation.?.outcome = if (created == .success) .{ .success = {} } else outcomes.castFailure(void, created);
                    creation.?.settled = true;
                    self.notify();
                    self.mutex.unlock(self.io);
                    if (created != .success) {
                        const observed = creation.?.outcome.observe();
                        self.mutex.lockUncancelable(self.io);
                        creation.?.observed = observed.copied;
                        self.allocated -= 1;
                        self.notify();
                        self.mutex.unlock(self.io);
                        metadata.finish();
                        self.allocator.destroy(lease);
                        return outcomes.castFailure(*Lease, observed.exit);
                    }
                    self.mutex.lockUncancelable(self.io);
                    creation.?.observed = true;
                    self.mutex.unlock(self.io);
                    resource = spare.?;
                    spare = null;
                    self.mutex.lockUncancelable(self.io);
                    if (self.sequence == std.math.maxInt(u128)) {
                        self.mutex.unlock(self.io);
                        resource.?.* = .{ .value = created.success, .metadata = resource_metadata };
                        self.mutex.lockUncancelable(self.io);
                        self.live += 1;
                        self.mutex.unlock(self.io);
                        var failed: outcomes.Exit(*Lease, E) = .{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.pool_resource_ordering_is_exhausted } };
                        metadata.storage.appendExit(&failed, self.retire(resource.?, false));
                        metadata.finish();
                        self.allocator.destroy(lease);
                        return failed;
                    }
                    self.sequence += 1;
                    resource.?.* = .{ .value = created.success, .order = self.sequence, .metadata = resource_metadata };
                    self.live += 1;
                    self.mutex.unlock(self.io);
                } else {
                    const now = self.clock.now();
                    const elapsed = std.math.sub(i96, now.nanoseconds, resource.?.returned.nanoseconds) catch {
                        var failure: outcomes.Exit(*Lease, E) = .{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.pool_idle_duration_overflowed } };
                        metadata.storage.appendExit(&failure, self.retire(resource.?, false));
                        metadata.finish();
                        self.allocator.destroy(lease);
                        return failure;
                    };
                    const expired = if (self.config.idle_time_to_live) |ttl| elapsed >= ttl.nanoseconds else false;
                    const validation: outcomes.Exit(bool, E) = if (expired) .{ .success = false } else if (token.isRequested()) .interrupted else self.factory.validate(self.factory.userdata, resource.?.value, &token);
                    if (validation != .success or !validation.success) {
                        const cleanup = self.retire(resource.?, false);
                        if (validation == .success and cleanup == .success) continue;
                        var failure: outcomes.Exit(*Lease, E) = if (validation == .success) outcomes.castFailure(*Lease, cleanup) else outcomes.castFailure(*Lease, validation);
                        if (validation != .success) metadata.storage.appendExit(&failure, cleanup);
                        metadata.finish();
                        self.allocator.destroy(lease);
                        return failure;
                    }
                }
                self.mutex.lockUncancelable(self.io);
                self.leases += 1;
                self.removeWaiter(&waiter);
                self.mutex.unlock(self.io);
                lease.* = .{ .pool = self, .allocator = self.allocator, .resource = resource.?, .completion = .{ .io = self.io }, .metadata = metadata };
                // The caller's scope adapter installs the lease before it checks
                // late cancellation. This raw success must not discard ownership.
                return .{ .success = lease };
            }
        }

        /// Set the maximum number of pool resources.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `capacity: usize`: new maximum number of pooled resources.
        /// Returns (`UnitExit`): A terminal Exit with no success payload and a typed failure channel.
        pub fn setCapacity(self: *Self, capacity: usize) UnitExit {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return closedExit(void);
            if (capacity == 0 or self.config.capacity == null) return .{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.only_a_bounded_pool_accepts_a_positive_replacement_capacity } };
            self.config.capacity = capacity;
            self.notify();
            return .{ .success = {} };
        }
        /// Inspect idle pool resources with the supplied predicate.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `token: ?*const Cancellation`: Cancellation token observed by this operation; its owner must outlive
        ///   pending work.
        /// - `predicate: comptime fn (A) bool`: Compile-time typed callback; its signature is checked at comptime where
        ///   static.
        /// Returns (`outcomes.Exit(bool, E)`): Carries `bool` on `.success` or `E` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn existsIdle(self: *Self, token: ?*const Cancellation, comptime predicate: fn (A) bool) outcomes.Exit(bool, E) {
            self.mutex.lockUncancelable(self.io);
            if (self.closed) {
                self.mutex.unlock(self.io);
                return closedExit(bool);
            }
            var count: usize = 0;
            var current = self.idle;
            while (current) |resource| {
                count += 1;
                current = resource.next;
            }
            const snapshot = self.allocator.alloc(*Resource, count) catch {
                self.mutex.unlock(self.io);
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            current = self.idle;
            for (snapshot) |*item| {
                item.* = current.?;
                current.?.pins += 1;
                current = current.?.next;
            }
            self.operations += 1;
            self.mutex.unlock(self.io);
            defer {
                self.mutex.lockUncancelable(self.io);
                for (snapshot) |resource| resource.pins -= 1;
                self.operations -= 1;
                self.notify();
                self.mutex.unlock(self.io);
                self.allocator.free(snapshot);
            }
            var found = false;
            for (snapshot) |resource| {
                if (if (token) |origin| origin.isRequested() else false) return .interrupted;
                if (predicate(resource.value)) {
                    found = true;
                    break;
                }
            }
            self.mutex.lockUncancelable(self.io);
            const closed = self.closed;
            self.mutex.unlock(self.io);
            return if (closed) closedExit(bool) else .{ .success = found };
        }

        /// Populate the pool toward a requested target size.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `target: usize`: minimum pool population to create under the configured maximum.
        /// - `token: ?*const Cancellation`: Cancellation token observed by this operation; its owner must outlive
        ///   pending work.
        /// Returns (`UnitExit`): A terminal Exit with no success payload and a typed failure channel.
        pub fn ensureSize(self: *Self, target: usize, token: ?*const Cancellation) UnitExit {
            self.mutex.lockUncancelable(self.io);
            if (self.closed or (self.config.capacity != null and target > self.config.capacity.?)) {
                self.mutex.unlock(self.io);
                return .{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.pool_population_target_exceeds_capacity_or_pool_is_closed } };
            }
            self.operations += 1;
            var observer: Waiter = .{ .io = self.io, .next = self.observers };
            self.observers = &observer;
            self.notify();
            self.mutex.unlock(self.io);
            observer.registration.data = &observer;
            if (token) |origin| origin.register(&observer.registration);
            defer {
                observer.registration.unregister();
                self.mutex.lockUncancelable(self.io);
                var link = &self.observers;
                while (link.*) |item| {
                    if (item == &observer) {
                        link.* = item.next;
                        break;
                    }
                    link = &item.next;
                }
                self.operations -= 1;
                self.notify();
                self.mutex.unlock(self.io);
            }
            const Observation = struct { job: *Population, metadata: *Metadata };
            const BorrowerObservation = struct { job: *BorrowerCreation, metadata: *Metadata };
            var relevant: std.ArrayList(Observation) = .empty;
            var borrowers: std.ArrayList(BorrowerObservation) = .empty;
            defer {
                for (relevant.items) |item| {
                    item.metadata.finish();
                    item.job.release();
                }
                relevant.deinit(self.allocator);
                for (borrowers.items) |item| {
                    item.metadata.finish();
                    item.job.release();
                }
                borrowers.deinit(self.allocator);
            }
            while (true) {
                if (if (token) |origin| origin.isRequested() else false) return .interrupted;
                self.mutex.lockUncancelable(self.io);
                var job = self.populations;
                var pending = false;
                var failure: ?UnitExit = null;
                var clone_failed = false;
                while (job) |item| {
                    var known = false;
                    for (relevant.items) |other| if (other.job == item) {
                        known = true;
                        break;
                    };
                    if (self.live < target and !item.settled and !known) {
                        const metadata = Metadata.make(self.allocator) orelse {
                            self.mutex.unlock(self.io);
                            return .{ .die = outcomes.Defect.out_of_memory };
                        };
                        relevant.append(self.allocator, .{ .job = item, .metadata = metadata }) catch {
                            metadata.finish();
                            self.mutex.unlock(self.io);
                            return .{ .die = outcomes.Defect.out_of_memory };
                        };
                        item.retain();
                    }
                    job = item.next;
                }
                var borrower = self.borrower_creations;
                while (borrower) |item| {
                    var known = false;
                    for (borrowers.items) |other| if (other.job == item) {
                        known = true;
                        break;
                    };
                    if (self.live < target and !item.settled and !known) {
                        const metadata = Metadata.make(self.allocator) orelse {
                            self.mutex.unlock(self.io);
                            return .{ .die = outcomes.Defect.out_of_memory };
                        };
                        borrowers.append(self.allocator, .{ .job = item, .metadata = metadata }) catch {
                            metadata.finish();
                            self.mutex.unlock(self.io);
                            return .{ .die = outcomes.Defect.out_of_memory };
                        };
                        item.retain();
                        self.notify();
                    }
                    borrower = item.next;
                }
                var creation_failed = false;
                for (relevant.items) |observation| {
                    if (!observation.job.settled) pending = true else if (observation.job.outcome != .success) creation_failed = true;
                }
                for (borrowers.items) |observation| {
                    if (!observation.job.settled) pending = true else if (observation.job.outcome != .success) creation_failed = true;
                }
                if (!pending) for (relevant.items) |observation| {
                    const item = observation.job;
                    if (item.outcome != .success) {
                        const copy = item.outcome.observe();
                        clone_failed = clone_failed or !copy.copied;
                        const copied = copy.exit;
                        if (failure) |*existing| observation.metadata.storage.appendExit(existing, copied) else failure = copied;
                    }
                };
                if (!pending) for (borrowers.items) |observation| {
                    const item = observation.job;
                    if (item.outcome != .success) {
                        const copy = item.outcome.observe();
                        clone_failed = clone_failed or !copy.copied;
                        if (failure) |*existing| observation.metadata.storage.appendExit(existing, copy.exit) else failure = copy.exit;
                    }
                };
                if (!pending and failure != null) {
                    if (!clone_failed) for (relevant.items) |item| {
                        item.job.observed = true;
                    };
                    if (!clone_failed) for (borrowers.items) |item| {
                        item.job.observed = true;
                    };
                    self.mutex.unlock(self.io);
                    return failure.?;
                }
                if (failure) |*discarded| discarded.deinit();
                if (!pending and (self.closed or self.live >= target)) {
                    const closed = self.closed;
                    for (relevant.items) |item| item.job.observed = true;
                    for (borrowers.items) |item| item.job.observed = true;
                    self.mutex.unlock(self.io);
                    return if (closed) closedExit(void) else .{ .success = {} };
                }
                if (!pending and self.config.capacity != null and target > self.config.capacity.?) {
                    self.mutex.unlock(self.io);
                    return .{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.pool_population_target_exceeds_capacity_or_pool_is_closed } };
                }
                const beyond = self.config.capacity != null and target > self.config.capacity.?;
                // An observed failure may release a reservation while a sibling
                // is still pending. Drain that admitted group before deciding
                // whether more creation is needed; otherwise its failure is
                // replaced by new attempts before it can be returned.
                const needed = if (beyond or creation_failed) 0 else target -| self.allocated;
                var committed: ?*Population = null;
                var remaining = needed;
                // Reserve every structural object and every slot before the first
                // worker is submitted. A failed reservation starts no new factory.
                while (remaining != 0 and !self.closed) : (remaining -= 1) {
                    const item = self.allocator.create(Population) catch break;
                    const resource = self.allocator.create(Resource) catch {
                        self.allocator.destroy(item);
                        break;
                    };
                    const metadata = Metadata.make(self.allocator) orelse {
                        self.allocator.destroy(resource);
                        self.allocator.destroy(item);
                        break;
                    };
                    const native_metadata = Metadata.make(self.allocator) orelse {
                        metadata.finish();
                        self.allocator.destroy(resource);
                        self.allocator.destroy(item);
                        break;
                    };
                    const diagnostic_metadata = Metadata.make(self.allocator) orelse {
                        native_metadata.finish();
                        metadata.finish();
                        self.allocator.destroy(resource);
                        self.allocator.destroy(item);
                        break;
                    };
                    const resource_metadata = Metadata.make(self.allocator) orelse {
                        diagnostic_metadata.finish();
                        native_metadata.finish();
                        metadata.finish();
                        self.allocator.destroy(resource);
                        self.allocator.destroy(item);
                        break;
                    };
                    resource.* = .{ .metadata = resource_metadata };
                    item.* = .{ .pool = self, .resource = resource, .metadata = metadata, .native_metadata = native_metadata, .diagnostic_metadata = diagnostic_metadata, .next = committed };
                    const observation_metadata = Metadata.make(self.allocator) orelse {
                        item.release();
                        break;
                    };
                    relevant.append(self.allocator, .{ .job = item, .metadata = observation_metadata }) catch {
                        observation_metadata.finish();
                        item.release();
                        break;
                    };
                    item.retain();
                    committed = item;
                }
                if (remaining != 0) {
                    var item = committed;
                    while (item) |discarded| {
                        item = discarded.next;
                        const removed = relevant.pop().?;
                        removed.metadata.finish();
                        discarded.release();
                        discarded.release();
                    }
                    self.mutex.unlock(self.io);
                    return .{ .die = outcomes.Defect.out_of_memory };
                }
                var item = committed;
                while (item) |entry| {
                    self.allocated += 1;
                    self.operations += 1;
                    item = entry.next;
                }
                self.mutex.unlock(self.io);
                item = committed;
                while (item) |entry| {
                    const next = entry.next;
                    entry.registration.data = entry;
                    self.cancellation.register(&entry.registration);
                    const submitted = self.executor.submit(.independent, .{ .data = entry, .run = Population.run, .cancellation = &entry.cancellation });
                    if (submitted == .accepted) entry.handle.init(submitted.accepted, &entry.cancellation) else {
                        entry.created = .{ .die = submitted.rejected };
                        entry.ready.store(true, .release);
                    }
                    self.mutex.lockUncancelable(self.io);
                    entry.accepted = submitted == .accepted;
                    entry.submitted = true;
                    entry.next = self.populations;
                    self.populations = entry;
                    self.notify();
                    self.mutex.unlock(self.io);
                    item = next;
                }
                if (committed != null) continue;
                self.mutex.lockUncancelable(self.io);
                observer.wake.reset();
                const cancelled = if (token) |origin| origin.isRequested() else false;
                var settled = true;
                for (relevant.items) |entry| if (!entry.job.settled) {
                    settled = false;
                    break;
                };
                for (borrowers.items) |entry| if (!entry.job.settled) {
                    settled = false;
                    break;
                };
                const changed = cancelled or self.closed or (settled and (self.live >= target or beyond or relevant.items.len != 0 or borrowers.items.len != 0)) or (!beyond and !creation_failed and self.allocated < target);
                self.mutex.unlock(self.io);
                if (!changed) observer.wake.wait(self.io) catch return .interrupted;
            }
        }

        fn manage(data: *anyopaque) void {
            const self: *Self = @ptrCast(@alignCast(data));
            const protection = self.io.swapCancelProtection(.blocked);
            defer _ = self.io.swapCancelProtection(protection);
            while (true) {
                self.mutex.lockUncancelable(self.io);
                var selected: ?*Population = null;
                var retired: ?*Population = null;
                var retired_borrower: ?*BorrowerCreation = null;
                var borrower_link = &self.borrower_creations;
                while (borrower_link.*) |creation| {
                    if (creation.settled and creation.observed) {
                        borrower_link.* = creation.next;
                        retired_borrower = creation;
                        break;
                    }
                    borrower_link = &creation.next;
                }
                var link = &self.populations;
                // Detach only the owner this pass will process. Borrower
                // retirement has priority; removing a population or idle
                // resource as well would lose it when that branch continues.
                if (retired_borrower == null) {
                    while (link.*) |job| {
                        if (job.settled and job.observed) {
                            link.* = job.next;
                            retired = job;
                            break;
                        }
                        link = &job.next;
                    }
                }
                var current = self.populations;
                while (current) |job| {
                    if (!job.settled and job.submitted and job.ready.load(.acquire)) {
                        selected = job;
                        break;
                    }
                    current = job.next;
                }
                const idle = if (self.closed and retired_borrower == null and retired == null) self.idle else null;
                if (idle) |resource| self.idle = resource.next;
                const done = self.closed and self.operations == 0 and self.leases == 0 and selected == null and idle == null;
                if (selected == null and idle == null and retired == null and retired_borrower == null) self.wake.reset();
                self.mutex.unlock(self.io);
                if (retired_borrower) |creation| {
                    creation.release();
                    continue;
                }
                if (retired) |job| {
                    job.release();
                    continue;
                }
                if (idle) |resource| {
                    const metadata = resource.metadata;
                    var result = self.retire(resource, true);
                    self.mutex.lockUncancelable(self.io);
                    self.recordCleanup(&result, metadata);
                    metadata.finish();
                    self.notify();
                    self.mutex.unlock(self.io);
                    result.deinit();
                    continue;
                }
                if (selected) |job| {
                    var result: UnitExit = .{ .success = {} };
                    if (job.created == .success) {
                        self.mutex.lockUncancelable(self.io);
                        const order_valid = self.sequence != std.math.maxInt(u128);
                        if (order_valid) self.sequence += 1;
                        job.resource.* = .{ .value = job.created.success, .order = self.sequence, .metadata = job.resource.metadata };
                        self.live += 1;
                        const retire_resource = self.closed or job.cancellation.isRequested() or !order_valid;
                        self.mutex.unlock(self.io);
                        if (retire_resource) {
                            result = if (order_valid) .interrupted else .{ .die = .{ .kind = .invalid_configuration, .message = diagnostics.pool_resource_ordering_is_exhausted } };
                            job.metadata.storage.appendExit(&result, self.retire(job.resource, false));
                        } else {
                            const now = self.clock.now();
                            self.mutex.lockUncancelable(self.io);
                            job.resource.returned = now;
                            self.insertIdle(job.resource);
                            self.mutex.unlock(self.io);
                        }
                    } else {
                        result = outcomes.castFailure(void, job.created);
                        job.resource.metadata.finish();
                        self.allocator.destroy(job.resource);
                        self.mutex.lockUncancelable(self.io);
                        self.allocated -= 1;
                        self.mutex.unlock(self.io);
                    }
                    if (job.accepted) {
                        const joined = job.handle.joinProtected(true);
                        if (joined.forwarding) |defect| job.native_metadata.storage.append(&result, .{ .release_defect = defect });
                        if (joined.joining) |defect| job.native_metadata.storage.append(&result, .{ .release_defect = defect });
                    }
                    job.registration.unregister();
                    self.mutex.lockUncancelable(self.io);
                    job.outcome = result;
                    job.settled = true;
                    self.operations -= 1;
                    self.notify();
                    self.mutex.unlock(self.io);
                    continue;
                }
                if (done) return;
                self.wake.waitUncancelable(self.io);
            }
        }
        /// Stop the capability and release its retained resources.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`UnitExit`): A terminal Exit with no success payload and a typed failure channel.
        pub fn shutdown(self: *Self) UnitExit {
            const protection = self.io.swapCancelProtection(.blocked);
            defer _ = self.io.swapCancelProtection(protection);
            self.close_mutex.lockUncancelable(self.io);
            defer self.close_mutex.unlock(self.io);
            if (self.manager_joined) return self.closing.clone();
            self.mutex.lockUncancelable(self.io);
            self.closed = true;
            self.notify();
            self.mutex.unlock(self.io);
            self.cancellation.request();
            const joined = self.manager.joinProtected(true);
            self.manager.deinit();
            self.owner_registration.unregister();
            self.manager_joined = true;
            if (joined.defect) |defect| self.manager_metadata.storage.append(&self.closing, .{ .release_defect = defect });
            var current = self.populations;
            while (current) |job| {
                if (!job.observed and job.outcome != .success) {
                    job.diagnostic_metadata.storage.appendExit(&self.closing, job.outcome);
                    job.outcome = .{ .success = {} };
                }
                current = job.next;
            }
            var borrower = self.borrower_creations;
            while (borrower) |creation| {
                if (!creation.observed and creation.outcome != .success) {
                    creation.storage.appendExit(&self.closing, creation.outcome);
                    if (creation.storage.retained()) creation.retain();
                    creation.outcome = .{ .success = {} };
                }
                borrower = creation.next;
            }
            return self.closing.clone();
        }
        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            std.debug.assert(self.manager_joined and self.operations == 0 and self.leases == 0 and self.idle == null);
            var current = self.populations;
            while (current) |job| {
                current = job.next;
                job.release();
            }
            var borrower = self.borrower_creations;
            while (borrower) |creation| {
                borrower = creation.next;
                creation.release();
            }
            const manager_retained = self.manager_metadata.storage.retained();
            self.closing.deinit();
            if (!manager_retained) self.allocator.destroy(self.manager_metadata);
            self.factory.deinit(self.factory.userdata);
            self.allocator.destroy(self);
        }
    };
}
