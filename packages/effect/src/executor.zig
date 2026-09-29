//! Caller-supplied async, CPU, and blocking execution capabilities.
const diagnostics = @import("internal/diagnostics.zig");
const std = @import("std");
const outcomes = @import("outcome.zig");
const Cancellation = @import("internal/runtime.zig").Cancellation;

/// Execution lane requested by an Effect. Providers must only accept lanes
/// covered by their advertised `Capabilities`.
pub const Kind = enum { async_io, independent, cpu, blocking };
/// Advertised lanes for one executor. `async_io` is always available through
/// `Executor.submit`; CPU and blocking work also require `independent` so a
/// parent worker cannot deadlock waiting on its own lane.
pub const Capabilities = struct { independent: bool, cpu: bool, blocking: bool };
/// A callback submitted to an executor. `data` is passed unchanged to `run`;
/// `cancellation` lets the provider request cooperative stop. All pointers are
/// borrowed until the accepted handle is joined and destroyed. On rejection,
/// the callback has not run and the caller keeps the job.
pub const Job = struct { data: *anyopaque, run: *const fn (*anyopaque) void, cancellation: ?*Cancellation = null };

/// An accepted job runs exactly once and remains owned until join completes.
/// Rejection guarantees that the submitted callback has not started. Providers
/// must reject saturated nested admission rather than deadlock a parent worker.
pub const Executor = struct {
    userdata: ?*anyopaque,
    capabilities: Capabilities,
    /// Submit a job to a provider. The callback receives borrowed `userdata`,
    /// the requested `Kind`, and the `Job`. Acceptance transfers responsibility
    /// for running the job exactly once and returns its owned handle. Rejection
    /// must happen before `Job.run` starts and leaves the job with the caller.
    submit_fn: *const fn (?*anyopaque, Kind, Job) Submission,

    /// Admission result. Rejection leaves the job with the caller.
    pub const Submission = union(enum) { accepted: Handle, rejected: outcomes.Defect };
    /// Owned provider handle. `request` forwards cancellation; `join` observes
    /// completion; `deinit` destroys the handle after joining.
    pub const Handle = struct {
        /// Typed join result channel described by `NativeJoinResult`.
        pub const JoinResult = NativeJoinResult;
        userdata: *anyopaque,
        vtable: *const VTable,

        /// Typed function-pointer interface for this capability. Implement every
        /// hook with the exact signature below; keep the table valid while the
        /// capability or pending work can invoke it.
        pub const VTable = struct {
            // Nonblocking and threadsafe. Returns an explicit forwarding defect
            // when the provider cannot request cancellation of owned work.
            /// Forward cancellation without joining.
            /// Parameters (callback order):
            /// - `1: *anyopaque`: the handle's `userdata`; it stays valid until `destroy`.
            /// Returns (`?outcomes.Defect`): `null` when the request was forwarded;
            /// otherwise a defect explaining why forwarding failed.
            request: *const fn (*anyopaque) ?outcomes.Defect,
            // The caller serializes joining and destruction. request may run
            // concurrently, but never owns or awaits the underlying handle.
            /// Join owned work and collect its outcome.
            /// Parameters (callback order):
            /// - `1: *anyopaque`: the handle's `userdata`; it stays valid until `destroy`.
            /// - `2: bool`: whether joining is protected against interruption.
            /// Returns (`JoinResult`): typed join outcome carrying completion and interruption/defect information.
            join: *const fn (*anyopaque, bool) JoinResult,
            /// Report whether work has completed.
            /// Parameters (callback order):
            /// - `1: *anyopaque`: the handle's `userdata`; it stays valid until `destroy`.
            /// Returns (`bool`): `true` after the callback has completed.
            is_complete: *const fn (*anyopaque) bool,
            /// Release the owned handle.
            /// Parameters (callback order):
            /// - `1: *anyopaque`: the handle's `userdata`; this call releases it.
            /// Returns (`void`): no value; the handle is invalid after this call.
            destroy: *const fn (*anyopaque) void,
        };

        /// Check whether the submitted job has completed.
        /// Parameters:
        /// - `self: Handle`: borrowed provider handle; retain its `userdata` until destruction.
        /// Returns (`bool`): `true` when the submitted callback has finished.
        pub fn isComplete(self: Handle) bool {
            return self.vtable.is_complete(self.userdata);
        }
        /// Ask the provider to cancel this handle's job without waiting for it.
        /// Parameters:
        /// - `self: Handle`: borrowed provider handle; retain its `userdata` until destruction.
        /// Returns (`?outcomes.Defect`): `null` if forwarded, or a provider defect
        /// when cancellation could not be requested. The owner must still join.
        pub fn request(self: Handle) ?outcomes.Defect {
            return self.vtable.request(self.userdata);
        }
        /// Wait for the child operation and obtain its result.
        /// Parameters:
        /// - `self: Handle`: borrowed provider handle; retain its `userdata` until destruction.
        /// Returns (`JoinResult`): Completion status, including interruption or provider defects where declared.
        pub fn join(self: Handle) JoinResult {
            return self.joinProtected(false);
        }
        /// Join a pending operation with the selected interruption protection.
        /// Parameters:
        /// - `self: Handle`: borrowed provider handle; retain its `userdata` until destruction.
        /// - `protected: bool`: Whether joining is protected against interruption.
        /// Returns (`JoinResult`): Completion status, including interruption or provider defects where declared.
        pub fn joinProtected(self: Handle, protected: bool) JoinResult {
            return self.vtable.join(self.userdata, protected);
        }
        /// Destroy a handle after its job has been joined; using it afterward is invalid.
        /// Parameters:
        /// - `self: *Handle`: unique owned handle to destroy.
        /// Returns (`void`): no value; the provider releases the handle's storage.
        pub fn deinit(self: *Handle) void {
            self.vtable.destroy(self.userdata);
            self.* = undefined;
        }
    };

    /// Admit work only when the requested lane is supported.
    /// Parameters:
    /// - `self: Executor`: Borrowed provider adapter and its advertised capabilities.
    /// - `kind: Kind`: Requested async, independent, CPU, or blocking lane.
    /// - `job: Job`: Callback and state transferred to the provider only on acceptance.
    /// Returns (`Submission`): Accepted work with an owned join handle, or a
    /// defect explaining rejection before the callback starts.
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

    /// Expose this NativeIo adapter as an Executor capability.
    /// Parameters:
    /// - `self: *NativeIo`: adapter whose address must remain stable while work is pending.
    /// Returns (`Executor`): a borrowed capability whose `userdata` points to
    /// this adapter; finish every accepted job before `deinit`.
    pub fn executor(self: *NativeIo) Executor {
        return .{ .userdata = self, .capabilities = self.capabilities, .submit_fn = submit };
    }

    /// Invalidate this adapter after all accepted jobs have been joined and destroyed.
    /// Parameters:
    /// - `self: *NativeIo`: adapter with no remaining owned jobs; an assertion
    /// checks this precondition.
    /// Returns (`void`): no value; the adapter cannot be used again without initialization.
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

    /// Take ownership of an accepted executor handle and register cancellation forwarding.
    /// Parameters:
    /// - `self: *Registered`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `handle: Executor.Handle`: Owned executor handle joined and destroyed by this operation.
    /// - `token: *const Cancellation`: Cancellation token observed by this operation; its owner must outlive pending
    ///   work.
    /// Returns (`void`): no value; the registration stays active until join removes it.
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

    /// Typed join result record.
    pub const JoinResult = struct { interrupted: bool, forwarding: ?outcomes.Defect, joining: ?outcomes.Defect };

    /// Wait for the child operation and obtain its result.
    /// Parameters:
    /// - `self: *Registered`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`JoinResult`): Completion status, including interruption or provider defects where declared.
    pub fn join(self: *Registered) JoinResult {
        return self.joinProtected(false);
    }
    /// Join a pending operation with the selected interruption protection.
    /// Parameters:
    /// - `self: *Registered`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `protected: bool`: Whether joining is protected against interruption.
    /// Returns (`JoinResult`): Completion status, including interruption or provider defects where declared.
    pub fn joinProtected(self: *Registered, protected: bool) JoinResult {
        const joined = self.handle.joinProtected(protected);
        self.registration.unregister();
        self.handle.deinit();
        return .{ .interrupted = joined.interrupted, .forwarding = self.failure, .joining = joined.defect };
    }
};
/// Typed native join result record.
pub const NativeJoinResult = struct { interrupted: bool = false, defect: ?outcomes.Defect = null };
