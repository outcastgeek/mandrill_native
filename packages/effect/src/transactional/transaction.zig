//! Mixed-type optimistic transactions in one caller-owned local domain.
//! The journal is the sole path for reads/writes; callbacks are repeatable,
//! synchronous descriptions and cannot bind arbitrary Effect or native I/O.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const Cancellation = @import("../internal/runtime.zig").Cancellation;
const Waiter = @import("../internal/waiter.zig").Waiter;
const TransactionError = @import("transaction_error.zig").Error;

/// Read the committed value without exposing mutable storage.
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// Returns (`bool`): `true` for a recursively copyable, non-owning value shape permitted in a transaction.
pub fn immutableValue(comptime T: type) bool {
    if (outcomes.ownsPayload(T)) return false;
    return switch (@typeInfo(T)) {
        .void, .bool, .int, .float, .enum_literal, .@"enum", .error_set => true,
        .optional => |info| immutableValue(info.child),
        .array => |info| immutableValue(info.child),
        .vector => |info| immutableValue(info.child),
        .@"struct" => |info| blk: {
            inline for (info.field_types) |Field| if (!immutableValue(Field)) break :blk false;
            break :blk true;
        },
        .@"union" => |info| blk: {
            inline for (info.field_types) |Field| if (!immutableValue(Field)) break :blk false;
            break :blk true;
        },
        else => false,
    };
}

/// Specialize a transactional reference by its stored value type.
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// Returns (`type`): journal-backed transactional reference to `T`.
pub fn Ref(comptime T: type) type {
    if (!immutableValue(T))
        @compileError("transactional references require immutable, pointer-free, copyable values");
    return struct {
        const Self = @This();
        domain: *Domain,
        value: T,
        version: u64 = 0,

        /// Create a transactional reference with its initial value.
        /// Parameters:
        /// - `domain: *Domain`: Transaction domain containing the cells and journal for this operation.
        /// - `value: T`: initial immutable value held by this transactional reference.
        /// Returns (`Self`): a reference whose writes are staged in the supplied domain.
        pub fn init(domain: *Domain, value: T) Self {
            return .{ .domain = domain, .value = value };
        }
    };
}

/// Specialize a committed value together with the revision that produced it.
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// Returns (`type`): committed value paired with the revision that produced it.
pub fn Observed(comptime T: type) type {
    return struct { value: T, version: u64 };
}

/// Specialize the typed decision outcome for transactional operations.
/// Parameters:
/// - `A: comptime type`: Value type produced by a successful computation.
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): transaction result distinguishing commit, retry, and typed failure.
pub fn Decision(comptime A: type, comptime E: type) type {
    return union(enum) {
        const Self = @This();
        pub const Success = A;
        pub const Error = E;
        commit: A,
        fail: E,
        retry,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            switch (self.*) {
                .commit => outcomes.discardPayload(&self.commit),
                .fail => outcomes.discardPayload(&self.fail),
                .retry => {},
            }
            self.* = undefined;
        }
    };
}

