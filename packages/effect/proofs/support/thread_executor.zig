//! External consumer implementation: OS threads, with no I/O service in its ABI.
const std = @import("std");
const fx = @import("effect");

pub const ThreadExecutor = struct {
    allocator: std.mem.Allocator,
    capacity: usize,
    capabilities: fx.ExecutionCapabilities,
    request_failure: ?fx.Defect = null,
    join_failure: ?fx.Defect = null,
    owned: std.atomic.Value(usize) = .init(0),
    peak: std.atomic.Value(usize) = .init(0),
    started: std.atomic.Value(usize) = .init(0),
    completed: std.atomic.Value(usize) = .init(0),
    joined: std.atomic.Value(usize) = .init(0),
    requested: std.atomic.Value(usize) = .init(0),

    pub fn executor(self: *@This()) fx.Executor {
        return .{ .userdata = self, .capabilities = self.capabilities, .submit_fn = submit };
    }

    pub fn deinit(self: *@This()) void {
        std.debug.assert(self.owned.load(.acquire) == 0);
        std.debug.assert(self.started.load(.acquire) == self.completed.load(.acquire));
        std.debug.assert(self.started.load(.acquire) == self.joined.load(.acquire));
    }

    const Task = struct {
        owner: *ThreadExecutor,
        job: fx.ExecutionJob,
        thread: std.Thread = undefined,
        completed: std.atomic.Value(bool) = .init(false),
        joined: bool = false,

        fn run(task: *@This()) void {
            _ = task.owner.started.fetchAdd(1, .monotonic);
            task.job.run(task.job.data);
            _ = task.owner.completed.fetchAdd(1, .monotonic);
            task.completed.store(true, .release);
        }
        fn request(data: *anyopaque) ?fx.Defect {
            const task: *@This() = @ptrCast(@alignCast(data));
            // The Effect token already holds the request. Only the native join
            // owner waits for this thread; cancellation forwarding never joins.
            _ = task.owner.requested.fetchAdd(1, .monotonic);
            return task.owner.request_failure;
        }
        fn join(data: *anyopaque, _: bool) fx.Executor.Handle.JoinResult {
            const task: *@This() = @ptrCast(@alignCast(data));
            std.debug.assert(!task.joined);
            task.thread.join();
            task.joined = true;
            _ = task.owner.joined.fetchAdd(1, .monotonic);
            return .{ .defect = task.owner.join_failure };
        }
        fn isComplete(data: *anyopaque) bool {
            const task: *@This() = @ptrCast(@alignCast(data));
            return task.completed.load(.acquire);
        }
        fn destroy(data: *anyopaque) void {
            const task: *@This() = @ptrCast(@alignCast(data));
            std.debug.assert(task.joined);
            const owner = task.owner;
            owner.allocator.destroy(task);
            _ = owner.owned.fetchSub(1, .acq_rel);
        }
    };

    fn submit(data: ?*anyopaque, _: fx.ExecutionKind, job: fx.ExecutionJob) fx.Executor.Submission {
        const self: *@This() = @ptrCast(@alignCast(data.?));
        var used = self.owned.load(.acquire);
        while (true) {
            if (used == self.capacity) return .{ .rejected = .{ .kind = .provider_unavailable, .message = "consumer thread capacity exhausted" } };
            if (self.owned.cmpxchgWeak(used, used + 1, .acq_rel, .acquire)) |changed| used = changed else break;
        }
        var peak = self.peak.load(.acquire);
        while (peak < used + 1) {
            if (self.peak.cmpxchgWeak(peak, used + 1, .acq_rel, .acquire)) |changed| peak = changed else break;
        }
        const task = self.allocator.create(Task) catch {
            _ = self.owned.fetchSub(1, .acq_rel);
            return .{ .rejected = fx.Defect.out_of_memory };
        };
        task.* = .{ .owner = self, .job = job };
        task.thread = std.Thread.spawn(.{ .allocator = self.allocator }, Task.run, .{task}) catch {
            self.allocator.destroy(task);
            _ = self.owned.fetchSub(1, .acq_rel);
            return .{ .rejected = .{ .kind = .provider_unavailable, .message = "consumer thread spawn failed" } };
        };
        return .{ .accepted = .{ .userdata = task, .vtable = &.{ .request = Task.request, .join = Task.join, .is_complete = Task.isComplete, .destroy = Task.destroy } } };
    }
};
