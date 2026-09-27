const diagnostics = @import("internal/diagnostics.zig");
const std = @import("std");
const outcomes = @import("outcome.zig");

/// Native command boundary. Construction starts the command inline. A pending
/// command owns all provider resources until its single join owner completes.
pub fn Async(comptime A: type, comptime E: type) type {
    return struct {
        const Self = @This();
        pub const Start = union(enum) { completed: outcomes.Exit(A, E), pending: Operation };
        pub const JoinResult = struct { exit: outcomes.Exit(A, E), interrupted: bool = false };
        pub const Operation = struct {
            userdata: *anyopaque,
            vtable: *const VTable,

            pub const VTable = struct {
                // Nonblocking, threadsafe cancellation forwarding. It cannot
                // call await/cancel concurrently on an owned std.Io.Future.
                request: *const fn (*anyopaque) ?outcomes.Defect,
                join: *const fn (*anyopaque, bool) JoinResult,
                destroy: *const fn (*anyopaque) void,
            };

            pub fn request(self: Operation) ?outcomes.Defect {
                return self.vtable.request(self.userdata);
            }
            pub fn join(self: Operation) JoinResult {
                return self.joinProtected(false);
            }
            pub fn joinProtected(self: Operation, protected: bool) JoinResult {
                return self.vtable.join(self.userdata, protected);
            }
            pub fn deinit(self: *Operation) void {
                self.vtable.destroy(self.userdata);
                self.* = undefined;
            }
        };

        pub fn nativeBorrowed(env: anytype, capture: anytype, comptime callback: fn (@TypeOf(capture), @TypeOf(env)) outcomes.Exit(A, E)) Start {
            return nativeCancellable(env, capture, struct {
                fn invoke(value: @TypeOf(capture), environment: @TypeOf(env), _: ?*const @import("internal/runtime.zig").Cancellation) outcomes.Exit(A, E) {
                    return callback(value, environment);
                }
            }.invoke);
        }

        pub fn nativeCancellable(env: anytype, capture: anytype, comptime callback: fn (@TypeOf(capture), @TypeOf(env), ?*const @import("internal/runtime.zig").Cancellation) outcomes.Exit(A, E)) Start {
            const EnvPointer = @TypeOf(env);
            const Capture = @TypeOf(capture);
            const Task = struct {
                allocator: std.mem.Allocator,
                io: std.Io,
                env: EnvPointer,
                capture: Capture,
                future: std.Io.Future(void) = undefined,
                result: outcomes.Exit(A, E) = .interrupted,
                cancellation: @import("internal/runtime.zig").Cancellation = .{},
                completed: std.atomic.Value(bool) = .init(false),
                requested: std.atomic.Value(bool) = .init(false),
                wake: std.Io.Event = .unset,
                joined: bool = false,
                fn work(self: *@This()) void {
                    self.result = callback(self.capture, self.env, &self.cancellation);
                    self.completed.store(true, .release);
                    self.wake.set(self.io);
                }
                fn request(data: *anyopaque) ?outcomes.Defect {
                    const self: *@This() = @ptrCast(@alignCast(data));
                    self.requested.store(true, .release);
                    self.cancellation.request();
                    self.wake.set(self.io);
                    return null;
                }
                fn join(data: *anyopaque, protected: bool) JoinResult {
                    const self: *@This() = @ptrCast(@alignCast(data));
                    std.debug.assert(!self.joined);
                    const previous = if (protected) self.io.swapCancelProtection(.blocked) else null;
                    defer if (previous) |value| {
                        _ = self.io.swapCancelProtection(value);
                    };
                    var interrupted = false;
                    self.wake.wait(self.io) catch {
                        interrupted = true;
                        self.requested.store(true, .release);
                        self.cancellation.request();
                    };
                    const protection = self.io.swapCancelProtection(.blocked);
                    defer _ = self.io.swapCancelProtection(protection);
                    if (self.requested.load(.acquire) and !self.completed.load(.acquire)) self.future.cancel(self.io) else self.future.await(self.io);
                    self.joined = true;
                    return .{ .exit = self.result, .interrupted = interrupted };
                }
                fn destroy(data: *anyopaque) void {
                    const self: *@This() = @ptrCast(@alignCast(data));
                    std.debug.assert(self.joined);
                    self.allocator.destroy(self);
                }
            };
            const task = env.allocator.create(Task) catch return .{ .completed = .{ .die = outcomes.Defect.out_of_memory } };
            task.* = .{ .allocator = env.allocator, .io = env.io, .env = env, .capture = capture };
            task.future = task.io.concurrent(Task.work, .{task}) catch {
                task.allocator.destroy(task);
                return .{ .completed = .{ .die = .{ .kind = .provider_unavailable, .message = diagnostics.native_asynchronous_operation_requires_independent_progress } } };
            };
            if (task.future.any_future == null) {
                const result = task.result;
                task.allocator.destroy(task);
                return .{ .completed = result };
            }
            return .{ .pending = .{ .userdata = task, .vtable = &.{ .request = Task.request, .join = Task.join, .destroy = Task.destroy } } };
        }

        /// Environment and callback captures remain borrowed through join. The
        /// native provider decides whether work completes inline or suspends.
        pub fn native(env: anytype, comptime callback: fn (@TypeOf(env)) outcomes.Exit(A, E)) Start {
            const EnvPointer = @TypeOf(env);
            const Task = struct {
                allocator: std.mem.Allocator,
                io: std.Io,
                env: EnvPointer,
                future: std.Io.Future(void) = undefined,
                outcome: outcomes.Exit(A, E) = .interrupted,
                completed: std.atomic.Value(bool) = .init(false),
                requested: std.atomic.Value(bool) = .init(false),
                wake: std.Io.Event = .unset,
                joined: bool = false,

                fn run(self: *@This()) void {
                    self.outcome = callback(self.env);
                    self.completed.store(true, .release);
                    self.wake.set(self.io);
                }

                fn request(data: *anyopaque) ?outcomes.Defect {
                    const task: *@This() = @ptrCast(@alignCast(data));
                    task.requested.store(true, .release);
                    task.wake.set(task.io);
                    return null;
                }

                fn join(data: *anyopaque, protected: bool) JoinResult {
                    const task: *@This() = @ptrCast(@alignCast(data));
                    std.debug.assert(!task.joined);
                    const previous = if (protected) task.io.swapCancelProtection(.blocked) else null;
                    defer if (previous) |value| {
                        _ = task.io.swapCancelProtection(value);
                    };
                    var interrupted = false;
                    task.wake.wait(task.io) catch {
                        interrupted = true;
                        task.requested.store(true, .release);
                    };
                    const protection = task.io.swapCancelProtection(.blocked);
                    defer _ = task.io.swapCancelProtection(protection);
                    if (task.requested.load(.acquire) and !task.completed.load(.acquire)) task.future.cancel(task.io) else task.future.await(task.io);
                    task.joined = true;
                    return .{ .exit = task.outcome, .interrupted = interrupted };
                }

                fn destroy(data: *anyopaque) void {
                    const task: *@This() = @ptrCast(@alignCast(data));
                    std.debug.assert(task.joined);
                    task.allocator.destroy(task);
                }
            };
            const task = env.allocator.create(Task) catch return .{ .completed = .{ .die = outcomes.Defect.out_of_memory } };
            task.* = .{ .allocator = env.allocator, .io = env.io, .env = env };
            task.future = task.io.concurrent(Task.run, .{task}) catch {
                task.allocator.destroy(task);
                return .{ .completed = .{ .die = .{ .kind = .provider_unavailable, .message = diagnostics.native_wakeable_asynchronous_operation_requires_independent_progress } } };
            };
            if (task.future.any_future == null) {
                const result = task.outcome;
                task.allocator.destroy(task);
                return .{ .completed = result };
            }
            return .{ .pending = .{ .userdata = task, .vtable = &.{ .request = Task.request, .join = Task.join, .destroy = Task.destroy } } };
        }
    };
}
