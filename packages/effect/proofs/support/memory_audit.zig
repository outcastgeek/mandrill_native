//! Thread-safe test instrumentation backed by Zig's leak-detecting allocator.
const std = @import("std");
pub const MemoryAudit = struct {
    backing: std.mem.Allocator = std.testing.allocator,
    mutex: std.Io.Mutex = .init,
    live: usize = 0,
    bytes: usize = 0,
    peak_live: usize = 0,
    peak_bytes: usize = 0,
    allocations: usize = 0,
    pub fn allocator(self: *@This()) std.mem.Allocator {
        return .{ .ptr = self, .vtable = &.{ .alloc = alloc, .resize = resize, .remap = remap, .free = free } };
    }
    fn record(self: *@This()) void {
        self.peak_live = @max(self.peak_live, self.live);
        self.peak_bytes = @max(self.peak_bytes, self.bytes);
    }
    fn alloc(data: *anyopaque, len: usize, alignment: std.mem.Alignment, address: usize) ?[*]u8 {
        const self: *@This() = @ptrCast(@alignCast(data));
        self.mutex.lockUncancelable(std.testing.io);
        defer self.mutex.unlock(std.testing.io);
        const result = self.backing.rawAlloc(len, alignment, address) orelse return null;
        self.live += 1;
        self.bytes += len;
        self.allocations += 1;
        self.record();
        return result;
    }
    fn resize(data: *anyopaque, memory: []u8, alignment: std.mem.Alignment, len: usize, address: usize) bool {
        const self: *@This() = @ptrCast(@alignCast(data));
        self.mutex.lockUncancelable(std.testing.io);
        defer self.mutex.unlock(std.testing.io);
        if (!self.backing.rawResize(memory, alignment, len, address)) return false;
        self.bytes = self.bytes - memory.len + len;
        self.record();
        return true;
    }
    fn remap(data: *anyopaque, memory: []u8, alignment: std.mem.Alignment, len: usize, address: usize) ?[*]u8 {
        const self: *@This() = @ptrCast(@alignCast(data));
        self.mutex.lockUncancelable(std.testing.io);
        defer self.mutex.unlock(std.testing.io);
        const result = self.backing.rawRemap(memory, alignment, len, address) orelse return null;
        self.bytes = self.bytes - memory.len + len;
        self.record();
        return result;
    }
    fn free(data: *anyopaque, memory: []u8, alignment: std.mem.Alignment, address: usize) void {
        const self: *@This() = @ptrCast(@alignCast(data));
        self.mutex.lockUncancelable(std.testing.io);
        defer self.mutex.unlock(std.testing.io);
        self.backing.rawFree(memory, alignment, address);
        self.live -= 1;
        self.bytes -= memory.len;
    }
};
