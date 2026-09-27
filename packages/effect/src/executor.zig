const diagnostics = @import("internal/diagnostics.zig");
const std = @import("std");
const outcomes = @import("outcome.zig");
const Cancellation = @import("internal/runtime.zig").Cancellation;

pub const Kind = enum { async_io, independent, cpu, blocking };
pub const Capabilities = struct { independent: bool, cpu: bool, blocking: bool };
pub const Job = struct { data: *anyopaque, run: *const fn (*anyopaque) void, cancellation: ?*Cancellation = null };

/// An accepted job runs exactly once and remains owned until join completes.
/// Rejection guarantees that the submitted callback has not started. Providers
/// must reject saturated nested admission rather than deadlock a parent worker.
pub const Executor = struct {
    userdata: ?*anyopaque,
    capabilities: Capabilities,
    submit_fn: *const fn (?*anyopaque, Kind, Job) Submission,

    pub const Submission = union(enum) { accepted: Handle, rejected: outcomes.Defect };
    pub const Handle = struct {
        pub const JoinResult = NativeJoinResult;
        userdata: *anyopaque,
        vtable: *const VTable,

        pub const VTable = struct {
            // Nonblocking and threadsafe. Returns an explicit forwarding defect
            // when the provider cannot request cancellation of owned work.
            request: *const fn (*anyopaque) ?outcomes.Defect,
            // The caller serializes joining and destruction. request may run
            // concurrently, but never owns or awaits the underlying handle.
            join: *const fn (*anyopaque, bool) JoinResult,
            is_complete: *const fn (*anyopaque) bool,
            destroy: *const fn (*anyopaque) void,
        };

        pub fn isComplete(self: Handle) bool {
            return self.vtable.is_complete(self.userdata);
        }
        pub fn request(self: Handle) ?outcomes.Defect {
            return self.vtable.request(self.userdata);
        }
        pub fn join(self: Handle) JoinResult {
            return self.joinProtected(false);
        }
        pub fn joinProtected(self: Handle, protected: bool) JoinResult {
            return self.vtable.join(self.userdata, protected);
        }
        pub fn deinit(self: *Handle) void {
            self.vtable.destroy(self.userdata);
            self.* = undefined;
        }
    };

    pub fn submit(self: Executor, kind: Kind, job: Job) Submission {
        const supported = switch (kind) {
            .async_io => true,
            .independent => self.capabilities.independent,
            .cpu => self.capabilities.independent and self.capabilities.cpu,
            .blocking => self.capabilities.independent and self.capabilities.blocking,
        };
        if (!supported) return .{ .rejected = .{ .kind = .provider_unavailable, .message = diagnostics.execution_provider_does_not_support_the_requested_capability } };
        return self.submit_fn(self.userdata, kind, job);
    }
};

