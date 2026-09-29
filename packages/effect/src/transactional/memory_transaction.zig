//! Volatile optimistic transactions over cells in one caller-owned domain.
//! A transaction callback stages only local reads and writes; validation and
//! publication happen atomically under the domain lock. Retry sleeps through
//! the caller's I/O implementation and is woken by a commit or cancellation.
const std = @import("std");
const effects = @import("../effect.zig");
const outcomes = @import("../outcome.zig");
const Cancellation = @import("../internal/runtime.zig").Cancellation;
const Waiter = @import("../internal/waiter.zig").Waiter;
const TransactionError = @import("transaction_error.zig").Error;

pub fn Decision(comptime A: type, comptime E: type) type {
    return union(enum) {
        const Self = @This();
        commit: A,
        fail: E,
        retry,

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

/// A domain may contain many cells of T. Distinct domains remain isolated,
/// including when their values happen to have the same Zig type.
pub fn Domain(comptime T: type) type {
    if (outcomes.ownsPayload(T))
        @compileError("transaction cells require a non-owning copyable value; own resources in a scope outside the transaction");
    return struct {
        const Self = @This();
        pub const Cell = struct {
            domain: *Self,
            value: T,
            version: u64 = 0,

            pub fn init(domain: *Self, value: T) Cell {
                return .{ .domain = domain, .value = value };
            }
        };

        pub const Journal = struct {
            const Entry = struct {
                cell: *Cell,
                observed: u64,
                snapshot: T,
                staged: ?T = null,
            };
            const Fault = enum { out_of_memory, cross_domain };

            domain: *Self,
            entries: std.array_list.Managed(Entry),
            fault: ?Fault = null,

            fn init(domain: *Self, allocator: std.mem.Allocator) Journal {
                return .{ .domain = domain, .entries = std.array_list.Managed(Entry).init(allocator) };
            }

            fn deinit(self: *Journal) void {
                self.entries.deinit();
                self.* = undefined;
            }

            fn find(self: *Journal, cell: *Cell) ?*Entry {
                for (self.entries.items) |*entry| if (entry.cell == cell) return entry;
                return null;
            }

            /// Null means the journal has a fault; the transaction runner
            /// returns its typed error or defect even if a callback ignores it.
            pub fn read(self: *Journal, cell: *Cell) ?T {
                if (self.fault != null) return null;
                if (cell.domain != self.domain) {
                    self.fault = .cross_domain;
                    return null;
                }
                if (self.find(cell)) |entry| return entry.staged orelse entry.snapshot;
                self.domain.mutex.lockUncancelable(self.domain.io);
                const current = cell.value;
                const version = cell.version;
                self.domain.mutex.unlock(self.domain.io);
                self.entries.append(.{ .cell = cell, .observed = version, .snapshot = current }) catch {
                    self.fault = .out_of_memory;
                    return null;
                };
                return current;
            }

            pub fn write(self: *Journal, cell: *Cell, value: T) void {
                if (self.read(cell) == null) return;
                self.find(cell).?.staged = value;
            }

            pub fn modify(self: *Journal, cell: *Cell, comptime change: fn (T) T) void {
                const current = self.read(cell) orelse return;
                self.write(cell, change(current));
            }
        };

        const Commit = enum { committed, conflict, version_exhausted, unavailable };
        const Wait = enum { changed, interrupted, unavailable };

        io: std.Io,
        mutex: std.Io.Mutex = .init,
        waiters: ?*Waiter = null,
        closed: bool = false,

        pub fn init(env: anytype) Self {
            const R = @typeInfo(@TypeOf(env)).pointer.child;
            if (R == void or !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
                @compileError("transaction domain requires the caller io in R");
            return .{ .io = env.io };
        }

        pub fn shutdown(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            self.closed = true;
            Waiter.signalAll(self.waiters);
            self.mutex.unlock(self.io);
        }

        pub fn deinit(self: *Self) void {
            self.shutdown();
            std.debug.assert(self.waiters == null);
            self.* = undefined;
        }

        fn commit(self: *Self, journal: *const Journal) Commit {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.closed) return .unavailable;
            for (journal.entries.items) |entry| {
                if (entry.cell.version != entry.observed) return .conflict;
                if (entry.staged != null and entry.cell.version == std.math.maxInt(u64)) return .version_exhausted;
            }
            var changed = false;
            for (journal.entries.items) |entry| {
                if (entry.staged) |value| {
                    entry.cell.value = value;
                    entry.cell.version += 1;
                    changed = true;
                }
            }
            if (changed) Waiter.signalAll(self.waiters);
            return .committed;
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
                if (entry.cell.version != entry.observed) {
                    self.mutex.unlock(self.io);
                    return .changed;
                }
            }
            waiter.register(&self.waiters, token);
            self.mutex.unlock(self.io);
            _ = waiter.event.wait(self.io) catch {};
            waiter.registration.unregister();
            self.mutex.lockUncancelable(self.io);
            waiter.remove(&self.waiters);
            const unavailable = self.closed;
            self.mutex.unlock(self.io);
            if (token) |cancellation| if (cancellation.isRequested()) return .interrupted;
            return if (unavailable) .unavailable else .changed;
        }

        /// Capture contains the caller's cell references and other immutable
        /// transaction inputs. The callback must be pure and repeatable because
        /// a conflicting read set is evaluated again. It cannot suspend or
        /// publish side effects; those belong before or after atomically.
        pub fn atomically(
            comptime A: type,
            comptime E: type,
            comptime R: type,
            comptime Capture: type,
            capture: *const Capture,
            comptime select: fn (*const R) *Self,
            comptime evaluate: fn (*const Capture, *Journal) Decision(A, E),
        ) effects.Effect(A, TransactionError(E), R) {
            const Failure = TransactionError(E);
            return effects.Effect(A, Failure, R).fromNativeCancellableBorrowed(Capture, capture, struct {
                fn run(input: *const Capture, env: *const R, token: ?*const Cancellation) outcomes.Exit(A, Failure) {
                    const domain = select(env);
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
}
