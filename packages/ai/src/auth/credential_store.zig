const std = @import("std");
const t = @import("types.zig");
const Owned = @import("../types.zig").Owned;
pub const InMemoryCredentialStore = struct {
    allocator: std.mem.Allocator,
    mutex: std.Io.Mutex = .init,
    entries: std.StringArrayHashMapUnmanaged(*Entry) = .{},
    // Lock entries survive deletion. Committed metadata has its own Map order,
    // so a deleted/re-added credential goes to the end without replacing locks.
    active: std.StringArrayHashMapUnmanaged(t.CredentialInfo) = .{},
    const Entry = struct { id: []const u8, mutex: std.Io.Mutex = .init, credential: ?Owned(t.Credential) = null };
    pub fn init(a: std.mem.Allocator) @This() {
        return .{ .allocator = a };
    }
    pub fn deinit(self: *@This()) void {
        self.active.deinit(self.allocator);
        for (self.entries.keys(), self.entries.values()) |key, entry| {
            self.allocator.free(key);
            if (entry.credential) |*c| c.deinit();
            self.allocator.destroy(entry);
        }
        self.entries.deinit(self.allocator);
        self.* = undefined;
    }
    pub fn store(self: *@This()) t.CredentialStore {
        return .{ .context = self, .readFn = read, .listFn = list, .modifyFn = modify, .deleteFn = delete };
    }
    fn lookupEntry(self: *@This(), io: std.Io, id: []const u8) !*Entry {
        try self.mutex.lock(io);
        defer self.mutex.unlock(io);
        if (self.entries.get(id)) |e| return e;
        const key = try self.allocator.dupe(u8, id);
        errdefer self.allocator.free(key);
        const e = try self.allocator.create(Entry);
        errdefer self.allocator.destroy(e);
        e.* = .{ .id = key };
        try self.entries.put(self.allocator, key, e);
        return e;
    }
    fn copy(a: std.mem.Allocator, credential: t.Credential) !t.Credential {
        return @import("../types.zig").cloneInto(t.Credential, a, credential);
    }
    fn read(ctx: *anyopaque, a: std.mem.Allocator, io: std.Io, id: []const u8) !?t.Credential {
        try io.checkCancel();
        const self: *@This() = @ptrCast(@alignCast(ctx));
        try self.mutex.lock(io);
        defer self.mutex.unlock(io);
        try io.checkCancel();
        const e = self.entries.get(id) orelse return null;
        return if (e.credential) |c| try copy(a, c.value()) else null;
    }
    fn list(ctx: *anyopaque, a: std.mem.Allocator, io: std.Io) ![]const t.CredentialInfo {
        const self: *@This() = @ptrCast(@alignCast(ctx));
        try self.mutex.lock(io);
        defer self.mutex.unlock(io);
        try io.checkCancel();
        const result = try a.alloc(t.CredentialInfo, self.active.count());
        var initialized: usize = 0;
        errdefer {
            for (result[0..initialized]) |info| a.free(info.providerId);
            a.free(result);
        }
        for (self.active.values(), result) |info, *out| {
            out.* = .{ .providerId = try a.dupe(u8, info.providerId), .type = info.type };
            initialized += 1;
        }
        return result;
    }
    fn modify(ctx: *anyopaque, a: std.mem.Allocator, io: std.Io, id: []const u8, mutation: t.CredentialStore.Mutation) !?t.Credential {
        const self: *@This() = @ptrCast(@alignCast(ctx));
        const e = try self.lookupEntry(io, id);
        try e.mutex.lock(io);
        defer e.mutex.unlock(io);
        try io.checkCancel();
        const protection = io.swapCancelProtection(if (mutation.completeOnCancel) .blocked else .unblocked);
        defer _ = io.swapCancelProtection(protection);
        var arena = std.heap.ArenaAllocator.init(a);
        defer arena.deinit();
        // Native callbacks receive an operation-owned snapshot, so a callback
        // cannot mutate a committed tree while status readers clone it.
        const current = if (e.credential) |c| try copy(arena.allocator(), c.value()) else null;
        const next = try mutation.call(mutation.context, arena.allocator(), io, current);
        try io.checkCancel();
        if (next) |c| {
            // Prepare both detached copies before committing. A failure to
            // allocate the caller's result must not replace the stored token.
            const result = try copy(a, c);
            try self.mutex.lock(io);
            defer self.mutex.unlock(io);
            try io.checkCancel();
            var owned = try Owned(t.Credential).clone(self.allocator, c);
            errdefer owned.deinit();
            try io.checkCancel();
            try self.active.put(self.allocator, e.id, .{ .providerId = e.id, .type = std.meta.activeTag(c) });
            if (e.credential) |*old| old.deinit();
            e.credential = owned;
            return result;
        }
        return if (e.credential) |c| try copy(a, c.value()) else null;
    }
    fn delete(ctx: *anyopaque, io: std.Io, id: []const u8) !void {
        const self: *@This() = @ptrCast(@alignCast(ctx));
        const e = try self.lookupEntry(io, id);
        try e.mutex.lock(io);
        defer e.mutex.unlock(io);
        try io.checkCancel();
        try self.mutex.lock(io);
        defer self.mutex.unlock(io);
        try io.checkCancel();
        _ = self.active.orderedRemove(id);
        if (e.credential) |*c| c.deinit();
        e.credential = null;
    }
};