/// Consumer-owned adapter. Capabilities describe the supplied implementation,
/// not std.Io in general: an event loop cannot claim independent CPU workers.
pub const NativeIo = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    capabilities: Capabilities,
    maximum_owned: usize,
    owned: std.atomic.Value(usize) = .init(0),

    pub fn executor(self: *NativeIo) Executor {
        return .{ .userdata = self, .capabilities = self.capabilities, .submit_fn = submit };
    }

    pub fn deinit(self: *NativeIo) void {
        std.debug.assert(self.owned.load(.acquire) == 0);
        self.* = undefined;
    }

    const Task = struct {
        owner: *NativeIo,
        allocator: std.mem.Allocator,
        io: std.Io,
        job: Job,
        future: std.Io.Future(void) = undefined,
        wake: std.Io.Event = .unset,
        completed: std.atomic.Value(bool) = .init(false),
        requested: std.atomic.Value(bool) = .init(false),
        joined: bool = false,
        parent_interrupted: bool = false,

        fn run(self: *Task) void {
            self.job.run(self.job.data);
            self.completed.store(true, .release);
            self.wake.set(self.io);
        }

        fn isComplete(data: *anyopaque) bool {
            const self: *Task = @ptrCast(@alignCast(data));
            return self.completed.load(.acquire);
        }

        fn request(data: *anyopaque) ?outcomes.Defect {
            const self: *Task = @ptrCast(@alignCast(data));
            self.requested.store(true, .release);
            self.wake.set(self.io);
            return null;
        }

        fn join(data: *anyopaque, protected: bool) Executor.Handle.JoinResult {
            const self: *Task = @ptrCast(@alignCast(data));
            if (self.joined) return .{ .interrupted = self.parent_interrupted };
            const previous = if (protected) self.io.swapCancelProtection(.blocked) else null;
            defer if (previous) |value| {
                _ = self.io.swapCancelProtection(value);
            };
            // Exactly this owner calls native await/cancel. A cancellation
            // registration only wakes this owner; it never races those calls.
            self.wake.wait(self.io) catch {
                self.parent_interrupted = true;
                self.requested.store(true, .release);
                if (self.job.cancellation) |token| token.request();
            };
            const protection = self.io.swapCancelProtection(.blocked);
            defer _ = self.io.swapCancelProtection(protection);
            if (self.requested.load(.acquire) and !self.completed.load(.acquire)) self.future.cancel(self.io) else self.future.await(self.io);
            self.joined = true;
            return .{ .interrupted = self.parent_interrupted };
        }

        fn destroy(data: *anyopaque) void {
            const self: *Task = @ptrCast(@alignCast(data));
            std.debug.assert(self.joined);
            _ = self.owner.owned.fetchSub(1, .acq_rel);
            self.allocator.destroy(self);
        }
    };

    fn submit(data: ?*anyopaque, kind: Kind, job: Job) Executor.Submission {
        const self: *NativeIo = @ptrCast(@alignCast(data.?));
        _ = kind;
        // Event-driven cancellation needs progress before join. std.Io.async
        // permits deferred execution until await, so this adapter requires the
        // stronger concurrent capability even for an asynchronous continuation.
        if (!self.capabilities.independent) return .{ .rejected = .{ .kind = .provider_unavailable, .message = diagnostics.native_wakeable_execution_requires_independent_progress } };
        var current = self.owned.load(.acquire);
        while (true) {
            if (current >= self.maximum_owned) return .{ .rejected = .{ .kind = .provider_unavailable, .message = diagnostics.execution_provider_admission_capacity_is_exhausted } };
            if (self.owned.cmpxchgWeak(current, current + 1, .acq_rel, .acquire)) |changed| current = changed else break;
        }
        const task = self.allocator.create(Task) catch {
            _ = self.owned.fetchSub(1, .acq_rel);
            return .{ .rejected = outcomes.Defect.out_of_memory };
        };
        task.* = .{ .owner = self, .allocator = self.allocator, .io = self.io, .job = job };
        task.future = self.io.concurrent(Task.run, .{task}) catch {
            self.allocator.destroy(task);
            _ = self.owned.fetchSub(1, .acq_rel);
            return .{ .rejected = .{ .kind = .provider_unavailable, .message = diagnostics.native_i_o_provider_rejected_concurrent_execution } };
        };
        return .{ .accepted = .{ .userdata = task, .vtable = &.{ .request = Task.request, .join = Task.join, .is_complete = Task.isComplete, .destroy = Task.destroy } } };
    }
};

/// Owns a forwarding registration through join. Requests are recorded even if
/// cancellation arrived before submission; no native handle has two owners.
pub const Registered = struct {
    handle: Executor.Handle,
    registration: Cancellation.Registration,
    failure: ?outcomes.Defect = null,
    gate: std.atomic.Value(bool) = .init(false),

    pub fn init(self: *Registered, handle: Executor.Handle, token: *const Cancellation) void {
        self.* = .{ .handle = handle, .registration = .{ .data = self, .forward = forward } };
        token.register(&self.registration);
    }

    fn forward(data: ?*anyopaque) void {
        const self: *Registered = @ptrCast(@alignCast(data.?));
        while (self.gate.swap(true, .acquire)) std.atomic.spinLoopHint();
        defer self.gate.store(false, .release);
        if (self.handle.request()) |failure| self.failure = failure;
    }

    pub const JoinResult = struct { interrupted: bool, forwarding: ?outcomes.Defect, joining: ?outcomes.Defect };

    pub fn join(self: *Registered) JoinResult {
        return self.joinProtected(false);
    }
    pub fn joinProtected(self: *Registered, protected: bool) JoinResult {
        const joined = self.handle.joinProtected(protected);
        self.registration.unregister();
        self.handle.deinit();
        return .{ .interrupted = joined.interrupted, .forwarding = self.failure, .joining = joined.defect };
    }
};
pub const NativeJoinResult = struct { interrupted: bool = false, defect: ?outcomes.Defect = null };
