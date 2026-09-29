//! Intrusive, cancellation-safe wait registration shared by local primitives.
//! An event is persistent so a signal before wait is not lost. The owner
//! unregisters its cancellation callback before destroying the waiter.
const std = @import("std");
const Cancellation = @import("runtime.zig").Cancellation;

/// Intrusive I/O waiter node; its owner keeps it alive until detached from the queue.
pub const Waiter = struct {
    previous: ?*Waiter = null,
    next: ?*Waiter = null,
    detached: bool = false,
    io: std.Io,
    event: std.Io.Event = .unset,
    registration: Cancellation.Registration = .{ .data = null, .forward = wake },

    fn wake(data: ?*anyopaque) void {
        const waiter: *Waiter = @ptrCast(@alignCast(data.?));
        waiter.event.set(waiter.io);
    }

    /// Register a definition or subscription with its owner.
    /// Parameters:
    /// - `self: *Waiter`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `first: *?*Waiter`: First operation in execution order.
    /// - `token: ?*const Cancellation`: Cancellation token observed by this operation; its owner must outlive pending
    ///   work.
    /// Returns (`void`): no value.
    pub fn register(self: *Waiter, first: *?*Waiter, token: ?*const Cancellation) void {
        self.detached = false;
        self.next = first.*;
        if (first.*) |head| head.previous = self;
        first.* = self;
        self.registration.data = self;
        if (token) |cancellation| cancellation.register(&self.registration);
    }

    /// Remove the selected entry.
    /// Parameters:
    /// - `self: *Waiter`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `first: *?*Waiter`: First operation in execution order.
    /// Returns (`void`): no value.
    pub fn remove(self: *Waiter, first: *?*Waiter) void {
        if (self.previous) |previous| previous.next = self.next else first.* = self.next;
        if (self.next) |next| next.previous = self.previous;
        self.previous = null;
        self.next = null;
    }

    /// Wake every waiter registered with this condition.
    /// Parameters:
    /// - `first: ?*Waiter`: First operation in execution order.
    /// Returns (`void`): no value.
    pub fn signalAll(first: ?*Waiter) void {
        var current = first;
        while (current) |waiter| {
            waiter.event.set(waiter.io);
            current = waiter.next;
        }
    }
};
