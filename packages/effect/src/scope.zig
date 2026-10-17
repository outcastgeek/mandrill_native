//! Structured scopes that retain resources and child work through finalization.
const std = @import("std");
const outcomes = @import("outcome.zig");
const rt = @import("internal/runtime.zig");

/// Specialize a scope that owns finalizers and their typed failure channel.
/// Parameters:
/// - `E: comptime type`: Expected application-failure type; defects and interruption remain separate outcomes.
/// Returns (`type`): owner of finalizers and child lifetimes that reports cleanup failures as `E`.
pub fn Scope(comptime E: type) type {
    return struct {
        const Self = @This();
        const Phase = enum { open, closing, closed };
        /// `Finalizer` is a callback interface with `next: ?*Finalizer`, `start: *const fn (*Finalizer,
        /// *rt.State, rt.Sink(void, E)) rt.Step`, `destroy: *const fn (*Finalizer) void`, `storage:
        /// outcomes.CleanupStorage(E)`.
        pub const Finalizer = struct {
            next: ?*Finalizer = null,
            start: *const fn (*Finalizer, *rt.State, rt.Sink(void, E)) rt.Step,
            destroy: *const fn (*Finalizer) void,
            storage: outcomes.CleanupStorage(E),
        };

        allocator: std.mem.Allocator,
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        changed: std.Io.Condition = .init,
        phase: Phase = .open,
        head: ?*Finalizer = null,
        result: outcomes.Exit(void, E) = .{ .success = {} },
        active: ?*Finalizer = null,
        cleanup: outcomes.Exit(void, E) = undefined,
        restore_services: rt.Services = undefined,
        restore_committed_delivery: bool = false,
        finish_data: ?*anyopaque = null,
        finish: ?*const fn (*rt.State, ?*anyopaque, outcomes.Exit(void, E)) rt.Step = null,

        /// Open a scope bound to the caller environment and its cleanup capabilities.
        /// Parameters:
        /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// Returns (`Self`): an open scope that must be closed to run registered finalizers.
        pub fn init(env: anytype) Self {
            return .{ .allocator = env.allocator, .io = env.io };
        }

        /// Caller ownership of a rejected finalizer is unchanged. Registration
        /// and close admission share the same lock, so no late resource is lost.
        ///
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `finalizer: *Finalizer`: Cleanup callback invoked at scope exit.
        /// Returns (`bool`): `true` if the scope was open and accepted the finalizer; `false` after closing began.
        pub fn register(self: *Self, finalizer: *Finalizer) bool {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.phase != .open) return false;
            finalizer.next = self.head;
            self.head = finalizer;
            return true;
        }

        /// Close admission: `owner` must drain finalizers, `pending` waits for
        /// that owner, and `completed` carries the already published cleanup Exit.
        pub const CloseAdmission = union(enum) { owner, pending, completed: outcomes.Exit(void, E) };
        /// Move the Scope toward closing while preventing new acquisitions.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`CloseAdmission`): The Scope’s admission or closing decision.
        pub fn admitClose(self: *Self) CloseAdmission {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            return switch (self.phase) {
                .open => blk: {
                    self.phase = .closing;
                    break :blk .owner;
                },
                .closing => .pending,
                .closed => .{ .completed = self.result.clone() },
            };
        }

        /// Null grants drain ownership. Other closers join the single drain and
        /// receive separately owned references to its immutable cleanup data.
        ///
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`?outcomes.Exit(void, E)`): Carries no value on `.success` or `E` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn beginClose(self: *Self) ?outcomes.Exit(void, E) {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            while (self.phase == .closing) self.changed.waitUncancelable(self.io, &self.mutex);
            if (self.phase == .closed) return self.result.clone();
            self.phase = .closing;
            return null;
        }

        /// Run and collect finalizers registered with this Scope.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `state: *rt.State`: Mutable operation state, retained for the duration of this call.
        /// - `data: ?*anyopaque`: opaque finalizer state supplied back to its release callback.
        /// - `finish: *const fn (*rt.State, ?*anyopaque, outcomes.Exit(void, E)) rt.Step`: Typed function selected at comptime; no runtime callback lookup is
        ///   required.
        /// Returns (`rt.Step`): The next interpreter step, including suspension or completion.
        pub fn drain(self: *Self, state: *rt.State, data: ?*anyopaque, finish: *const fn (*rt.State, ?*anyopaque, outcomes.Exit(void, E)) rt.Step) rt.Step {
            self.restore_services = state.services;
            self.restore_committed_delivery = state.committed_delivery;
            state.committed_delivery = false;
            self.finish_data = data;
            self.finish = finish;
            state.protection += 1;
            return .{ .data = self, .advance = next };
        }

        fn next(state: *rt.State, data: ?*anyopaque) rt.Step {
            const self: *Self = @ptrCast(@alignCast(data.?));
            if (self.head) |finalizer| {
                self.head = finalizer.next;
                self.active = finalizer;
                return finalizer.start(finalizer, state, .{ .output = &self.cleanup, .next_step = .{ .data = self, .advance = released } });
            }
            state.protection -= 1;
            state.services = self.restore_services;
            state.committed_delivery = self.restore_committed_delivery;
            self.mutex.lockUncancelable(self.io);
            self.phase = .closed;
            const value = self.result.clone();
            self.changed.broadcast(self.io);
            self.mutex.unlock(self.io);
            return self.finish.?(state, self.finish_data, value);
        }

        fn released(_: *rt.State, data: ?*anyopaque) rt.Step {
            const self: *Self = @ptrCast(@alignCast(data.?));
            const finalizer = self.active.?;
            finalizer.storage.appendExit(&self.result, self.cleanup);
            if (!finalizer.storage.retained()) finalizer.destroy(finalizer);
            self.active = null;
            return .{ .data = self, .advance = next };
        }

        /// The scope owner must have joined all users and completed close first.
        ///
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            std.debug.assert(self.phase == .closed and self.head == null and self.active == null);
            self.result.deinit();
            self.* = undefined;
        }
    };
}
