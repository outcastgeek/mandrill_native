//! Intrusive, cancellation-safe wait registration shared by local primitives.
//! An event is persistent so a signal before wait is not lost. The owner
//! unregisters its cancellation callback before destroying the waiter.
const std = @import("std");
const Cancellation = @import("runtime.zig").Cancellation;

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

    pub fn register(self: *Waiter, first: *?*Waiter, token: ?*const Cancellation) void {
        self.detached = false;
        self.next = first.*;
        if (first.*) |head| head.previous = self;
        first.* = self;
        self.registration.data = self;
        if (token) |cancellation| cancellation.register(&self.registration);
    }

    pub fn remove(self: *Waiter, first: *?*Waiter) void {
        if (self.previous) |previous| previous.next = self.next else first.* = self.next;
        if (self.next) |next| next.previous = self.previous;
        self.previous = null;
        self.next = null;
    }

    pub fn signalAll(first: ?*Waiter) void {
        var current = first;
        while (current) |waiter| {
            waiter.event.set(waiter.io);
            current = waiter.next;
        }
    }
};
