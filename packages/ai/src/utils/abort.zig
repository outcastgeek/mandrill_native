//! Native promise/abort equivalents use explicit Io and an owned observed task.
const std = @import("std");
pub fn operationSignal(io: ?std.Io, fallback: std.Io) std.Io {
    return io orelse fallback;
}
/// The owner retains its producer context through deinit. Result allocations
/// borrow this operation's arena; clone retained values before releasing it.
/// A canceled waiter does not cancel or join the observed operation. One owner
/// joins/deinitializes it only after callers have stopped using its state.
pub fn ObservedOperation(comptime T: type) type {
    return struct {
        allocator: std.mem.Allocator,
        io: std.Io,
        state: *State,
        future: std.Io.Future(void),
        pub const Producer = struct { context: ?*anyopaque, call: *const fn (?*anyopaque, std.mem.Allocator, std.Io) anyerror!T };
        const State = struct {
            arena: std.heap.ArenaAllocator,
            io: std.Io,
            producer: Producer,
            done: std.Io.Event = .unset,
            result: anyerror!T = error.Canceled,
            fn run(self: *@This()) void {
                defer self.done.set(self.io);
                self.result = self.producer.call(self.producer.context, self.arena.allocator(), self.io);
            }
        };
        pub fn init(a: std.mem.Allocator, io: std.Io, producer: Producer) !@This() {
            const state = try a.create(State);
            errdefer a.destroy(state);
            state.* = .{ .arena = .init(a), .io = io, .producer = producer };
            errdefer state.arena.deinit();
            return .{ .allocator = a, .io = io, .state = state, .future = try io.concurrent(State.run, .{state}) };
        }
        pub fn wait(self: *@This(), io: std.Io) !T {
            try self.state.done.wait(io);
            try io.checkCancel();
            return self.state.result;
        }
        pub fn join(self: *@This()) !T {
            self.future.await(self.io);
            return self.state.result;
        }
        pub fn deinit(self: *@This()) void {
            self.future.await(self.io);
            self.state.arena.deinit();
            self.allocator.destroy(self.state);
            self.* = undefined;
        }
    };
}
pub fn raceWithAbortSignal(comptime T: type, operation: *ObservedOperation(T), io: std.Io) !T {
    return operation.wait(io);
}
