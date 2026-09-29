//! Scoped, bounded broadcast with explicit slow-subscriber policy. A shared
//! ring retains one owned copy per event; temporary readers hold a reference
//! while cloning outside the mutex, so user clone/release code never runs
//! inside a critical section.
const std = @import("std");
const outcomes = @import("outcome.zig");
const effects = @import("effect.zig");
const Cancellation = @import("internal/runtime.zig").Cancellation;
const Waiter = @import("internal/waiter.zig").Waiter;

pub const Overflow = enum { backpressure, drop_oldest, drop_newest, reject };
pub const Error = enum { invalid_capacity, invalid_replay, closed, full, sequence_exhausted, concurrent_take, resource_limit, empty };
pub const PublishResult = union(enum) { published: u64, dropped };

pub fn Delivery(comptime T: type) type {
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = outcomes.ownsPayload(T);
        item: T,
        gap: u64,
        end,

        pub fn deinit(self: *Self) void {
            if (self.* == .item) outcomes.discardPayload(&self.item);
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .item => |value| .{ .item = try outcomes.clonePayload(value) },
                else => self.*,
            };
        }
    };
}

pub fn PubSub(comptime T: type) type {
    return struct {
        const Self = @This();
        const Slot = struct { refs: usize = 1, value: T };
        pub const Subscription = struct {
            const Sub = @This();
            bus: *Self,
            previous: ?*Sub = null,
            next: ?*Sub = null,
            cursor: u64,
            active: bool = true,
            taking: bool = false,
            waiters: ?*Waiter = null,

            pub fn takeToken(self: *Sub, token: ?*const Cancellation) outcomes.Exit(Delivery(T), Error) {
                const bus = self.bus;
                bus.mutex.lockUncancelable(bus.io);
                if (self.taking) {
                    bus.mutex.unlock(bus.io);
                    return .{ .fail = .concurrent_take };
                }
                self.taking = true;
                bus.mutex.unlock(bus.io);

                while (true) {
                    bus.mutex.lockUncancelable(bus.io);
                    if (if (token) |cancellation| cancellation.isRequested() else false) {
                        self.taking = false;
                        bus.mutex.unlock(bus.io);
                        return .interrupted;
                    }
                    if (self.cursor < bus.first_sequence) {
                        const skipped = bus.first_sequence - self.cursor;
                        self.cursor = bus.first_sequence;
                        self.taking = false;
                        Waiter.signalAll(bus.publisher_waiters);
                        bus.mutex.unlock(bus.io);
                        return .{ .success = .{ .gap = skipped } };
                    }
                    if (self.cursor < bus.next_sequence) {
                        const slot = bus.slots[bus.slotIndex(self.cursor)].?;
                        slot.refs = std.math.add(usize, slot.refs, 1) catch {
                            self.taking = false;
                            bus.mutex.unlock(bus.io);
                            return .{ .fail = .resource_limit };
                        };
                        bus.active_readers = std.math.add(usize, bus.active_readers, 1) catch {
                            slot.refs -= 1;
                            self.taking = false;
                            bus.mutex.unlock(bus.io);
                            return .{ .fail = .resource_limit };
                        };
                        bus.mutex.unlock(bus.io);
                        const copied = outcomes.clonePayload(slot.value) catch {
                            bus.mutex.lockUncancelable(bus.io);
                            self.taking = false;
                            bus.active_readers -= 1;
                            const destroy = bus.dropRefLocked(slot);
                            bus.mutex.unlock(bus.io);
                            if (destroy) bus.destroySlot(slot);
                            return .{ .die = outcomes.Defect.out_of_memory };
                        };
                        bus.mutex.lockUncancelable(bus.io);
                        self.cursor += 1;
                        self.taking = false;
                        bus.active_readers -= 1;
                        Waiter.signalAll(bus.publisher_waiters);
                        const destroy = bus.dropRefLocked(slot);
                        bus.mutex.unlock(bus.io);
                        if (destroy) bus.destroySlot(slot);
                        return .{ .success = .{ .item = copied } };
                    }
                    if (!self.active or bus.closed) {
                        self.taking = false;
                        bus.mutex.unlock(bus.io);
                        return .{ .success = .end };
                    }
                    var waiter: Waiter = .{ .io = bus.io };
                    waiter.register(&self.waiters, token);
                    bus.mutex.unlock(bus.io);
                    _ = waiter.event.wait(bus.io) catch {};
                    waiter.registration.unregister();
                    bus.mutex.lockUncancelable(bus.io);
                    waiter.remove(&self.waiters);
                    bus.mutex.unlock(bus.io);
                }
            }

            pub fn take(comptime R: type, self: *Sub) effects.Effect(Delivery(T), Error, R) {
                return effects.Effect(Delivery(T), Error, R).fromNativeCancellableBorrowed(Sub, self, struct {
                    fn run(subscription: *const Sub, _: *const R, token: ?*const Cancellation) outcomes.Exit(Delivery(T), Error) {
                        return @constCast(subscription).takeToken(token);
                    }
                }.run);
            }

            pub fn close(self: *Sub) void {
                const bus = self.bus;
                bus.mutex.lockUncancelable(bus.io);
                defer bus.mutex.unlock(bus.io);
                if (!self.active) return;
                self.active = false;
                if (self.previous) |previous| previous.next = self.next else bus.subscribers = self.next;
                if (self.next) |next| next.previous = self.previous;
                Waiter.signalAll(self.waiters);
                Waiter.signalAll(bus.publisher_waiters);
            }

            pub fn deinit(self: *Sub) void {
                self.close();
                const bus = self.bus;
                bus.mutex.lockUncancelable(bus.io);
                const drained = !self.taking and self.waiters == null;
                bus.mutex.unlock(bus.io);
                std.debug.assert(drained);
                bus.allocator.destroy(self);
            }
        };

        pub const SnapshotSubscription = struct {
            allocator: std.mem.Allocator,
            initial: T,
            subscription: *Subscription,

            pub fn deinit(self: *@This()) void {
                self.subscription.deinit();
                outcomes.discardPayload(&self.initial);
                self.allocator.destroy(self);
            }
        };

        allocator: std.mem.Allocator,
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        slots: []?*Slot,
        first_sequence: u64 = 0,
        next_sequence: u64 = 0,
        subscribers: ?*Subscription = null,
        publisher_waiters: ?*Waiter = null,
        active_readers: usize = 0,
        active_publishers: usize = 0,
        overflow: Overflow,
        replay: usize,
        closed: bool = false,

        /// Capacity and replay are caller policy. Sequence zero is the first
        /// publication; cursors always name the next publication to consume.
        pub fn init(env: anytype, capacity: usize, replay: usize, overflow: Overflow) outcomes.Exit(Self, Error) {
            if (capacity == 0 or std.math.cast(u64, capacity) == null) return .{ .fail = .invalid_capacity };
            if (replay > capacity) return .{ .fail = .invalid_replay };
            const slots = env.allocator.alloc(?*Slot, capacity) catch return .{ .die = outcomes.Defect.out_of_memory };
            @memset(slots, null);
            return .{ .success = .{ .allocator = env.allocator, .io = env.io, .slots = slots, .overflow = overflow, .replay = replay } };
        }

        fn slotIndex(self: *Self, sequence: u64) usize {
            return @intCast(sequence % @as(u64, @intCast(self.slots.len)));
        }

        fn dropRefLocked(_: *Self, slot: *Slot) bool {
            std.debug.assert(slot.refs > 0);
            slot.refs -= 1;
            return slot.refs == 0;
        }

        fn destroySlot(self: *Self, slot: *Slot) void {
            outcomes.discardPayload(&slot.value);
            self.allocator.destroy(slot);
        }

        fn oldestProtected(self: *Self) bool {
            var next = self.subscribers;
            while (next) |sub| {
                if (sub.cursor <= self.first_sequence) return true;
                next = sub.next;
            }
            return false;
        }

        fn publishToken(self: *Self, value: T, token: ?*const Cancellation) outcomes.Exit(PublishResult, Error) {
            self.mutex.lockUncancelable(self.io);
            if (self.closed) {
                self.mutex.unlock(self.io);
                return .{ .fail = .closed };
            }
            self.active_publishers = std.math.add(usize, self.active_publishers, 1) catch {
                self.mutex.unlock(self.io);
                return .{ .fail = .resource_limit };
            };
            self.mutex.unlock(self.io);
            defer {
                self.mutex.lockUncancelable(self.io);
                self.active_publishers -= 1;
                self.mutex.unlock(self.io);
            }
            const copy = outcomes.clonePayload(value) catch return .{ .die = outcomes.Defect.out_of_memory };
            const slot = self.allocator.create(Slot) catch {
                var owned = copy;
                outcomes.discardPayload(&owned);
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            slot.* = .{ .value = copy };
            var installed = false;
            defer if (!installed) self.destroySlot(slot);

            while (true) {
                self.mutex.lockUncancelable(self.io);
                if (if (token) |cancellation| cancellation.isRequested() else false) {
                    self.mutex.unlock(self.io);
                    return .interrupted;
                }
                if (self.closed) {
                    self.mutex.unlock(self.io);
                    return .{ .fail = .closed };
                }
                if (self.next_sequence == std.math.maxInt(u64)) {
                    self.mutex.unlock(self.io);
                    return .{ .fail = .sequence_exhausted };
                }
                const full = self.next_sequence - self.first_sequence == @as(u64, @intCast(self.slots.len));
                if (full and self.oldestProtected()) switch (self.overflow) {
                    .drop_newest => {
                        self.mutex.unlock(self.io);
                        return .{ .success = .dropped };
                    },
                    .reject => {
                        self.mutex.unlock(self.io);
                        return .{ .fail = .full };
                    },
                    .backpressure => {
                        var waiter: Waiter = .{ .io = self.io };
                        waiter.register(&self.publisher_waiters, token);
                        self.mutex.unlock(self.io);
                        _ = waiter.event.wait(self.io) catch {};
                        waiter.registration.unregister();
                        self.mutex.lockUncancelable(self.io);
                        waiter.remove(&self.publisher_waiters);
                        self.mutex.unlock(self.io);
                        continue;
                    },
                    .drop_oldest => {},
                };
                const sequence = self.next_sequence;
                const index = self.slotIndex(sequence);
                const old_slot = if (full) self.slots[index] else null;
                self.slots[index] = slot;
                self.next_sequence += 1;
                if (full) self.first_sequence += 1;
                installed = true;
                var subscriber = self.subscribers;
                while (subscriber) |sub| {
                    Waiter.signalAll(sub.waiters);
                    subscriber = sub.next;
                }
                const destroy_old = if (old_slot) |old| self.dropRefLocked(old) else false;
                self.mutex.unlock(self.io);
                if (destroy_old) self.destroySlot(old_slot.?);
                return .{ .success = .{ .published = sequence } };
            }
        }

        /// Direct local publication for provider bootstrapping and other
        /// non-cancellable synchronous boundaries. Effect callers use publish.
        pub fn publishImmediate(self: *Self, value: T) outcomes.Exit(PublishResult, Error) {
            return self.publishToken(value, null);
        }

        /// A bridge with an owned runtime item can use the Effect branch token
        /// so backpressure never traps cancellation behind a publisher wait.
        pub fn publishCancellable(self: *Self, value: T, token: ?*const Cancellation) outcomes.Exit(PublishResult, Error) {
            return self.publishToken(value, token);
        }

        pub fn publish(comptime R: type, comptime select_bus: fn (*const R) *Self, comptime select_value: fn (*const R) T) effects.Effect(PublishResult, Error, R) {
            return effects.Effect(PublishResult, Error, R).fromNativeCancellableBorrowed(void, &{}, struct {
                fn run(_: *const void, env: *const R, token: ?*const Cancellation) outcomes.Exit(PublishResult, Error) {
                    return select_bus(env).publishToken(select_value(env), token);
                }
            }.run);
        }

        pub fn subscribe(self: *Self) outcomes.Exit(*Subscription, Error) {
            const sub = self.allocator.create(Subscription) catch return .{ .die = outcomes.Defect.out_of_memory };
            self.mutex.lockUncancelable(self.io);
            if (self.closed) {
                self.mutex.unlock(self.io);
                self.allocator.destroy(sub);
                return .{ .fail = .closed };
            }
            const retained = self.next_sequence - self.first_sequence;
            const replay_count: u64 = @min(@as(u64, @intCast(self.replay)), retained);
            sub.* = .{ .bus = self, .cursor = self.next_sequence - replay_count, .next = self.subscribers };
            if (self.subscribers) |head| head.previous = sub;
            self.subscribers = sub;
            self.mutex.unlock(self.io);
            return .{ .success = sub };
        }

        /// Captures the latest retained value and installs the subscriber at
        /// one mutex boundary. The caller receives that initial value, then
        /// consumes only later publications. Cloning runs after unlocking
        /// while a slot reference prevents use-after-free.
        pub fn snapshotAndSubscribe(self: *Self) outcomes.Exit(*SnapshotSubscription, Error) {
            const observation = self.allocator.create(SnapshotSubscription) catch return .{ .die = outcomes.Defect.out_of_memory };
            const sub = self.allocator.create(Subscription) catch {
                self.allocator.destroy(observation);
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            self.mutex.lockUncancelable(self.io);
            if (self.closed or self.next_sequence == self.first_sequence) {
                const problem: Error = if (self.closed) .closed else .empty;
                self.mutex.unlock(self.io);
                self.allocator.destroy(sub);
                self.allocator.destroy(observation);
                return .{ .fail = problem };
            }
            const latest_sequence = self.next_sequence - 1;
            const slot = self.slots[self.slotIndex(latest_sequence)].?;
            slot.refs = std.math.add(usize, slot.refs, 1) catch {
                self.mutex.unlock(self.io);
                self.allocator.destroy(sub);
                self.allocator.destroy(observation);
                return .{ .fail = .resource_limit };
            };
            self.active_readers = std.math.add(usize, self.active_readers, 1) catch {
                slot.refs -= 1;
                self.mutex.unlock(self.io);
                self.allocator.destroy(sub);
                self.allocator.destroy(observation);
                return .{ .fail = .resource_limit };
            };
            sub.* = .{ .bus = self, .cursor = self.next_sequence, .next = self.subscribers };
            if (self.subscribers) |head| head.previous = sub;
            self.subscribers = sub;
            self.mutex.unlock(self.io);

            const initial = outcomes.clonePayload(slot.value) catch {
                sub.deinit();
                self.mutex.lockUncancelable(self.io);
                self.active_readers -= 1;
                const destroy = self.dropRefLocked(slot);
                self.mutex.unlock(self.io);
                if (destroy) self.destroySlot(slot);
                self.allocator.destroy(observation);
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            self.mutex.lockUncancelable(self.io);
            self.active_readers -= 1;
            const destroy = self.dropRefLocked(slot);
            self.mutex.unlock(self.io);
            if (destroy) self.destroySlot(slot);
            observation.* = .{ .allocator = self.allocator, .initial = initial, .subscription = sub };
            return .{ .success = observation };
        }

        pub fn latest(self: *Self) outcomes.Exit(T, Error) {
            self.mutex.lockUncancelable(self.io);
            if (self.next_sequence == self.first_sequence) {
                self.mutex.unlock(self.io);
                return .{ .fail = .empty };
            }
            const slot = self.slots[self.slotIndex(self.next_sequence - 1)].?;
            slot.refs = std.math.add(usize, slot.refs, 1) catch {
                self.mutex.unlock(self.io);
                return .{ .fail = .resource_limit };
            };
            self.active_readers = std.math.add(usize, self.active_readers, 1) catch {
                slot.refs -= 1;
                self.mutex.unlock(self.io);
                return .{ .fail = .resource_limit };
            };
            self.mutex.unlock(self.io);
            const value = outcomes.clonePayload(slot.value) catch {
                self.mutex.lockUncancelable(self.io);
                self.active_readers -= 1;
                const destroy = self.dropRefLocked(slot);
                self.mutex.unlock(self.io);
                if (destroy) self.destroySlot(slot);
                return .{ .die = outcomes.Defect.out_of_memory };
            };
            self.mutex.lockUncancelable(self.io);
            self.active_readers -= 1;
            const destroy = self.dropRefLocked(slot);
            self.mutex.unlock(self.io);
            if (destroy) self.destroySlot(slot);
            return .{ .success = value };
        }

        pub fn withSubscription(
            comptime R: type,
            comptime A: type,
            comptime E: type,
            comptime select_bus: fn (*const R) *Self,
            comptime map_error: fn (Error) E,
            comptime use: fn (*Subscription) effects.Effect(A, E, R),
        ) effects.Effect(A, E, R) {
            const Acquire = effects.Effect(*Subscription, Error, R);
            const acquire = Acquire.fromOutcome(struct {
                fn run(env: *const R) outcomes.Exit(*Subscription, Error) {
                    return select_bus(env).subscribe();
                }
            }.run).mapError(map_error);
            return acquire.acquireRelease(use, struct {
                fn finish(sub: *Subscription, _: outcomes.PrimaryOutcome(E)) effects.Effect(void, E, R) {
                    return effects.Effect(void, E, R).fromBorrowed(Subscription, sub, struct {
                        fn release(owner: *const Subscription, _: *const R) outcomes.Exit(void, E) {
                            @constCast(owner).deinit();
                            return .{ .success = {} };
                        }
                    }.release);
                }
            }.finish);
        }

        pub fn shutdown(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            self.closed = true;
            Waiter.signalAll(self.publisher_waiters);
            var subscriber = self.subscribers;
            while (subscriber) |sub| {
                Waiter.signalAll(sub.waiters);
                subscriber = sub.next;
            }
        }

        pub fn deinit(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            const drained = self.closed and self.subscribers == null and self.publisher_waiters == null and self.active_readers == 0 and self.active_publishers == 0;
            self.mutex.unlock(self.io);
            std.debug.assert(drained);
            for (self.slots) |maybe_slot| if (maybe_slot) |slot| self.destroySlot(slot);
            self.allocator.free(self.slots);
            self.* = undefined;
        }
    };
}
