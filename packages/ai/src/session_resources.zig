const std = @import("std");
pub const SessionResourceCleanup = SessionResources.Resource;
/// An unregister handle borrows the stable registry and callback context.
pub const SessionResourceRegistration = struct {
    registry: *SessionResources,
    cleanup: SessionResourceCleanup,
    pub fn unregister(self: @This()) void {
        self.registry.unregister(self.cleanup);
    }
};
pub fn registerSessionResourceCleanup(registry: *SessionResources, cleanup: SessionResourceCleanup) !SessionResourceRegistration {
    try registry.register(cleanup);
    return .{ .registry = registry, .cleanup = cleanup };
}
/// The result aggregates every callback failure. Free the outer slice with
/// registry.allocator. The caller synchronizes access, as for registry.close.
pub fn cleanupSessionResources(registry: *SessionResources, io: std.Io, session: ?[]const u8) ![]const anyerror {
    return registry.close(io, session);
}
pub const SessionResources = struct {
    // The owner synchronizes concurrent registry access. Callbacks may mutate
    // this registry reentrantly; the cursor observes additions in insertion order.
    allocator: std.mem.Allocator,
    entries: std.ArrayList(Entry) = .empty,
    closeDepth: usize = 0,
    pub const Resource = struct { context: ?*anyopaque, close: *const fn (?*anyopaque, std.Io, ?[]const u8) anyerror!void };
    const Entry = struct { resource: Resource, active: bool = true };
    pub fn init(a: std.mem.Allocator) @This() {
        return .{ .allocator = a };
    }
    pub fn register(self: *@This(), resource: Resource) !void {
        for (self.entries.items) |entry| if (entry.active and entry.resource.context == resource.context and entry.resource.close == resource.close) return;
        try self.entries.append(self.allocator, .{ .resource = resource });
    }
    pub fn unregister(self: *@This(), resource: Resource) void {
        for (self.entries.items, 0..) |entry, index| if (entry.active and entry.resource.context == resource.context and entry.resource.close == resource.close) {
            if (self.closeDepth > 0) self.entries.items[index].active = false else _ = self.entries.orderedRemove(index);
            return;
        };
    }
    pub fn close(self: *@This(), io: std.Io, session: ?[]const u8) ![]const anyerror {
        var errors: std.ArrayList(anyerror) = .empty;
        errdefer errors.deinit(self.allocator);
        self.closeDepth += 1;
        defer {
            self.closeDepth -= 1;
            if (self.closeDepth == 0) {
                var index: usize = 0;
                while (index < self.entries.items.len) {
                    if (!self.entries.items[index].active) _ = self.entries.orderedRemove(index) else index += 1;
                }
            }
        }
        var index: usize = 0;
        while (index < self.entries.items.len) {
            const entry = self.entries.items[index];
            index += 1;
            if (!entry.active) continue;
            entry.resource.close(entry.resource.context, io, session) catch |err| {
                if (err == error.Canceled) io.recancel();
                try errors.append(self.allocator, err);
            };
        }
        return errors.toOwnedSlice(self.allocator);
    }
    pub fn deinit(self: *@This()) void {
        self.entries.deinit(self.allocator);
        self.* = undefined;
    }
};
