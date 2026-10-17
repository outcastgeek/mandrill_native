//! Deterministic partial-admission provider for public DSL proofs.
const std = @import("std");
const fx = @import("effect");
const Threads = @import("thread_executor.zig").ThreadExecutor;

pub const Controlled = struct {
    base: Threads,
    io: std.Io,
    first_started: *std.Io.Event,
    capture_active: *std.atomic.Value(bool),
    items_active: *std.atomic.Value(bool),
    invalid: *std.atomic.Value(bool),
    reject_second_worker: bool,
    reject_first_worker: bool = false,
    submissions: std.atomic.Value(usize) = .init(0),
    accepted: std.atomic.Value(usize) = .init(0),
    requested: std.atomic.Value(usize) = .init(0),
    joined: std.atomic.Value(usize) = .init(0),
    destroyed: std.atomic.Value(usize) = .init(0),
    pub fn executor(self: *@This()) fx.Executor {
        return .{ .userdata = self, .capabilities = self.base.capabilities, .submit_fn = submit };
    }
    pub fn deinit(self: *@This()) void {
        std.debug.assert(self.accepted.load(.acquire) == self.joined.load(.acquire));
        std.debug.assert(self.accepted.load(.acquire) == self.destroyed.load(.acquire));
        self.base.deinit();
    }

    const Handle = struct {
        owner: *Controlled,
        base: fx.Executor.Handle,
        worker: bool,
        joined: bool = false,
        fn request(data: *anyopaque) ?fx.Defect {
            const self: *@This() = @ptrCast(@alignCast(data));
            if (self.worker) _ = self.owner.requested.fetchAdd(1, .acq_rel);
            return self.base.request();
        }
        fn join(data: *anyopaque, protected: bool) fx.Executor.Handle.JoinResult {
            const self: *@This() = @ptrCast(@alignCast(data));
            const result = self.base.joinProtected(protected);
            self.joined = true;
            if (self.worker) _ = self.owner.joined.fetchAdd(1, .acq_rel);
            return result;
        }
        fn complete(data: *anyopaque) bool {
            const self: *@This() = @ptrCast(@alignCast(data));
            return self.base.isComplete();
        }
        fn destroy(data: *anyopaque) void {
            const self: *@This() = @ptrCast(@alignCast(data));
            std.debug.assert(self.joined);
            const owner = self.owner;
            self.base.deinit();
            if (self.worker) {
                if (!owner.capture_active.load(.acquire) or !owner.items_active.load(.acquire)) owner.invalid.store(true, .release);
                _ = owner.destroyed.fetchAdd(1, .acq_rel);
            }
            owner.base.allocator.destroy(self);
        }
    };
    fn submit(data: ?*anyopaque, kind: fx.ExecutionKind, job: fx.ExecutionJob) fx.Executor.Submission {
        const self: *@This() = @ptrCast(@alignCast(data.?));
        const worker = kind == .independent;
        // runAsync executes its prefix before admitting its async_io root.
        // Classify actual traversal submissions by their independent lane.
        if (worker) {
            const ordinal = self.submissions.fetchAdd(1, .acq_rel);
            if (ordinal == 0 and self.reject_first_worker) return .{ .rejected = .{ .kind = .provider_unavailable, .message = "controlled first-worker rejection" } };
            if (ordinal == 1) {
                self.first_started.waitUncancelable(self.io);
                if (self.reject_second_worker) return .{ .rejected = .{ .kind = .provider_unavailable, .message = "controlled second-worker rejection" } };
            }
        }
        const handle = self.base.allocator.create(Handle) catch return .{ .rejected = fx.Defect.out_of_memory };
        const accepted = self.base.executor().submit(kind, job);
        if (accepted == .rejected) {
            self.base.allocator.destroy(handle);
            return accepted;
        }
        handle.* = .{ .owner = self, .base = accepted.accepted, .worker = worker };
        if (handle.worker) _ = self.accepted.fetchAdd(1, .acq_rel);
        return .{ .accepted = .{ .userdata = handle, .vtable = &.{ .request = Handle.request, .join = Handle.join, .is_complete = Handle.complete, .destroy = Handle.destroy } } };
    }
};
