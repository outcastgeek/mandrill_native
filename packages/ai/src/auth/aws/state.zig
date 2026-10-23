const std = @import("std");
/// Explicit client-owned token/refresh state and owned background tasks.
pub const State = struct {
    allocator: std.mem.Allocator,
    mutex: std.Io.Mutex = .init,
    lastRefresh: std.StringHashMapUnmanaged(i64) = .{},
    tokens: std.StringHashMapUnmanaged(std.json.Parsed(std.json.Value)) = .{},
    background: std.ArrayList(*Background) = .empty,
    const Background = struct {
        io: std.Io,
        future: std.Io.Future(void),
        context: *anyopaque,
        release: *const fn (*anyopaque) void,
        done: *std.atomic.Value(bool),
    };
    fn reapBackgroundLocked(self: *@This()) void {
        var index: usize = 0;
        while (index < self.background.items.len) {
            const job = self.background.items[index];
            if (job.done.load(.acquire)) {
                _ = self.background.swapRemove(index);
                job.future.await(job.io);
                job.release(job.context);
                self.allocator.destroy(job);
            } else index += 1;
        }
    }
    /// Takes ownership only on success. Inputs retained by the job's opaque
    /// Io/state/auth/transport contexts remain valid until owner deinit.
    /// deinit requires registration callers to be quiescent.
    pub fn retainBackground(self: *@This(), io: std.Io, future: std.Io.Future(void), context: *anyopaque, release: *const fn (*anyopaque) void, done: *std.atomic.Value(bool)) !void {
        const job = try self.allocator.create(Background);
        errdefer self.allocator.destroy(job);
        job.* = .{ .io = io, .future = future, .context = context, .release = release, .done = done };
        try self.mutex.lock(io);
        defer self.mutex.unlock(io);
        self.reapBackgroundLocked();
        try self.background.append(self.allocator, job);
    }
    fn deinitBackground(self: *@This()) void {
        for (self.background.items) |job| {
            job.future.cancel(job.io);
            job.release(job.context);
            self.allocator.destroy(job);
        }
        self.background.deinit(self.allocator);
    }
    pub fn init(a: std.mem.Allocator) @This() {
        return .{ .allocator = a };
    }
    pub fn ssoRefreshReady(self: *@This(), io: std.Io, session: []const u8) !bool {
        try self.mutex.lock(io);
        defer self.mutex.unlock(io);
        const now = std.Io.Clock.real.now(io).toMilliseconds();
        return @as(i128, now) - (self.lastRefresh.get(session) orelse 0) >= 30_000;
    }
    pub fn beginSsoRefresh(self: *@This(), io: std.Io, session: []const u8) !bool {
        try self.mutex.lock(io);
        defer self.mutex.unlock(io);
        const now = std.math.cast(i64, std.Io.Clock.real.now(io).toMilliseconds()) orelse return error.TimestampOutOfRange;
        if (@as(i128, now) - (self.lastRefresh.get(session) orelse 0) < 30_000) return false;
        if (self.lastRefresh.getPtr(session)) |value| value.* = now else {
            const key = try self.allocator.dupe(u8, session);
            errdefer self.allocator.free(key);
            try self.lastRefresh.put(self.allocator, key, now);
        }
        return true;
    }
    /// Owned snapshots replace credential cache rereads after refresh.
    pub fn token(self: *@This(), a: std.mem.Allocator, io: std.Io, key: []const u8) !?std.json.Value {
        try self.mutex.lock(io);
        defer self.mutex.unlock(io);
        return if (self.tokens.get(key)) |value| try @import("../../json/value.zig").clone(a, value.value) else null;
    }
    pub fn putToken(self: *@This(), io: std.Io, key: []const u8, value: std.json.Value) !void {
        const arena = try self.allocator.create(std.heap.ArenaAllocator);
        arena.* = std.heap.ArenaAllocator.init(self.allocator);
        const owned = std.json.Parsed(std.json.Value){ .arena = arena, .value = @import("../../json/value.zig").clone(arena.allocator(), value) catch |err| {
            arena.deinit();
            self.allocator.destroy(arena);
            return err;
        } };
        errdefer owned.deinit();
        try self.mutex.lock(io);
        defer self.mutex.unlock(io);
        if (self.tokens.getPtr(key)) |slot| {
            slot.deinit();
            slot.* = owned;
        } else {
            const name = try self.allocator.dupe(u8, key);
            errdefer self.allocator.free(name);
            try self.tokens.put(self.allocator, name, owned);
        }
    }
    pub fn deinit(self: *@This()) void {
        self.deinitBackground();
        var entries = self.tokens.iterator();
        while (entries.next()) |entry| {
            self.allocator.free(entry.key_ptr.*);
            entry.value_ptr.deinit();
        }
        self.tokens.deinit(self.allocator);
        var keys = self.lastRefresh.keyIterator();
        while (keys.next()) |key| self.allocator.free(key.*);
        self.lastRefresh.deinit(self.allocator);
        self.* = undefined;
    }
};
