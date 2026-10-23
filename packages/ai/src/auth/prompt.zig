//! Owned native prompt operations for hosts whose prompts can outlive a login.
const std = @import("std");
const j = @import("../json/value.zig");

/// A single waiting reference. Waiters must join before deinit. The producer
/// retains its independent reference and storage until it finishes.
pub const PromptOperation = struct {
    context: *anyopaque,
    waitFn: *const fn (*anyopaque, std.mem.Allocator, std.Io) anyerror![]const u8,
    cancelFn: *const fn (*anyopaque) void,
    releaseFn: *const fn (*anyopaque) void,
    pub fn wait(self: @This(), a: std.mem.Allocator, io: std.Io) ![]const u8 {
        return self.waitFn(self.context, a, io);
    }
    /// Requests cancellation without joining the prompt producer.
    pub fn cancel(self: @This()) void {
        self.cancelFn(self.context);
    }
    pub fn deinit(self: *@This()) void {
        self.cancel();
        self.releaseFn(self.context);
        self.* = undefined;
    }
};

/// Create this owner outside the login operation. Its allocator, Io backend
/// and retained host context live through deinit; the allocator supports
/// concurrent calls. Stop all login/waiter calls before deinit. Shutdown joins
/// pending producers, including a host prompt that ignores cancellation.
/// Completed abandoned prompts release their storage automatically.
pub const PromptOwner = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    callbacks: Callbacks,
    group: std.Io.Group = .init,
    pub const Callbacks = struct {
        context: ?*anyopaque,
        /// Return an owned copy or retained reference, independent of the
        /// stack/arena used to construct this owner. Paired with releaseContext.
        retainContext: *const fn (?*anyopaque, std.mem.Allocator, std.Io) anyerror!?*anyopaque,
        releaseContext: *const fn (?*anyopaque, std.mem.Allocator, std.Io) void,
        /// Prompt JSON and result allocation belong to this operation. The
        /// cancellation event is the native manual-prompt AbortSignal. A host
        /// may observe it or finish later; either lifetime is owned here.
        prompt: *const fn (?*anyopaque, std.mem.Allocator, std.Io, std.json.Value, *std.Io.Event) anyerror![]const u8,
        notify: *const fn (?*anyopaque, std.Io, std.json.Value) anyerror!void,
    };
    pub fn create(a: std.mem.Allocator, io: std.Io, callbacks: Callbacks) !*@This() {
        const self = try a.create(@This());
        errdefer a.destroy(self);
        self.* = .{ .allocator = a, .io = io, .callbacks = callbacks };
        self.callbacks.context = try callbacks.retainContext(callbacks.context, a, io);
        return self;
    }
    pub fn deinit(self: *@This()) void {
        self.group.cancel(self.io);
        self.callbacks.releaseContext(self.callbacks.context, self.allocator, self.io);
        const a = self.allocator;
        a.destroy(self);
    }
    pub fn interaction(self: *@This(), error_message: ?*?[]const u8) @import("types.zig").Interaction {
        return .{ .context = self, .prompt = prompt, .notify = notify, .startPrompt = startPrompt, .errorMessage = error_message };
    }
    pub fn start(self: *@This(), value: std.json.Value) !PromptOperation {
        const state = try self.allocator.create(State);
        errdefer self.allocator.destroy(state);
        state.* = .{ .owner = self, .arena = .init(self.allocator), .value = undefined };
        errdefer state.arena.deinit();
        state.value = try j.clone(state.arena.allocator(), value);
        try self.group.concurrent(self.io, State.run, .{state});
        return .{ .context = state, .waitFn = State.wait, .cancelFn = State.cancel, .releaseFn = State.release };
    }
    fn startPrompt(ctx: ?*anyopaque, _: std.mem.Allocator, _: std.Io, value: std.json.Value) !PromptOperation {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        return self.start(value);
    }
    fn prompt(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, value: std.json.Value) ![]const u8 {
        var operation = try startPrompt(ctx, a, io, value);
        defer operation.deinit();
        return operation.wait(a, io);
    }
    fn notify(ctx: ?*anyopaque, io: std.Io, value: std.json.Value) !void {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        return self.callbacks.notify(self.callbacks.context, io, value);
    }
    const State = struct {
        owner: *PromptOwner,
        arena: std.heap.ArenaAllocator,
        value: std.json.Value,
        references: std.atomic.Value(usize) = .init(2),
        done: std.Io.Event = .unset,
        canceled: std.Io.Event = .unset,
        result: anyerror![]const u8 = error.Canceled,
        fn run(self: *@This()) std.Io.Cancelable!void {
            const owner = self.owner;
            self.result = owner.callbacks.prompt(owner.callbacks.context, self.arena.allocator(), owner.io, self.value, &self.canceled);
            const canceled = if (self.result) |_| false else |err| err == error.Canceled;
            self.done.set(owner.io);
            release(self);
            if (canceled) return error.Canceled;
        }
        fn wait(ctx: *anyopaque, a: std.mem.Allocator, io: std.Io) ![]const u8 {
            const self: *@This() = @ptrCast(@alignCast(ctx));
            try self.done.wait(io);
            try io.checkCancel();
            return a.dupe(u8, try self.result);
        }
        fn cancel(ctx: *anyopaque) void {
            const self: *@This() = @ptrCast(@alignCast(ctx));
            self.canceled.set(self.owner.io);
        }
        fn release(ctx: *anyopaque) void {
            const self: *@This() = @ptrCast(@alignCast(ctx));
            if (self.references.fetchSub(1, .acq_rel) != 1) return;
            const a = self.owner.allocator;
            self.arena.deinit();
            a.destroy(self);
        }
    };
};
