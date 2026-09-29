const std = @import("std");
const outcome = @import("../outcome.zig");
const diagnostics = @import("diagnostics.zig");
const providers = @import("../services.zig");

// These are the documented R capability fields. Reflection uses their typed
// identifiers so the public convention has one authoritative definition.
pub const EnvironmentField = enum { allocator, io, cancellation };
pub const Delivery = enum(u8) { ordinary, terminal };
pub const Interruption = enum(u8) { clear, requested };

pub const Cancellation = struct {
    requested: std.atomic.Value(bool) = .init(false),
    gate: std.atomic.Value(bool) = .init(false),
    first: ?*Registration = null,

    /// A registration belongs to its registering owner until unregister has
    /// returned. Forwarding MUST be nonblocking and must not mutate this token's
    /// registrations: it wakes a handle owner, never joins a native Future.
    pub const Registration = struct {
        data: ?*anyopaque,
        forward: *const fn (?*anyopaque) void,
        previous: ?*Registration = null,
        next: ?*Registration = null,
        token: ?*Cancellation = null,

        pub fn unregister(self: *Registration) void {
            const token = self.token orelse return;
            token.lock();
            defer token.unlock();
            if (self.previous) |previous| previous.next = self.next else token.first = self.next;
            if (self.next) |next| next.previous = self.previous;
            self.token = null;
        }
    };

    fn lock(self: *Cancellation) void {
        while (self.gate.swap(true, .acquire)) std.atomic.spinLoopHint();
    }

    fn unlock(self: *Cancellation) void {
        self.gate.store(false, .release);
    }

    pub fn register(self: *const Cancellation, registration: *Registration) void {
        const token = @constCast(self);
        token.lock();
        defer token.unlock();
        std.debug.assert(registration.token == null);
        if (token.requested.load(.acquire)) {
            registration.forward(registration.data);
            return;
        }
        registration.previous = null;
        registration.next = token.first;
        registration.token = token;
        if (token.first) |first| first.previous = registration;
        token.first = registration;
    }

    pub fn request(self: *Cancellation) void {
        if (self.requested.swap(true, .acq_rel)) return;
        self.lock();
        defer self.unlock();
        var next = self.first;
        while (next) |registration| {
            next = registration.next;
            registration.forward(registration.data);
        }
    }

    pub fn isRequested(self: *const Cancellation) bool {
        return self.requested.load(.acquire);
    }
};

pub const Services = struct {
    // Snapshot the native providers' vtable/data pairs, not addresses of R's
    // fields. This preserves the chosen provider across field replacement and
    // avoids nested optional-struct tags in the interpreter representation.
    allocator: ?*const std.mem.Allocator.VTable = null,
    // The allocator vtable is the presence tag. A valid stateless allocator
    // may use address zero for its context; wrapping that pointer in an
    // optional would mistake it for absence.
    allocator_data: *anyopaque = undefined,
    io: ?*const std.Io.VTable = null,
    io_data: ?*anyopaque = null,
    cancellation: ?*const Cancellation = null,
    // Resolve cancellation polling from R's known capabilities once. Pure
    // environments have no provider poll; their checkpoints still honor the
    // interpreter's latched interruption and cleanup protection.
    poll: ?*const fn (*const Services) bool = null,

    pub inline fn allocatorValue(self: Services) ?std.mem.Allocator {
        const vtable = self.allocator orelse return null;
        return .{ .ptr = self.allocator_data, .vtable = vtable };
    }

    pub inline fn ioValue(self: Services) ?std.Io {
        const vtable = self.io orelse return null;
        return .{ .userdata = self.io_data, .vtable = vtable };
    }

    pub inline fn inherit(self: Services, enclosing: Services) Services {
        return .{
            .allocator = self.allocator orelse enclosing.allocator,
            .allocator_data = if (self.allocator != null) self.allocator_data else enclosing.allocator_data,
            .io = self.io orelse enclosing.io,
            .io_data = if (self.io != null) self.io_data else enclosing.io_data,
            .cancellation = self.cancellation orelse enclosing.cancellation,
            .poll = if (self.io != null) self.poll else enclosing.poll,
        };
    }
};

// Runtime-built descriptions form a uniquely owned chain. Destruction and
// provider resolution must not recurse through that chain. The origin points
// into a retained ancestor, whose typed adapter resolves this run's R.
pub const DescriptionNode = struct {
    previous: ?*DescriptionNode,
    allocator: std.mem.Allocator,
    destroy: *const fn (*DescriptionNode) void,
    origin: ServiceOrigin,

    pub const ServiceOrigin = struct {
        data: *const anyopaque,
        resolve: *const fn (*const anyopaque, *const anyopaque) Services,

        pub fn services(self: ServiceOrigin, env: *const anyopaque) Services {
            return self.resolve(env, self.data);
        }
    };

    pub fn deinit(head: ?*DescriptionNode) void {
        var current = head;
        while (current) |node| {
            current = node.previous;
            node.destroy(node);
        }
    }
};

