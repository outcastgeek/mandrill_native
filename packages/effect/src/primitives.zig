const diagnostics = @import("internal/diagnostics.zig");
const std = @import("std");
const outcomes = @import("outcome.zig");
const Cancellation = @import("internal/runtime.zig").Cancellation;
const Waiter = @import("internal/waiter.zig").Waiter;

pub fn Deferred(comptime A: type, comptime E: type) type {
    return struct {
        const Self = @This();
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        value: ?outcomes.Exit(A, E) = null,
        copy_failed: bool = false,
        waiters: ?*Waiter = null,

        pub fn init(env: anytype) Self {
            return .{ .io = env.io };
        }

        /// Takes ownership only when true is returned. A rejected value remains
        /// owned by the caller; successful observers get separate metadata refs.
        pub fn complete(self: *Self, value: outcomes.Exit(A, E)) bool {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.value != null) return false;
            self.value = value;
            Waiter.signalAll(self.waiters);
            return true;
        }

        /// Retains an independent result while preserving whether that copy
        /// succeeded. The source owner still owns the original result.
        pub fn completeCopy(self: *Self, value: outcomes.Exit(A, E)) bool {
            var result = value.observe();
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.value != null) {
                result.exit.deinit();
                return false;
            }
            self.value = result.exit;
            self.copy_failed = !result.copied;
            Waiter.signalAll(self.waiters);
            return true;
        }

        pub fn poll(self: *Self) ?outcomes.Exit(A, E) {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            return if (self.value) |value| value.clone() else null;
        }

        pub fn waitToken(self: *Self, token: ?*const Cancellation) outcomes.Exit(A, E) {
            return self.waitObserved(token).exit;
        }

        pub const WaitResult = struct { exit: outcomes.Exit(A, E), observer_interrupted: bool = false, clone_failed: bool = false };

        fn observation(self: *const Self, value: outcomes.Exit(A, E)) WaitResult {
            const result = value.observe();
            return .{ .exit = result.exit, .clone_failed = self.copy_failed or !result.copied };
        }

        pub fn waitObserved(self: *Self, token: ?*const Cancellation) WaitResult {
            var waiter: Waiter = .{ .io = self.io };
            self.mutex.lockUncancelable(self.io);
            if (token) |cancellation| if (cancellation.isRequested()) {
                self.mutex.unlock(self.io);
                return .{ .exit = .interrupted, .observer_interrupted = true };
            };
            if (self.value) |value| {
                const result = self.observation(value);
                self.mutex.unlock(self.io);
                return result;
            }
            waiter.register(&self.waiters, token);
            self.mutex.unlock(self.io);
            var interrupted = false;
            waiter.event.wait(self.io) catch {
                interrupted = true;
            };
            waiter.registration.unregister();
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            waiter.remove(&self.waiters);
            if (interrupted or (if (token) |cancellation| cancellation.isRequested() else false)) return .{ .exit = .interrupted, .observer_interrupted = true };
            return self.observation(self.value.?);
        }

        /// Waits for settlement without copying an owning result. Controllers
        /// use this before transferring their own retained diagnostic outcome.
        pub fn waitReadyToken(self: *Self, token: ?*const Cancellation) bool {
            var waiter: Waiter = .{ .io = self.io };
            self.mutex.lockUncancelable(self.io);
            if (token) |cancellation| if (cancellation.isRequested()) {
                self.mutex.unlock(self.io);
                return false;
            };
            if (self.value != null) {
                self.mutex.unlock(self.io);
                return true;
            }
            waiter.register(&self.waiters, token);
            self.mutex.unlock(self.io);
            var interrupted = false;
            waiter.event.wait(self.io) catch {
                interrupted = true;
            };
            waiter.registration.unregister();
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            waiter.remove(&self.waiters);
            return !interrupted and !(if (token) |cancellation| cancellation.isRequested() else false);
        }

        pub fn deinit(self: *Self) void {
            std.debug.assert(self.waiters == null);
            if (self.value) |*value| value.deinit();
            self.* = undefined;
        }
    };
}

pub fn Latch(comptime E: type) type {
    return Deferred(void, E);
}

pub fn modificationResult(comptime T: type, comptime updater: anytype) type {
    const info = switch (@typeInfo(@TypeOf(updater))) {
        .@"fn" => |info| info,
        else => @compileError(diagnostics.ref_modifier_contract),
    };
    if (info.params.len != 1 or info.params[0].type != T)
        @compileError(diagnostics.ref_modifier_contract);
    const Change = info.return_type orelse @compileError(diagnostics.ref_modifier_contract);
    if (@typeInfo(Change) != .@"struct" or !@hasField(Change, "result") or !@hasField(Change, "value"))
        @compileError(diagnostics.ref_modifier_contract);
    if (@FieldType(Change, "value") != T)
        @compileError(diagnostics.ref_modifier_contract);
    return @FieldType(Change, "result");
}

