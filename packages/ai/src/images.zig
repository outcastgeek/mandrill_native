//! Explicitly owned compatibility registry. Native libraries do not install a
//! process-global registry or read an ambient I/O backend during import.
const std = @import("std");
const t = @import("types.zig");
const m = @import("models.zig");
pub const ImagesApiProvider = m.ApiImplementation;
pub const Registry = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    arena: std.heap.ArenaAllocator,
    mutex: std.Io.Mutex = .init,
    entries: std.StringArrayHashMapUnmanaged(Entry) = .{},
    const Entry = struct { provider: m.ApiImplementation, sourceId: ?[]const u8 };
    const Registered = struct {
        original: ImagesApiProvider,
        fn generateImages(ctx: ?*anyopaque, operation: m.OperationContext, model: t.Model, context: t.ImagesContext, options: t.ImagesOptions) !t.AssistantImages {
            const self: *@This() = @ptrCast(@alignCast(ctx.?));
            try @import("api/common.zig").requireApi(operation.allocator, model.api, self.original.api, operation.errorMessage orelse options.errorMessage, operation.errorStorage);
            const call = self.original.generateImages orelse return error.MissingImageImplementation;
            return call(self.original.context, operation, model, context, options);
        }
    };
    fn insertLocked(self: *@This(), provider: ImagesApiProvider, source: ?[]const u8) !void {
        const registered = try self.arena.allocator().create(Registered);
        registered.* = .{ .original = provider };
        try self.entries.put(self.allocator, provider.api, .{ .provider = .{ .api = provider.api, .context = registered, .generateImages = Registered.generateImages }, .sourceId = source });
    }
    pub fn init(a: std.mem.Allocator, io: std.Io) !@This() {
        var self = @This(){ .allocator = a, .io = io, .arena = .init(a) };
        errdefer self.deinit();
        try self.registerBuiltInImagesApiProviders();
        return self;
    }
    pub fn deinit(self: *@This()) void {
        self.entries.deinit(self.allocator);
        self.arena.deinit();
        self.* = undefined;
    }
    pub fn registerImagesApiProvider(self: *@This(), provider: ImagesApiProvider, source: ?[]const u8) !void {
        if (provider.generateImages == null) return error.MissingImageImplementation;
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        const a = self.arena.allocator();
        var owned = provider;
        owned.api = try a.dupe(u8, provider.api);
        try self.insertLocked(owned, if (source) |id| try a.dupe(u8, id) else null);
    }
    /// Implementations borrow this registry; original callback contexts remain
    /// caller-owned. Replacement does not invalidate a previously returned view.
    pub fn getImagesApiProvider(self: *@This(), api: []const u8) !?ImagesApiProvider {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        return if (self.entries.get(api)) |entry| entry.provider else null;
    }
    pub fn unregisterImagesApiProviders(self: *@This(), source: []const u8) !void {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        var index: usize = 0;
        while (index < self.entries.count()) {
            const id = self.entries.values()[index].sourceId;
            if (id != null and std.mem.eql(u8, id.?, source)) {
                _ = self.entries.orderedRemove(self.entries.keys()[index]);
            } else index += 1;
        }
    }
    pub fn registerBuiltInImagesApiProviders(self: *@This()) !void {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        try self.insertLocked(@import("api/openrouter_images.zig").implementation(), null);
    }
    pub fn generateImages(self: *@This(), operation: m.OperationContext, model: t.Model, context: t.ImagesContext, options: t.ImagesOptions) !t.AssistantImages {
        const implementation = (try self.getImagesApiProvider(model.api)) orelse {
            try @import("api/common.zig").rememberError(operation.allocator, operation.errorMessage orelse options.errorMessage, operation.errorStorage, "No API provider registered for api: {s}", .{model.api});
            return error.UnsupportedImageApi;
        };
        const call = implementation.generateImages orelse return error.UnsupportedImageApi;
        try @import("api/common.zig").requireApi(operation.allocator, model.api, implementation.api, operation.errorMessage orelse options.errorMessage, operation.errorStorage);
        return call(implementation.context, operation, model, context, options);
    }
};
pub const generateImages = Registry.generateImages;