// This module is not exported. Application code supplies R, never State.
pub const State = struct {
    services: Services,
    origin_cancellation: ?*const Cancellation = null,
    branch_cancellation: ?*const Cancellation = null,
    interruption: Interruption = .clear,
    protection: usize = 0,
    annotations: []const providers.Annotation = &.{},
    span: ?providers.SpanId = null,
    fiber: bool = false,
    protected_failure: bool = false,
    ignore_environment_cancellation: bool = false,

    pub inline fn isPristine(self: *const State) bool {
        // The specialized entry is valid only for this exact empty state.
        // Bound providers, inherited cancellation, and protected cleanup all
        // retain the ordinary evaluator and its supplied runtime dependencies.
        return self.services.allocator == null and self.services.io == null and
            self.services.cancellation == null and self.services.poll == null and
            self.origin_cancellation == null and self.branch_cancellation == null and self.interruption == .clear and self.protection == 0 and self.annotations.len == 0 and self.span == null and !self.fiber and !self.protected_failure and !self.ignore_environment_cancellation;
    }

    pub inline fn cancelled(self: *State) bool {
        if (self.protection != 0) return false;
        if (self.interruption == .requested) return true;
        if (self.branch_cancellation) |token| {
            if (token.isRequested()) {
                self.interruption = .requested;
                return true;
            }
        }
        // Provision/provider replacement cannot detach a run from its original
        // logical cancellation token. Poll it separately only when different
        // from the active environment's token.
        if (self.origin_cancellation) |token| {
            if (token != self.services.cancellation and token.isRequested()) {
                self.interruption = .requested;
                return true;
            }
        }
        if (!self.ignore_environment_cancellation) if (self.services.cancellation) |token| {
            if (token.isRequested()) {
                self.interruption = .requested;
                return true;
            }
        };
        if (self.services.poll) |poll| {
            if (poll(&self.services)) self.interruption = .requested;
        }
        return self.interruption == .requested;
    }

    pub inline fn publish(self: *State, value: anytype) @TypeOf(value) {
        if (!self.cancelled()) return value;
        return switch (value) {
            .success => blk: {
                var discarded = value;
                discarded.deinit();
                break :blk .interrupted;
            },
            .fail => if (self.fiber and self.protected_failure) value else blk: {
                var discarded = value;
                discarded.deinit();
                break :blk .interrupted;
            },
            else => value,
        };
    }
};

pub const Step = struct {
    data: ?*anyopaque,
    advance: ?*const fn (*State, ?*anyopaque) Step,
    suspended: ?*const fn (*State, ?*anyopaque) Step = null,

    pub const done: Step = .{ .data = null, .advance = null };
};

pub fn Sink(comptime A: type, comptime E: type) type {
    return struct {
        output: *outcome.Exit(A, E),
        next_step: Step,

        pub fn deliver(self: @This(), value: outcome.Exit(A, E)) Step {
            self.output.* = value;
            return self.next_step;
        }
    };
}

pub inline fn services(comptime R: type, env: *const R) Services {
    if (R == void) return .{};
    if (@typeInfo(R) != .@"struct") return .{};
    var result: Services = .{};
    if (@hasField(R, @tagName(EnvironmentField.allocator))) {
        if (@FieldType(R, @tagName(EnvironmentField.allocator)) != std.mem.Allocator)
            @compileError(diagnostics.allocator_field);
        result.allocator = env.allocator.vtable;
        result.allocator_data = env.allocator.ptr;
    }
    if (@hasField(R, @tagName(EnvironmentField.io))) {
        if (@FieldType(R, @tagName(EnvironmentField.io)) != std.Io)
            @compileError(diagnostics.io_field);
        result.io = env.io.vtable;
        result.io_data = env.io.userdata;
    }
    if (@hasField(R, @tagName(EnvironmentField.cancellation))) {
        const T = @FieldType(R, @tagName(EnvironmentField.cancellation));
        if (T != *Cancellation and T != *const Cancellation)
            @compileError(diagnostics.cancellation_field);
        result.cancellation = env.cancellation;
    }
    if (@hasField(R, @tagName(EnvironmentField.io))) {
        result.poll = struct {
            fn poll(active: *const Services) bool {
                active.ioValue().?.checkCancel() catch return true;
                return false;
            }
        }.poll;
    }
    return result;
}
