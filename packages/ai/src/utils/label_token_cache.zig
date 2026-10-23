const std = @import("std");
pub const Cache = struct {
    allocator: std.mem.Allocator,
    mutex: std.Io.Mutex = .init,
    entries: std.StringHashMapUnmanaged(*Entry) = .{},
    pub const FailureDetails = struct {
        status: ?u16 = null,
        errorBody: ?[]const u8 = null,
        message: ?[]const u8 = null,
        fn clone(self: @This(), a: std.mem.Allocator) !@This() {
            const body = if (self.errorBody) |value| try a.dupe(u8, value) else null;
            errdefer if (body) |value| a.free(value);
            return .{ .status = self.status, .errorBody = body, .message = if (self.message) |value| try a.dupe(u8, value) else null };
        }
        pub fn deinit(self: *@This(), a: std.mem.Allocator) void {
            if (self.errorBody) |value| a.free(value);
            if (self.message) |value| a.free(value);
            self.* = .{};
        }
    };
    const Entry = struct {
        key: []const u8,
        users: usize = 0,
        retained: bool = true,
        mutex: std.Io.Mutex = .init,
        loaded: bool = false,
        token: ?f64 = null,
        failure: ?anyerror = null,
        details: FailureDetails = .{},
    };
    const Load = *const fn (?*anyopaque, *FailureDetails) anyerror!?f64;
    const FailureOutput = struct { allocator: std.mem.Allocator, value: *FailureDetails };
    pub fn init(a: std.mem.Allocator) @This() {
        return .{ .allocator = a };
    }
    /// Like other transport-owned services, release active request handles first.
    pub fn deinit(self: *@This()) void {
        var entries = self.entries.valueIterator();
        while (entries.next()) |entry| self.destroyEntry(entry.*);
        self.entries.deinit(self.allocator);
        self.* = undefined;
    }
    fn destroyEntry(self: *@This(), entry: *Entry) void {
        entry.details.deinit(self.allocator);
        self.allocator.free(entry.key);
        self.allocator.destroy(entry);
    }
    fn release(self: *@This(), io: std.Io, entry: *Entry) void {
        self.mutex.lockUncancelable(io);
        defer self.mutex.unlock(io);
        entry.users -= 1;
        if (!entry.retained and entry.users == 0) self.destroyEntry(entry);
    }
    pub fn resolve(self: *@This(), io: std.Io, key: []const u8, context: ?*anyopaque, load: *const fn (?*anyopaque) anyerror!?f64) !?f64 {
        var legacy = struct {
            context: ?*anyopaque,
            load: *const fn (?*anyopaque) anyerror!?f64,
            fn call(ctx: ?*anyopaque, _: *FailureDetails) !?f64 {
                const self_: *@This() = @ptrCast(@alignCast(ctx.?));
                return self_.load(self_.context);
            }
        }{ .context = context, .load = load };
        return self.resolveInternal(io, key, &legacy, @TypeOf(legacy).call, null);
    }
    /// Loader details may borrow its operation. Cached failures and each
    /// returned failure detail get independent copies on their owners. Release
    /// returned details with deinit(a), or allocate them in an operation arena.
    pub fn resolveWithFailure(self: *@This(), a: std.mem.Allocator, io: std.Io, key: []const u8, context: ?*anyopaque, load: Load, details: *FailureDetails) !?f64 {
        details.* = .{};
        return self.resolveInternal(io, key, context, load, .{ .allocator = a, .value = details });
    }
    fn resolveInternal(self: *@This(), io: std.Io, key: []const u8, context: ?*anyopaque, load: Load, output: ?FailureOutput) !?f64 {
        const entry = blk: {
            try self.mutex.lock(io);
            defer self.mutex.unlock(io);
            if (self.entries.get(key)) |entry| {
                entry.users += 1;
                break :blk entry;
            }
            const owned_key = try self.allocator.dupe(u8, key);
            errdefer self.allocator.free(owned_key);
            const entry = try self.allocator.create(Entry);
            errdefer self.allocator.destroy(entry);
            entry.* = .{ .key = owned_key, .users = 1 };
            try self.entries.put(self.allocator, owned_key, entry);
            break :blk entry;
        };
        defer self.release(io, entry);
        try entry.mutex.lock(io);
        defer entry.mutex.unlock(io);
        if (!entry.loaded and entry.failure == null) {
            var details = FailureDetails{};
            const token: ?f64 = load(context, &details) catch |err| failed: {
                self.mutex.lockUncancelable(io);
                defer self.mutex.unlock(io);
                // Existing waiters retain this attempt. A later lookup starts
                // a fresh one, as with the source's evicted rejected promise.
                _ = self.entries.remove(entry.key);
                entry.retained = false;
                entry.failure = err;
                entry.details = details.clone(self.allocator) catch |copy_err| {
                    entry.failure = copy_err;
                    break :failed null;
                };
                break :failed null;
            };
            if (entry.failure == null) {
                entry.token = token;
                entry.loaded = true;
            }
        }
        if (entry.failure) |err| {
            if (output) |target| target.value.* = try entry.details.clone(target.allocator);
            return err;
        }
        return entry.token;
    }
};
