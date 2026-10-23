const std = @import("std");
const t = @import("types.zig");
const j = @import("json/value.zig");
/// Store cancellation uses each method's explicit I/O argument.
pub const ModelsStoreOperationOptions = std.Io;
pub const ModelsStoreEntry = struct {
    models: []const t.Model,
    lastModified: ?f64 = null,
    checkedAt: ?f64 = null,
    etag: ?[]const u8 = null,
    jsonFields: std.json.Value = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
/// The host retains this context through Models teardown. Read results belong
/// to the supplied allocation region; write inputs borrow their caller through
/// return. The default store clones both retained writes and returned reads.
pub const ModelsStore = struct {
    context: *anyopaque,
    readFn: *const fn (*anyopaque, std.mem.Allocator, std.Io, []const u8) anyerror!?ModelsStoreEntry,
    writeFn: *const fn (*anyopaque, std.Io, []const u8, ModelsStoreEntry) anyerror!void,
    deleteFn: *const fn (*anyopaque, std.Io, []const u8) anyerror!void,
    pub fn read(self: @This(), a: std.mem.Allocator, io: std.Io, id: []const u8) !?ModelsStoreEntry {
        return self.readFn(self.context, a, io, id);
    }
    pub fn write(self: @This(), io: std.Io, id: []const u8, value: ModelsStoreEntry) !void {
        return self.writeFn(self.context, io, id, value);
    }
    pub fn delete(self: @This(), io: std.Io, id: []const u8) !void {
        return self.deleteFn(self.context, io, id);
    }
};
pub const InMemoryModelsStore = struct {
    allocator: std.mem.Allocator,
    mutex: std.Io.Mutex = .init,
    entries: std.StringHashMapUnmanaged(t.Owned(ModelsStoreEntry)) = .{},
    pub fn init(a: std.mem.Allocator) @This() {
        return .{ .allocator = a };
    }
    pub fn store(self: *@This()) ModelsStore {
        return .{ .context = self, .readFn = read, .writeFn = write, .deleteFn = delete };
    }
    pub fn deinit(self: *@This()) void {
        var it = self.entries.iterator();
        while (it.next()) |entry| {
            self.allocator.free(entry.key_ptr.*);
            entry.value_ptr.deinit();
        }
        self.entries.deinit(self.allocator);
        self.* = undefined;
    }
    fn read(ctx: *anyopaque, a: std.mem.Allocator, io: std.Io, id: []const u8) !?ModelsStoreEntry {
        const self: *@This() = @ptrCast(@alignCast(ctx));
        try self.mutex.lock(io);
        defer self.mutex.unlock(io);
        try io.checkCancel();
        const value = self.entries.get(id) orelse return null;
        return try t.cloneInto(ModelsStoreEntry, a, value.value());
    }
    fn write(ctx: *anyopaque, io: std.Io, id: []const u8, entry: ModelsStoreEntry) !void {
        const self: *@This() = @ptrCast(@alignCast(ctx));
        try io.checkCancel();
        try self.mutex.lock(io);
        defer self.mutex.unlock(io);
        try io.checkCancel();
        var value = try t.Owned(ModelsStoreEntry).clone(self.allocator, entry);
        errdefer value.deinit();
        try io.checkCancel();
        const key = try self.allocator.dupe(u8, id);
        errdefer self.allocator.free(key);
        const gop = try self.entries.getOrPut(self.allocator, key);
        if (gop.found_existing) {
            self.allocator.free(key);
            gop.value_ptr.deinit();
        }
        gop.value_ptr.* = value;
    }
    fn delete(ctx: *anyopaque, io: std.Io, id: []const u8) !void {
        const self: *@This() = @ptrCast(@alignCast(ctx));
        try self.mutex.lock(io);
        defer self.mutex.unlock(io);
        try io.checkCancel();
        if (self.entries.fetchRemove(id)) |removed| {
            self.allocator.free(removed.key);
            var value = removed.value;
            value.deinit();
        }
    }
};