pub fn Ref(comptime T: type) type {
    if (outcomes.ownsPayload(T)) @compileError(diagnostics.ref_values_must_be_borrowed_handles_or_ordinary_copyable_values_an_owning);
    return struct {
        const Self = @This();
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        value: T,

        pub fn init(env: anytype, value: T) Self {
            return .{ .io = env.io, .value = value };
        }
        pub fn get(self: *Self) T {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            return self.value;
        }
        pub fn set(self: *Self, value: T) void {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            self.value = value;
        }
        pub fn modify(self: *Self, comptime updater: anytype) modificationResult(T, updater) {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            const next = updater(self.value);
            self.value = next.value;
            return next.result;
        }
        pub fn update(self: *Self, comptime transform: fn (T) T) void {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            self.value = transform(self.value);
        }
    };
}

pub const QueueError = enum { shutdown };

pub fn Queue(comptime T: type) type {
    return struct {
        const Self = @This();
        pub const TryOffer = enum { accepted, full, closed };
        const BufferNode = struct { value: T, next: ?*BufferNode = null };
        const Offer = struct { value: T, result: ?outcomes.Exit(void, QueueError) = null, waiter: Waiter, previous: ?*Offer = null, next: ?*Offer = null, token: ?*const Cancellation };
        const Taker = struct { result: ?outcomes.Exit(T, QueueError) = null, waiter: Waiter, previous: ?*Taker = null, next: ?*Taker = null, token: ?*const Cancellation };
        allocator: std.mem.Allocator,
        io: std.Io,
        capacity: ?usize,
        mutex: std.Io.Mutex = .init,
        items: []T,
        head: usize = 0,
        count: usize = 0,
        buffered: ?*BufferNode = null,
        buffer_tail: ?*BufferNode = null,
        offers: ?*Offer = null,
        offer_tail: ?*Offer = null,
        takers: ?*Taker = null,
        taker_tail: ?*Taker = null,
        waiters: ?*Waiter = null,
        closed: bool = false,

        /// null is explicit unbounded admission; zero is a rendezvous queue.
        pub fn init(env: anytype, capacity: ?usize) outcomes.Exit(Self, QueueError) {
            const items = env.allocator.alloc(T, capacity orelse 0) catch return .{ .die = outcomes.Defect.out_of_memory };
            return .{ .success = .{ .allocator = env.allocator, .io = env.io, .capacity = capacity, .items = items } };
        }

        fn removeOffer(self: *Self, offer: *Offer) void {
            if (offer.previous) |previous| previous.next = offer.next else self.offers = offer.next;
            if (offer.next) |next| next.previous = offer.previous else self.offer_tail = offer.previous;
        }

        fn removeTaker(self: *Self, taker: *Taker) void {
            if (taker.previous) |previous| previous.next = taker.next else self.takers = taker.next;
            if (taker.next) |next| next.previous = taker.previous else self.taker_tail = taker.previous;
        }

        fn popOffer(self: *Self) ?*Offer {
            while (self.offers) |offer| {
                self.removeOffer(offer);
                if (if (offer.token) |token| token.isRequested() else false) {
                    offer.result = .interrupted;
                    offer.waiter.event.set(self.io);
                    continue;
                }
                return offer;
            }
            return null;
        }

        fn popTaker(self: *Self) ?*Taker {
            while (self.takers) |taker| {
                self.removeTaker(taker);
                if (if (taker.token) |token| token.isRequested() else false) {
                    taker.result = .interrupted;
                    taker.waiter.event.set(self.io);
                    continue;
                }
                return taker;
            }
            return null;
        }

        fn appendBuffer(self: *Self, value: T) bool {
            if (self.capacity != null) {
                const tail = if (self.count < self.items.len - self.head) self.head + self.count else self.count - (self.items.len - self.head);
                self.items[tail] = value;
            } else {
                const node = self.allocator.create(BufferNode) catch return false;
                node.* = .{ .value = value };
                if (self.buffer_tail) |tail| tail.next = node else self.buffered = node;
                self.buffer_tail = node;
            }
            self.count += 1;
            return true;
        }

        fn popBuffer(self: *Self) T {
            self.count -= 1;
            if (self.capacity != null) {
                const value = self.items[self.head];
                self.head = if (self.head + 1 == self.items.len) 0 else self.head + 1;
                return value;
            }
            const node = self.buffered.?;
            const value = node.value;
            self.buffered = node.next;
            if (self.buffered == null) self.buffer_tail = null;
            self.allocator.destroy(node);
            return value;
        }

        /// Never waits for capacity. Ownership moves only on `accepted`;
        /// otherwise the caller retains `value`, including after a defect.
        pub fn tryOfferOwned(self: *Self, value: *T) outcomes.Exit(TryOffer, QueueError) {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .{ .success = .closed };
            if (self.popTaker()) |taker| {
                taker.result = .{ .success = value.* };
                value.* = undefined;
                taker.waiter.event.set(self.io);
                return .{ .success = .accepted };
            }
            if (self.capacity != null and self.count >= self.capacity.?)
                return .{ .success = .full };
            if (!self.appendBuffer(value.*)) return .{ .die = outcomes.Defect.out_of_memory };
            value.* = undefined;
            return .{ .success = .accepted };
        }

        pub fn offerToken(self: *Self, value: T, token: ?*const Cancellation) outcomes.Exit(void, QueueError) {
            self.mutex.lockUncancelable(self.io);
            if (if (token) |cancellation| cancellation.isRequested() else false) {
                self.mutex.unlock(self.io);
                return .interrupted;
            }
            if (self.closed) {
                self.mutex.unlock(self.io);
                return .{ .fail = .shutdown };
            }
            if (self.popTaker()) |taker| {
                taker.result = .{ .success = value };
                taker.waiter.event.set(self.io);
                self.mutex.unlock(self.io);
                return .{ .success = {} };
            }
            if (self.capacity == null or self.count < self.capacity.?) {
                const stored = self.appendBuffer(value);
                self.mutex.unlock(self.io);
                return if (stored) .{ .success = {} } else .{ .die = outcomes.Defect.out_of_memory };
            }
            var offer: Offer = .{ .value = value, .waiter = .{ .io = self.io }, .token = token, .previous = self.offer_tail };
            if (self.offer_tail) |tail| tail.next = &offer else self.offers = &offer;
            self.offer_tail = &offer;
            offer.waiter.register(&self.waiters, token);
            self.mutex.unlock(self.io);
            var interrupted = false;
            offer.waiter.event.wait(self.io) catch {
                interrupted = true;
            };
            offer.waiter.registration.unregister();
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            offer.waiter.remove(&self.waiters);
            // A committed transfer wins over a later cancellation request.
            if (offer.result) |result| return result;
            std.debug.assert(interrupted or (if (token) |cancellation| cancellation.isRequested() else false));
            self.removeOffer(&offer);
            return .interrupted;
        }

        pub fn takeToken(self: *Self, token: ?*const Cancellation) outcomes.Exit(T, QueueError) {
            self.mutex.lockUncancelable(self.io);
            if (if (token) |cancellation| cancellation.isRequested() else false) {
                self.mutex.unlock(self.io);
                return .interrupted;
            }
            if (self.count != 0) {
                const value = self.popBuffer();
                if (!self.closed) if (self.popOffer()) |offer| {
                    const stored = self.appendBuffer(offer.value);
                    offer.result = if (stored) .{ .success = {} } else .{ .die = outcomes.Defect.out_of_memory };
                    offer.waiter.event.set(self.io);
                };
                self.mutex.unlock(self.io);
                return .{ .success = value };
            }
            if (!self.closed) if (self.popOffer()) |offer| {
                const value = offer.value;
                offer.result = .{ .success = {} };
                offer.waiter.event.set(self.io);
                self.mutex.unlock(self.io);
                return .{ .success = value };
            };
            if (self.closed) {
                self.mutex.unlock(self.io);
                return .{ .fail = .shutdown };
            }
            var taker: Taker = .{ .waiter = .{ .io = self.io }, .token = token, .previous = self.taker_tail };
            if (self.taker_tail) |tail| tail.next = &taker else self.takers = &taker;
            self.taker_tail = &taker;
            taker.waiter.register(&self.waiters, token);
            self.mutex.unlock(self.io);
            var interrupted = false;
            taker.waiter.event.wait(self.io) catch {
                interrupted = true;
            };
            taker.waiter.registration.unregister();
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            taker.waiter.remove(&self.waiters);
            if (taker.result) |result| return result;
            std.debug.assert(interrupted or (if (token) |cancellation| cancellation.isRequested() else false));
            self.removeTaker(&taker);
            return .interrupted;
        }

        pub fn shutdown(self: *Self, drain: bool) void {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            self.closed = true;
            if (!drain) while (self.count != 0) {
                var discarded = self.popBuffer();
                outcomes.discardPayload(&discarded);
            };
            while (self.offers) |offer| {
                self.removeOffer(offer);
                offer.result = .{ .fail = .shutdown };
                offer.waiter.event.set(self.io);
            }
            while (self.takers) |taker| {
                self.removeTaker(taker);
                taker.result = .{ .fail = .shutdown };
                taker.waiter.event.set(self.io);
            }
        }

        pub fn size(self: *Self) usize {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            return self.count;
        }

        pub fn deinit(self: *Self) void {
            std.debug.assert(self.waiters == null and self.offers == null and self.takers == null);
            while (self.count != 0) {
                var discarded = self.popBuffer();
                outcomes.discardPayload(&discarded);
            }
            self.allocator.free(self.items);
            self.* = undefined;
        }
    };
}
