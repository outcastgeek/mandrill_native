const std = @import("std");
const outcomes = @import("outcome.zig");
const rt = @import("internal/runtime.zig");

pub fn Scope(comptime E: type) type {
    return struct {
        const Self = @This();
        const Phase = enum { open, closing, closed };
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
        finish_data: ?*anyopaque = null,
        finish: ?*const fn (*rt.State, ?*anyopaque, outcomes.Exit(void, E)) rt.Step = null,

        pub fn init(env: anytype) Self {
            return .{ .allocator = env.allocator, .io = env.io };
        }

        /// Caller ownership of a rejected finalizer is unchanged. Registration
        /// and close admission share the same lock, so no late resource is lost.
        pub fn register(self: *Self, finalizer: *Finalizer) bool {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.phase != .open) return false;
            finalizer.next = self.head;
            self.head = finalizer;
            return true;
        }

        pub const CloseAdmission = union(enum) { owner, pending, completed: outcomes.Exit(void, E) };
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
        pub fn beginClose(self: *Self) ?outcomes.Exit(void, E) {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            while (self.phase == .closing) self.changed.waitUncancelable(self.io, &self.mutex);
            if (self.phase == .closed) return self.result.clone();
            self.phase = .closing;
            return null;
        }

        pub fn drain(self: *Self, state: *rt.State, data: ?*anyopaque, finish: *const fn (*rt.State, ?*anyopaque, outcomes.Exit(void, E)) rt.Step) rt.Step {
            self.restore_services = state.services;
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
        pub fn deinit(self: *Self) void {
            std.debug.assert(self.phase == .closed and self.head == null and self.active == null);
            self.result.deinit();
            self.* = undefined;
        }
    };
}
