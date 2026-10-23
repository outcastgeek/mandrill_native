//! Native notifications use Io events and synchronous abort registrations.
const std = @import("std");
const diagnostics = @import("diagnostics.zig");
const Listener = struct {
    node: std.DoublyLinkedList.Node = .{},
    input: ?*AbortSignal = null,
    output: *AbortSignal,
};
pub const AbortSignal = struct {
    arena: std.heap.ArenaAllocator,
    mutex: std.Io.Mutex = .init,
    event: std.Io.Event = .unset,
    reason: ?diagnostics.ThrownValue = null,
    listeners: std.DoublyLinkedList = .{},
    pub fn init(a: std.mem.Allocator) @This() {
        return .{ .arena = .init(a) };
    }
    /// Retained reason data belongs to this signal. Detach dependent combined
    /// signals and stop all users before deinit.
    pub fn deinit(self: *@This()) void {
        std.debug.assert(self.listeners.first == null);
        self.arena.deinit();
        self.* = undefined;
    }
    pub fn abort(self: *@This(), io: std.Io, reason: ?diagnostics.ThrownValue) !void {
        self.mutex.lockUncancelable(io);
        defer self.mutex.unlock(io);
        if (self.event.isSet()) return;
        const selected = if (reason) |value| (if (value == .undefined_value) defaultReason() else value) else defaultReason();
        var failure: ?anyerror = null;
        self.reason = @import("../types.zig").cloneInto(diagnostics.ThrownValue, self.arena.allocator(), selected) catch |err| failed: {
            failure = err;
            break :failed .{ .native = err };
        };
        self.event.set(io);
        // Publish every dependent signal before this abort returns. Keeping
        // the input lock also excludes cleanup until these callbacks settle.
        var node = self.listeners.first;
        while (node) |current| {
            const listener: *Listener = @fieldParentPtr("node", current);
            listener.output.abort(io, self.reason) catch {};
            node = current.next;
        }
        if (failure) |err| return err;
    }
    fn defaultReason() diagnostics.ThrownValue {
        return .{ .exception = .{ .name = "AbortError", .message = "This operation was aborted" } };
    }
};
pub const CombinedAbortSignal = struct {
    signal: ?*AbortSignal = null,
    state: ?*State = null,
    const State = struct {
        allocator: std.mem.Allocator,
        io: std.Io,
        signal: AbortSignal,
        listeners: []Listener = &.{},
        fn cleanup(self: *@This()) void {
            for (self.listeners) |*listener| if (listener.input) |input| {
                input.mutex.lockUncancelable(self.io);
                input.listeners.remove(&listener.node);
                listener.input = null;
                input.mutex.unlock(self.io);
            };
        }
    };
    /// Detach registrations, waiting for an active synchronous publication.
    /// The copied signal/reason stays valid until deinit. This is idempotent.
    pub fn cleanup(self: *@This()) void {
        if (self.state) |state| state.cleanup();
    }
    pub fn deinit(self: *@This()) void {
        self.cleanup();
        if (self.state) |state| {
            state.signal.deinit();
            state.allocator.free(state.listeners);
            state.allocator.destroy(state);
        }
        self.* = undefined;
    }
};
/// Inputs remain valid through cleanup. Zero inputs return no signal; one
/// reuses its signal; multiple inputs own a new signal and registrations.
/// First publication copies the reason into the output signal. The allocator
/// remains available through deinit and supports concurrent allocations.
pub fn combineAbortSignals(a: std.mem.Allocator, io: std.Io, signals: []const ?*AbortSignal) !CombinedAbortSignal {
    var count: usize = 0;
    var single: ?*AbortSignal = null;
    for (signals) |signal| if (signal) |value| {
        count += 1;
        single = value;
    };
    if (count < 2) return .{ .signal = single };
    const state = try a.create(CombinedAbortSignal.State);
    state.* = .{ .allocator = a, .io = io, .signal = .init(a) };
    errdefer {
        state.cleanup();
        state.signal.deinit();
        a.free(state.listeners);
        a.destroy(state);
    }
    state.listeners = try a.alloc(Listener, count);
    for (state.listeners) |*listener| listener.* = .{ .output = &state.signal };
    var index: usize = 0;
    for (signals) |signal| if (signal) |value| {
        value.mutex.lockUncancelable(io);
        defer value.mutex.unlock(io);
        if (value.event.isSet()) {
            try state.signal.abort(io, value.reason);
            break;
        }
        const listener = &state.listeners[index];
        index += 1;
        listener.input = value;
        value.listeners.append(&listener.node);
    };
    return .{ .signal = &state.signal, .state = state };
}
