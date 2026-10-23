const std = @import("std");
const t = @import("../types.zig");
/// Extension-owned stream with the pinned generic completion/result callbacks.
/// Published values borrow this stream until deinit. Callbacks run while its
/// mutex is held and must not reenter the stream. The caller owns producers
/// and joins them and any consumers before deinit.
pub fn EventStream(comptime E: type, comptime R: type) type {
    return struct {
        allocator: std.mem.Allocator,
        io: std.Io,
        arena: std.heap.ArenaAllocator,
        mutex: std.Io.Mutex = .init,
        changed: std.Io.Condition = .init,
        queue: std.ArrayList(E) = .empty,
        cursor: usize = 0,
        done: bool = false,
        final: R = undefined,
        resultReady: bool = false,
        failure: ?anyerror = null,
        callbacks: Callbacks,
        pub const Callbacks = struct {
            context: ?*anyopaque = null,
            isComplete: *const fn (?*anyopaque, E) bool,
            extractResult: *const fn (?*anyopaque, E) R,
        };
        pub fn init(a: std.mem.Allocator, io: std.Io, callbacks: Callbacks) @This() {
            return .{ .allocator = a, .io = io, .arena = .init(a), .callbacks = callbacks };
        }
        pub fn create(a: std.mem.Allocator, io: std.Io, callbacks: Callbacks) !*@This() {
            const self = try a.create(@This());
            self.* = .init(a, io, callbacks);
            return self;
        }
        pub fn deinit(self: *@This()) void {
            self.queue.deinit(self.allocator);
            self.arena.deinit();
            self.* = undefined;
        }
        pub fn destroy(self: *@This()) void {
            const a = self.allocator;
            self.deinit();
            a.destroy(self);
        }
        fn failLocked(self: *@This(), err: anyerror) void {
            if (self.resultReady or self.failure != null) return;
            self.failure = err;
            self.done = true;
            self.changed.broadcast(self.io);
        }
        pub fn fail(self: *@This(), err: anyerror) void {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            self.failLocked(err);
        }
        pub fn push(self: *@This(), event: E) !void {
            try self.mutex.lock(self.io);
            defer self.mutex.unlock(self.io);
            if (self.done) return;
            const owned = t.cloneInto(E, self.arena.allocator(), event) catch |err| {
                self.failLocked(err);
                return err;
            };
            const complete = self.callbacks.isComplete(self.callbacks.context, owned);
            const extracted_result: R = if (complete) t.cloneInto(R, self.arena.allocator(), self.callbacks.extractResult(self.callbacks.context, owned)) catch |err| {
                self.failLocked(err);
                return err;
            } else undefined;
            self.queue.append(self.allocator, owned) catch |err| {
                self.failLocked(err);
                return err;
            };
            if (complete) {
                self.final = extracted_result;
                self.resultReady = true;
                self.done = true;
            }
            self.changed.broadcast(self.io);
        }
        pub fn end(self: *@This(), final_result: ?R) !void {
            try self.mutex.lock(self.io);
            defer self.mutex.unlock(self.io);
            // end() closes iteration; a later explicit result can still settle
            // the source's separate final-result promise.
            if (final_result) |value| if (!self.resultReady and self.failure == null) {
                self.final = t.cloneInto(R, self.arena.allocator(), value) catch |err| {
                    self.failure = err;
                    self.done = true;
                    self.changed.broadcast(self.io);
                    return err;
                };
                self.resultReady = true;
            };
            self.done = true;
            self.changed.broadcast(self.io);
        }
        pub fn next(self: *@This()) !?E {
            try self.mutex.lock(self.io);
            defer self.mutex.unlock(self.io);
            while (self.cursor == self.queue.items.len and !self.done) try self.changed.wait(self.io, &self.mutex);
            if (self.cursor < self.queue.items.len) {
                const event = self.queue.items[self.cursor];
                self.cursor += 1;
                return event;
            }
            if (self.failure) |err| return err;
            return null;
        }
        pub fn result(self: *@This()) !R {
            try self.mutex.lock(self.io);
            defer self.mutex.unlock(self.io);
            while (!self.resultReady and self.failure == null) try self.changed.wait(self.io, &self.mutex);
            if (self.failure) |err| return err;
            return self.final;
        }
    };
}
/// Growable publication queue: completion never waits for an iterator to drain.
/// Returned events/messages borrow this handle until deinit. A single owner calls
/// deinit, awaitProducer, or cancel; next/result may have concurrent waiters.
pub const AssistantMessageEventStream = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    arena: std.heap.ArenaAllocator,
    mutex: std.Io.Mutex = .init,
    changed: std.Io.Condition = .init,
    events: std.ArrayList(t.AssistantMessageEvent) = .empty,
    cursor: usize = 0,
    done: bool = false,
    final: ?t.AssistantMessage = null,
    last_partial: ?t.AssistantMessage = null,
    emergency_base: ?t.AssistantMessage = null,
    emergency: ?t.AssistantMessageEvent = null,
    emergency_delivered: bool = false,
    failure: ?anyerror = null,
    started_wall: i64,
    started_mono: std.Io.Timestamp,
    producer: ?std.Io.Future(void) = null,

    pub fn init(a: std.mem.Allocator, io: std.Io) @This() {
        return .{ .allocator = a, .io = io, .arena = .init(a), .started_wall = std.Io.Clock.real.now(io).toMilliseconds(), .started_mono = std.Io.Clock.awake.now(io) };
    }
    pub fn create(a: std.mem.Allocator, io: std.Io) !*@This() {
        const self = try a.create(@This());
        self.* = .init(a, io);
        return self;
    }
    /// Call before starting a producer. These owned identifiers make the first
    /// publication's allocation failure reportable without borrowing its input.
    pub fn prepareModel(self: *@This(), model: t.Model) !void {
        if (self.producer != null or self.last_partial != null or self.done) return error.StreamAlreadyStarted;
        const a = self.arena.allocator();
        const base = t.AssistantMessage{ .api = try a.dupe(u8, model.api), .provider = try a.dupe(u8, model.provider), .model = try a.dupe(u8, model.id), .timestamp = @floatFromInt(self.started_wall) };
        try self.events.ensureTotalCapacity(self.allocator, 1);
        self.emergency_base = base;
    }
    pub fn destroy(self: *@This()) void {
        const a = self.allocator;
        self.deinit();
        a.destroy(self);
    }
    /// Single-owner operation: cancel and join while retaining published output.
    pub fn cancel(self: *@This()) void {
        if (self.producer) |*future| {
            future.cancel(self.io);
            self.producer = null;
        }
        self.fail(error.Canceled);
    }
    /// Single-owner operation: join the producer without consuming any events.
    pub fn awaitProducer(self: *@This()) void {
        if (self.producer) |*future| {
            future.await(self.io);
            self.producer = null;
        }
        self.fail(error.MissingFinalResult);
    }
    pub fn deinit(self: *@This()) void {
        if (self.producer) |*future| {
            future.cancel(self.io);
            self.producer = null;
        }
        self.events.deinit(self.allocator);
        self.arena.deinit();
        self.* = undefined;
    }
    fn time(self: *@This(), message: *t.AssistantMessage) void {
        if (!@import("../json/value.zig").fieldPresent(message.*, "durationMs") and !(message.timestamp < @as(f64, @floatFromInt(self.started_wall)))) {
            const elapsed = self.started_mono.durationTo(std.Io.Clock.awake.now(self.io)).toNanoseconds();
            message.durationMs = @floatFromInt(@divFloor(@max(0, elapsed) +| (@as(i96, std.time.ns_per_ms) / 2), std.time.ns_per_ms));
            @import("../json/value.zig").markPresent(message, "durationMs");
        }
    }
    fn emergencyLocked(self: *@This(), err: anyerror, owned_fallback: ?t.AssistantMessage) void {
        if (owned_fallback orelse self.last_partial orelse self.emergency_base) |partial| {
            var message = partial;
            message.stopReason = if (err == error.Canceled) .aborted else .@"error";
            message.errorMessage = @errorName(err);
            self.time(&message);
            self.final = message;
            self.emergency = .{ .@"error" = .{ .reason = message.stopReason, .@"error" = message } };
        } else self.failure = err;
        self.done = true;
        self.changed.broadcast(self.io);
    }
    pub fn latestPartial(self: *@This()) ?t.AssistantMessage {
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        return self.last_partial;
    }
    pub fn push(self: *@This(), original: t.AssistantMessageEvent) !void {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        if (self.done) return;
        // Deep clone at publication; a network/reducer buffer cannot corrupt history.
        var event = t.cloneInto(t.AssistantMessageEvent, self.arena.allocator(), original) catch |err| {
            self.emergencyLocked(err, null);
            return err;
        };
        switch (event) {
            .done => |*v| {
                self.time(&v.message);
                self.final = v.message;
                self.done = true;
            },
            .@"error" => |*v| {
                self.time(&v.@"error");
                self.final = v.@"error";
                self.done = true;
            },
            inline else => |value| self.last_partial = value.partial,
        }
        const fallback: t.AssistantMessage = switch (event) {
            .done => |value| value.message,
            .@"error" => |value| value.@"error",
            inline else => |value| value.partial,
        };
        self.events.append(self.allocator, event) catch |err| {
            self.emergencyLocked(err, fallback);
            return err;
        };
        self.changed.broadcast(self.io);
    }
    pub fn fail(self: *@This(), err: anyerror) void {
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        if (self.final != null or self.failure != null) return;
        if (self.done) {
            // Iteration may already have been closed by end(null). A native
            // producer failure still settles the independently pending result.
            self.failure = err;
            self.changed.broadcast(self.io);
            return;
        }
        self.emergencyLocked(err, null);
    }
    pub fn end(self: *@This(), message: ?t.AssistantMessage) !void {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        // Closing iteration does not resolve the source's result promise.
        // A later end(message) may supply it, including after end(null).
        if (message) |value| if (self.final == null and self.failure == null) {
            var final = t.cloneInto(t.AssistantMessage, self.arena.allocator(), value) catch |err| {
                self.emergencyLocked(err, null);
                return err;
            };
            if (!self.done) self.time(&final);
            self.final = final;
        };
        self.done = true;
        self.changed.broadcast(self.io);
    }
    pub fn next(self: *@This()) !?t.AssistantMessageEvent {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        while (self.cursor == self.events.items.len and !self.done) try self.changed.wait(self.io, &self.mutex);
        if (self.cursor < self.events.items.len) {
            const event = self.events.items[self.cursor];
            self.cursor += 1;
            return event;
        }
        if (self.emergency) |event| if (!self.emergency_delivered) {
            self.emergency_delivered = true;
            return event;
        };
        if (self.failure) |err| return err;
        return null;
    }
    pub fn result(self: *@This()) !t.AssistantMessage {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        while (self.final == null and self.failure == null) try self.changed.wait(self.io, &self.mutex);
        if (self.failure) |err| return err;
        return self.final.?;
    }
};
pub fn createAssistantMessageEventStream(a: std.mem.Allocator, io: std.Io) !*AssistantMessageEventStream {
    return AssistantMessageEventStream.create(a, io);
}
