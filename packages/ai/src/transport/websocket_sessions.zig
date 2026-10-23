const std = @import("std");
const ws = @import("websocket.zig");
const t = @import("../types.zig");
pub const Continuation = struct { lastRequestBody: std.json.Value, lastResponseId: []const u8, lastResponseItems: std.json.Value };
pub const DebugStats = struct {
    requests: u64 = 0,
    connectionsCreated: u64 = 0,
    connectionsReused: u64 = 0,
    cachedContextRequests: u64 = 0,
    storeTrueRequests: u64 = 0,
    fullContextRequests: u64 = 0,
    deltaRequests: u64 = 0,
    lastInputItems: usize = 0,
    lastDeltaInputItems: ?usize = null,
    lastPreviousResponseId: ?[]const u8 = null,
    websocketFailures: u64 = 0,
    sseFallbacks: u64 = 0,
    websocketFallbackActive: bool = false,
    lastWebSocketError: ?[]const u8 = null,
};
pub const Cache = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    mutex: std.Io.Mutex = .init,
    entries: std.ArrayList(*Entry) = .empty,
    sessions: std.StringHashMapUnmanaged(DebugStats) = .{},
    closing: bool = false,
    activeRequests: usize = 0,
    changed: std.Io.Condition = .init,
    uuidGenerator: @import("../utils/uuid.zig").Generator = .{},
    pub const Entry = struct { session: []const u8, account: []const u8, connection: ws.Connection, socket: *ws.Session, busy: bool = true, closed: bool = false, transient: bool = false, createdAt: i64, idleAt: i64 = 0, continuation: ?Continuation = null, idleTask: ?std.Io.Future(void) = null, owner: *Cache, namesArena: std.heap.ArenaAllocator, continuationArena: std.heap.ArenaAllocator };
    pub fn init(a: std.mem.Allocator, io: std.Io) Cache {
        return .{ .allocator = a, .io = io };
    }
    pub fn deinit(self: *Cache) void {
        const io = self.io;
        const protection = io.swapCancelProtection(.blocked);
        defer _ = io.swapCancelProtection(protection);
        self.mutex.lockUncancelable(self.io);
        self.closing = true;
        for (self.entries.items) |entry| self.closeEntry(entry, "debug_close");
        while (self.activeRequests > 0) self.changed.waitUncancelable(io, &self.mutex);
        self.pruneEntries();
        self.mutex.unlock(self.io);
        var stats_it = self.sessions.iterator();
        while (stats_it.next()) |entry| {
            self.freeStats(entry.value_ptr.*);
            self.allocator.free(entry.key_ptr.*);
        }
        self.entries.deinit(self.allocator);
        self.sessions.deinit(self.allocator);
        self.* = undefined;
    }
    pub fn beginRequest(self: *Cache) !void {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        if (self.closing) return error.SessionCacheClosed;
        self.activeRequests += 1;
    }
    pub fn sessionResource(self: *Cache) @import("../session_resources.zig").SessionResources.Resource {
        return .{ .context = self, .close = closeResource };
    }
    fn closeResource(context: ?*anyopaque, _: std.Io, session: ?[]const u8) !void {
        const self: *Cache = @ptrCast(@alignCast(context.?));
        self.close(session);
    }
    pub fn endRequest(self: *Cache) void {
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        self.activeRequests -= 1;
        self.changed.broadcast(self.io);
    }
    fn closeEntry(self: *Cache, entry: *Entry, reason: []const u8) void {
        if (entry.idleTask) |*task| {
            task.cancel(self.io);
            entry.idleTask = null;
        }
        if (!entry.closed) {
            entry.closed = true;
            entry.continuation = null;
            _ = entry.continuationArena.reset(.free_all);
            entry.socket.close(reason);
            if (!entry.busy) entry.connection.deinit();
        }
    }
    fn pruneEntries(self: *Cache) void {
        var index: usize = 0;
        while (index < self.entries.items.len) {
            const entry = self.entries.items[index];
            if (!entry.closed or entry.busy) {
                index += 1;
                continue;
            }
            if (entry.idleTask) |*task| task.cancel(self.io);
            entry.socket.deinit();
            entry.namesArena.deinit();
            entry.continuationArena.deinit();
            self.allocator.destroy(entry);
            _ = self.entries.orderedRemove(index);
        }
    }
    fn createEntry(self: *Cache, session: []const u8, account: []const u8, connection: ws.Connection, transient: bool) !*Entry {
        const entry = try self.allocator.create(Entry);
        errdefer self.allocator.destroy(entry);
        const socket = try ws.Session.open(self.allocator, self.io, connection);
        errdefer socket.deinit();
        entry.* = .{ .session = "", .account = "", .connection = connection, .socket = socket, .transient = transient, .createdAt = std.Io.Clock.awake.now(self.io).toMilliseconds(), .owner = self, .namesArena = .init(self.allocator), .continuationArena = .init(self.allocator) };
        errdefer entry.namesArena.deinit();
        errdefer entry.continuationArena.deinit();
        entry.session = try entry.namesArena.allocator().dupe(u8, session);
        entry.account = try entry.namesArena.allocator().dupe(u8, account);
        try self.entries.append(self.allocator, entry);
        return entry;
    }
    fn freeStats(self: *Cache, value: DebugStats) void {
        if (value.lastPreviousResponseId) |id| self.allocator.free(id);
        if (value.lastWebSocketError) |message| self.allocator.free(message);
    }
    pub fn close(self: *Cache, session: ?[]const u8) void {
        const protection = self.io.swapCancelProtection(.blocked);
        defer _ = self.io.swapCancelProtection(protection);
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        const selected = if (session) |id| (if (id.len > 0) id else null) else null;
        for (self.entries.items) |entry| if (!entry.transient and (selected == null or std.mem.eql(u8, selected.?, entry.session))) self.closeEntry(entry, "debug_close");
        self.pruneEntries();
    }
    /// Socket cleanup and debug/fallback reset are separate public operations.
    /// An empty session selects every session, matching source truthiness.
    pub fn resetStats(self: *Cache, session: ?[]const u8) void {
        const protection = self.io.swapCancelProtection(.blocked);
        defer _ = self.io.swapCancelProtection(protection);
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        const selected = if (session) |id| (if (id.len > 0) id else null) else null;
        if (selected) |id| {
            if (self.sessions.fetchRemove(id)) |removed| {
                self.freeStats(removed.value);
                self.allocator.free(removed.key);
            }
        } else {
            var it = self.sessions.iterator();
            while (it.next()) |entry| {
                self.freeStats(entry.value_ptr.*);
                self.allocator.free(entry.key_ptr.*);
            }
            self.sessions.clearRetainingCapacity();
        }
    }
    fn expiry(entry: *Entry) void {
        const self = entry.owner;
        const protection = self.io.swapCancelProtection(.unblocked);
        defer _ = self.io.swapCancelProtection(protection);
        self.io.sleep(.fromMilliseconds(300_000), .awake) catch |err| {
            if (err == error.Canceled) self.io.recancel();
            return;
        };
        self.mutex.lock(self.io) catch |err| {
            if (err == error.Canceled) self.io.recancel();
            return;
        };
        defer self.mutex.unlock(self.io);
        if (!entry.busy and !entry.closed) {
            entry.closed = true;
            entry.continuation = null;
            _ = entry.continuationArena.reset(.free_all);
            entry.socket.close("idle_timeout");
            entry.connection.deinit();
        }
    }
    pub fn acquire(self: *Cache, session: []const u8, account: []const u8) !?*Entry {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        if (self.closing) return error.SessionCacheClosed;
        self.pruneEntries();
        const now = std.Io.Clock.awake.now(self.io).toMilliseconds();
        for (self.entries.items) |entry| {
            if (entry.closed or entry.transient or !std.mem.eql(u8, entry.session, session) or !std.mem.eql(u8, entry.account, account)) continue;
            if (entry.busy) return null;
            if (!entry.socket.isOpen() or now - entry.createdAt >= 3_300_000 or now - entry.idleAt >= 300_000) {
                self.closeEntry(entry, if (now - entry.createdAt >= 3_300_000) "connection_age_limit" else if (now - entry.idleAt >= 300_000) "idle_timeout" else "done");
                continue;
            }
            if (entry.idleTask) |*task| {
                task.cancel(self.io);
                entry.idleTask = null;
            }
            entry.busy = true;
            entry.socket.setActive(true);
            return entry;
        }
        self.pruneEntries();
        return null;
    }
    pub fn insert(self: *Cache, session: []const u8, account: []const u8, connection: ws.Connection) !?*Entry {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        if (self.closing) return error.SessionCacheClosed;
        self.pruneEntries();
        for (self.entries.items) |entry| if (!entry.closed and !entry.transient and std.mem.eql(u8, entry.session, session) and std.mem.eql(u8, entry.account, account)) return null;
        return self.createEntry(session, account, connection, false);
    }
    /// Teardown also owns uncached sockets created while a cached lease is busy.
    /// They never participate in continuation or connection reuse.
    pub fn trackTransient(self: *Cache, connection: ws.Connection) !*Entry {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        if (self.closing) return error.SessionCacheClosed;
        self.pruneEntries();
        return self.createEntry("", "", connection, true);
    }
    pub fn release(self: *Cache, entry: *Entry, keep: bool) void {
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        if (entry.closed) {
            entry.busy = false;
            entry.connection.deinit();
            self.pruneEntries();
            return;
        }
        if (!keep or self.closing or entry.transient) {
            entry.busy = false;
            self.closeEntry(entry, "done");
            self.pruneEntries();
            return;
        }
        if (!entry.socket.isOpen()) {
            entry.busy = false;
            self.closeEntry(entry, "done");
            self.pruneEntries();
            return;
        }
        entry.socket.setActive(false);
        entry.busy = false;
        entry.idleAt = std.Io.Clock.awake.now(self.io).toMilliseconds();
        entry.idleTask = self.io.concurrent(expiry, .{entry}) catch {
            self.closeEntry(entry, "done");
            self.pruneEntries();
            return;
        };
    }
    pub fn setContinuation(self: *Cache, entry: *Entry, value: Continuation) !void {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        if (entry.closed) return;
        var replacement = std.heap.ArenaAllocator.init(self.allocator);
        errdefer replacement.deinit();
        const copied = try t.cloneInto(Continuation, replacement.allocator(), value);
        entry.continuationArena.deinit();
        entry.continuationArena = replacement;
        entry.continuation = copied;
    }
    pub fn copyContinuation(self: *Cache, entry: *Entry, a: std.mem.Allocator) !?Continuation {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        return if (entry.continuation) |value| try t.cloneInto(Continuation, a, value) else null;
    }
    pub fn clearContinuation(self: *Cache, entry: *Entry) !void {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        entry.continuation = null;
        _ = entry.continuationArena.reset(.free_all);
    }
    fn statsPointer(self: *Cache, session: []const u8) !*DebugStats {
        if (self.sessions.getPtr(session)) |value| return value;
        const key = try self.allocator.dupe(u8, session);
        errdefer self.allocator.free(key);
        try self.sessions.put(self.allocator, key, .{});
        return self.sessions.getPtr(key).?;
    }
    pub fn stats(self: *Cache, a: std.mem.Allocator, session: []const u8) !?DebugStats {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        return if (self.sessions.get(session)) |value| try t.cloneInto(DebugStats, a, value) else null;
    }
    pub fn fallbackActive(self: *Cache, session: []const u8) !bool {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        return if (self.sessions.get(session)) |value| value.websocketFallbackActive else false;
    }
    pub fn failure(self: *Cache, session: []const u8, message: []const u8) !void {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        const value = try self.statsPointer(session);
        const copied = try self.allocator.dupe(u8, message);
        if (value.lastWebSocketError) |old| self.allocator.free(old);
        value.websocketFailures += 1;
        value.lastWebSocketError = copied;
    }
    pub fn fallback(self: *Cache, session: []const u8) !void {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        const value = try self.statsPointer(session);
        value.sseFallbacks += 1;
        value.websocketFallbackActive = true;
    }
    pub fn requested(self: *Cache, session: []const u8, reused: bool, cached: bool, count: usize, previous: ?[]const u8) !void {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        const value = try self.statsPointer(session);
        value.requests += 1;
        if (reused) value.connectionsReused += 1 else value.connectionsCreated += 1;
        if (cached) value.cachedContextRequests += 1;
        const copied = if (previous) |id| try self.allocator.dupe(u8, id) else null;
        if (value.lastPreviousResponseId) |old| self.allocator.free(old);
        value.lastInputItems = count;
        value.lastDeltaInputItems = if (previous != null) count else null;
        value.lastPreviousResponseId = copied;
        if (previous != null) value.deltaRequests += 1 else value.fullContextRequests += 1;
    }
};