/// Caller-owned, process-local serializable transaction domain. A short commit
/// gate validates read versions and publishes staged writes together; retries
/// wait on observed changes without holding the gate across suspension.
pub const Domain = struct {
    const Self = @This();
    const Commit = enum { committed, conflict, version_exhausted, unavailable };
    const Wait = enum { changed, interrupted, unavailable };

    /// Per-attempt read and write set checked before atomic commit.
    pub const Journal = struct {
        const Entry = struct {
            identity: *anyopaque,
            version: *u64,
            observed: u64,
            snapshot: *anyopaque,
            staged: ?*anyopaque = null,
            apply: *const fn (*anyopaque, *const anyopaque) void,
            duplicate: *const fn (std.mem.Allocator, *const anyopaque) error{OutOfMemory}!*anyopaque,
            release: *const fn (std.mem.Allocator, *anyopaque) void,

            fn clone(self: *const Entry, allocator: std.mem.Allocator) error{OutOfMemory}!Entry {
                const snapshot = try self.duplicate(allocator, self.snapshot);
                errdefer self.release(allocator, snapshot);
                const staged = if (self.staged) |value| try self.duplicate(allocator, value) else null;
                return .{
                    .identity = self.identity,
                    .version = self.version,
                    .observed = self.observed,
                    .snapshot = snapshot,
                    .staged = staged,
                    .apply = self.apply,
                    .duplicate = self.duplicate,
                    .release = self.release,
                };
            }

            fn deinit(self: *Entry, allocator: std.mem.Allocator) void {
                self.release(allocator, self.snapshot);
                if (self.staged) |value| self.release(allocator, value);
                self.* = undefined;
            }
        };
        /// `Fault` cases: `out_of_memory`, `cross_domain`.
        pub const Fault = enum { out_of_memory, cross_domain };
        /// Committed value and revision snapshot used by observers.
        pub const Checkpoint = struct {
            entries: std.array_list.Managed(Entry),
            allocator: std.mem.Allocator,

            /// Release resources owned by this value.
            /// Parameters:
            /// - `self: *Checkpoint`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
            /// Returns (`void`): no value.
            pub fn deinit(self: *Checkpoint) void {
                for (self.entries.items) |*entry| entry.deinit(self.allocator);
                self.entries.deinit();
                self.* = undefined;
            }
        };

        domain: *Self,
        allocator: std.mem.Allocator,
        entries: std.array_list.Managed(Entry),
        fault: ?Fault = null,

        fn init(domain: *Self, allocator: std.mem.Allocator) Journal {
            return .{
                .domain = domain,
                .allocator = allocator,
                .entries = std.array_list.Managed(Entry).init(allocator),
            };
        }

        fn deinit(self: *Journal) void {
            for (self.entries.items) |*entry| entry.deinit(self.allocator);
            self.entries.deinit();
            self.* = undefined;
        }

        /// An alternative records an exact journal savepoint. Abandoning its
        /// retry branch restores read and write sets, including prior staged
        /// values, before evaluating the next alternative.
        ///
        /// Parameters:
        /// - `self: *Journal`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`error{OutOfMemory}!Checkpoint`): A `Checkpoint` on success; `OutOfMemory` if required allocation fails.
        pub fn checkpoint(self: *Journal) error{OutOfMemory}!Checkpoint {
            var saved: Checkpoint = .{
                .entries = std.array_list.Managed(Entry).init(self.allocator),
                .allocator = self.allocator,
            };
            errdefer saved.deinit();
            try saved.entries.ensureTotalCapacity(self.entries.items.len);
            for (self.entries.items) |*entry| {
                const copy = try entry.clone(self.allocator);
                saved.entries.appendAssumeCapacity(copy);
            }
            return saved;
        }

        /// Restore state from the supplied snapshot or checkpoint.
        /// Parameters:
        /// - `self: *Journal`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `saved: *Checkpoint`: Borrowed checkpoint used for rollback or replay.
        /// Returns (`void`): no value.
        pub fn restore(self: *Journal, saved: *Checkpoint) void {
            for (self.entries.items) |*entry| entry.deinit(self.allocator);
            self.entries.deinit();
            self.entries = saved.entries;
            saved.entries = std.array_list.Managed(Entry).init(saved.allocator);
        }

        fn find(self: *Journal, identity: *anyopaque) ?*Entry {
            for (self.entries.items) |*entry| if (entry.identity == identity) return entry;
            return null;
        }

        /// Read through the selected capability.
        /// Parameters:
        /// - `self: *Journal`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `T: comptime type`: Value type stored or transported by the returned component.
        /// - `reference: *Ref(T)`: Reference cell used for transactional access.
        /// Returns (`outcomes.Result(T, Fault)`): An ok/err Result; owned payloads must be released according to their type contract.
        pub fn read(self: *Journal, comptime T: type, reference: *Ref(T)) outcomes.Result(T, Fault) {
            if (self.fault) |fault| return .{ .err = fault };
            if (reference.domain != self.domain) {
                self.fault = .cross_domain;
                return .{ .err = .cross_domain };
            }
            if (self.find(reference)) |entry| {
                const current: *const T = @ptrCast(@alignCast(entry.staged orelse entry.snapshot));
                return .{ .ok = current.* };
            }
            const snapshot = self.allocator.create(T) catch {
                self.fault = .out_of_memory;
                return .{ .err = .out_of_memory };
            };
            self.domain.mutex.lockUncancelable(self.domain.io);
            snapshot.* = reference.value;
            const observed = reference.version;
            self.domain.mutex.unlock(self.domain.io);
            self.entries.append(.{
                .identity = reference,
                .version = &reference.version,
                .observed = observed,
                .snapshot = snapshot,
                .apply = struct {
                    fn apply(identity: *anyopaque, source: *const anyopaque) void {
                        const cell: *Ref(T) = @ptrCast(@alignCast(identity));
                        const value: *const T = @ptrCast(@alignCast(source));
                        cell.value = value.*;
                    }
                }.apply,
                .duplicate = struct {
                    fn duplicate(allocator: std.mem.Allocator, source: *const anyopaque) error{OutOfMemory}!*anyopaque {
                        const value: *const T = @ptrCast(@alignCast(source));
                        const copy = try allocator.create(T);
                        copy.* = value.*;
                        return copy;
                    }
                }.duplicate,
                .release = struct {
                    fn release(allocator: std.mem.Allocator, source: *anyopaque) void {
                        allocator.destroy(@as(*T, @ptrCast(@alignCast(source))));
                    }
                }.release,
            }) catch {
                self.allocator.destroy(snapshot);
                self.fault = .out_of_memory;
                return .{ .err = .out_of_memory };
            };
            return .{ .ok = snapshot.* };
        }

        /// Read a transactional value and record its observed revision.
        /// Parameters:
        /// - `self: *Journal`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `T: comptime type`: Value type stored or transported by the returned component.
        /// - `reference: *Ref(T)`: Reference cell used for transactional access.
        /// Returns (`outcomes.Result(Observed(T), Fault)`): An ok/err Result; owned payloads must be released according to their type contract.
        pub fn readObserved(self: *Journal, comptime T: type, reference: *Ref(T)) outcomes.Result(Observed(T), Fault) {
            const value = switch (self.read(T, reference)) {
                .ok => |current| current,
                .err => |fault| return .{ .err = fault },
            };
            return .{ .ok = .{ .value = value, .version = self.find(reference).?.observed } };
        }

        /// Write through the selected capability.
        /// Parameters:
        /// - `self: *Journal`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `T: comptime type`: Value type stored or transported by the returned component.
        /// - `reference: *Ref(T)`: Reference cell used for transactional access.
        /// - `value: T`: immutable replacement staged in the current journal.
        /// Returns (`void`): no value.
        pub fn write(self: *Journal, comptime T: type, reference: *Ref(T), value: T) void {
            if (self.read(T, reference) == .err) return;
            const entry = self.find(reference).?;
            if (entry.staged) |existing| {
                const stored: *T = @ptrCast(@alignCast(existing));
                stored.* = value;
                return;
            }
            const staged = self.allocator.create(T) catch {
                self.fault = .out_of_memory;
                return;
            };
            staged.* = value;
            entry.staged = staged;
        }

        /// Update the current state with the supplied callback.
        /// Parameters:
        /// - `self: *Journal`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `T: comptime type`: Value type stored or transported by the returned component.
        /// - `reference: *Ref(T)`: Reference cell used for transactional access.
        /// - `change: comptime fn (T) T`: Compile-time typed function selected at comptime; no runtime callback
        ///   lookup is required.
        /// Returns (`void`): no value.
        pub fn modify(self: *Journal, comptime T: type, reference: *Ref(T), comptime change: fn (T) T) void {
            const current = switch (self.read(T, reference)) {
                .ok => |value| value,
                .err => return,
            };
            self.write(T, reference, change(current));
        }

        /// Validate the transaction’s current journal view.
        /// Parameters:
        /// - `self: *Journal`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `condition: bool`: Condition controlling the selected branch.
        /// Returns (`bool`): `true` only when `condition` holds and the journal has no prior fault.
        pub fn check(self: *Journal, condition: bool) bool {
            if (!condition) return false;
            return self.fault == null;
        }
    };

    io: std.Io,
    mutex: std.Io.Mutex = .init,
    signal_mutex: std.Io.Mutex = .init,
    waiters: ?*Waiter = null,
    active_waiters: usize = 0,
    closed: bool = false,

    /// Create a transaction domain using the caller environment.
    /// Parameters:
    /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
    /// Returns (`Self`): a domain that owns its synchronized journal state.
    pub fn init(env: anytype) Self {
        const R = @typeInfo(@TypeOf(env)).pointer.child;
        if (R == void or !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
            @compileError("transaction domain requires caller I/O in R");
        return .{ .io = env.io };
    }

    /// Stop the capability and release its retained resources.
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn shutdown(self: *Self) void {
        self.signal_mutex.lockUncancelable(self.io);
        self.mutex.lockUncancelable(self.io);
        self.closed = true;
        const pending = self.detachWaiters();
        self.mutex.unlock(self.io);
        Waiter.signalAll(pending);
        self.signal_mutex.unlock(self.io);
    }

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *Self) void {
        self.shutdown();
        std.debug.assert(self.active_waiters == 0);
        self.* = undefined;
    }

    fn commit(self: *Self, journal: *const Journal) Commit {
        self.signal_mutex.lockUncancelable(self.io);
        defer self.signal_mutex.unlock(self.io);
        self.mutex.lockUncancelable(self.io);
        if (self.closed) {
            self.mutex.unlock(self.io);
            return .unavailable;
        }
        for (journal.entries.items) |entry| {
            if (entry.version.* != entry.observed) {
                self.mutex.unlock(self.io);
                return .conflict;
            }
            if (entry.staged != null and entry.version.* == std.math.maxInt(u64)) {
                self.mutex.unlock(self.io);
                return .version_exhausted;
            }
        }
        var changed = false;
        for (journal.entries.items) |entry| {
            if (entry.staged) |staged| {
                entry.apply(entry.identity, staged);
                entry.version.* += 1;
                changed = true;
            }
        }
        const pending = if (changed) self.detachWaiters() else null;
        self.mutex.unlock(self.io);
        Waiter.signalAll(pending);
        return .committed;
    }

    fn detachWaiters(self: *Self) ?*Waiter {
        const pending = self.waiters;
        self.waiters = null;
        var current = pending;
        while (current) |waiter| {
            waiter.detached = true;
            current = waiter.next;
        }
        return pending;
    }

    fn waitForReadChange(self: *Self, journal: *const Journal, token: ?*const Cancellation) Wait {
        if (token) |cancellation| if (cancellation.isRequested()) return .interrupted;
        var waiter: Waiter = .{ .io = self.io };
        self.mutex.lockUncancelable(self.io);
        if (self.closed) {
            self.mutex.unlock(self.io);
            return .unavailable;
        }
        for (journal.entries.items) |entry| {
            if (entry.version.* != entry.observed) {
                self.mutex.unlock(self.io);
                return .changed;
            }
        }
        waiter.register(&self.waiters, token);
        self.active_waiters += 1;
        self.mutex.unlock(self.io);
        _ = waiter.event.wait(self.io) catch {};
        waiter.registration.unregister();
        self.signal_mutex.lockUncancelable(self.io);
        self.mutex.lockUncancelable(self.io);
        if (!waiter.detached) waiter.remove(&self.waiters);
        self.active_waiters -= 1;
        const unavailable = self.closed;
        self.mutex.unlock(self.io);
        self.signal_mutex.unlock(self.io);
        if (token) |cancellation| if (cancellation.isRequested()) return .interrupted;
        return if (unavailable) .unavailable else .changed;
    }

    /// Evaluate and commit a transaction with retry on conflict.
    /// Parameters:
    /// - `A: comptime type`: Value type produced by a successful computation.
    /// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
    /// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
    /// - `Capture: comptime type`: Compile-time captured state type retained by the composed operation.
    /// - `capture: *const Capture`: Operation-specific state retained until the callback completes.
    /// - `select_domain: comptime fn (*const R) *Self`: Compile-time typed selector from the current environment; its result
    ///   must remain valid through use.
    /// - `evaluate: comptime fn (*const Capture, *Journal) Decision(A, E)`: Compile-time typed function selected at comptime; no runtime callback lookup
    ///   is required.
    /// Returns (`effects.Effect(A, TransactionError(E), R)`): typed result after the journal read set validates and its writes commit.
    pub fn atomically(
        comptime A: type,
        comptime E: type,
        comptime R: type,
        comptime Capture: type,
        capture: *const Capture,
        comptime select_domain: fn (*const R) *Self,
        comptime evaluate: fn (*const Capture, *Journal) Decision(A, E),
    ) effects.Effect(A, TransactionError(E), R) {
        const Failure = TransactionError(E);
        if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
            @compileError("transaction execution requires caller allocator in R");
        return effects.Effect(A, Failure, R).fromNativeCancellableBorrowed(Capture, capture, struct {
            fn run(input: *const Capture, env: *const R, token: ?*const Cancellation) outcomes.Exit(A, Failure) {
                const domain = select_domain(env);
                while (true) {
                    if (token) |cancellation| if (cancellation.isRequested()) return .interrupted;
                    var journal = Journal.init(domain, env.allocator);
                    var decision = evaluate(input, &journal);
                    if (journal.fault) |fault| {
                        decision.deinit();
                        journal.deinit();
                        return switch (fault) {
                            .out_of_memory => .{ .die = outcomes.Defect.out_of_memory },
                            .cross_domain => .{ .fail = .cross_domain },
                        };
                    }
                    switch (decision) {
                        .fail => |failure| {
                            journal.deinit();
                            return .{ .fail = .{ .domain = failure } };
                        },
                        .commit => |result| {
                            const status = domain.commit(&journal);
                            journal.deinit();
                            switch (status) {
                                .committed => return .{ .success = result },
                                .conflict => {
                                    var discarded = result;
                                    outcomes.discardPayload(&discarded);
                                    continue;
                                },
                                .version_exhausted, .unavailable => {
                                    var discarded = result;
                                    outcomes.discardPayload(&discarded);
                                    return .{ .fail = if (status == .version_exhausted) .version_exhausted else .unavailable };
                                },
                            }
                        },
                        .retry => {
                            if (journal.entries.items.len == 0) {
                                journal.deinit();
                                return .{ .fail = .empty_retry_read_set };
                            }
                            const status = domain.waitForReadChange(&journal, token);
                            journal.deinit();
                            switch (status) {
                                .changed => continue,
                                .interrupted => return .interrupted,
                                .unavailable => return .{ .fail = .unavailable },
                            }
                        },
                    }
                }
            }
        }.run);
    }
};
