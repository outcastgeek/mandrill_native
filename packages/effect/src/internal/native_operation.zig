const diagnostics = @import("diagnostics.zig");
const std = @import("std");
const outcomes = @import("../outcome.zig");
const Cancellation = @import("runtime.zig").Cancellation;

/// Bridge a logical token to a native I/O operation without sharing Future
/// ownership. Only this invocation awaits/cancels its own Future.
pub fn run(env: anytype, capture: anytype, token: ?*const Cancellation, comptime callback: anytype) @typeInfo(@TypeOf(callback)).@"fn".return_type.? {
    const Exit = @typeInfo(@TypeOf(callback)).@"fn".return_type.?;
    const Task = struct {
        env: @TypeOf(env),
        capture: @TypeOf(capture),
        result: Exit = .interrupted,
        done: std.atomic.Value(bool) = .init(false),
        wake: std.Io.Event = .unset,
        fn work(self: *@This()) void {
            self.result = callback(self.capture, self.env);
            self.done.store(true, .release);
            self.wake.set(self.env.io);
        }
        fn request(data: ?*anyopaque) void {
            const self: *@This() = @ptrCast(@alignCast(data.?));
            self.wake.set(self.env.io);
        }
    };
    if (token) |origin| if (origin.isRequested()) return .interrupted;
    var task: Task = .{ .env = env, .capture = capture };
    var registration: Cancellation.Registration = .{ .data = &task, .forward = Task.request };
    if (token) |origin| origin.register(&registration);
    defer registration.unregister();
    var future = env.io.concurrent(Task.work, .{&task}) catch return .{ .die = .{ .kind = .provider_unavailable, .message = diagnostics.native_operation_requires_independent_i_o_progress } };
    var cancelled = false;
    task.wake.wait(env.io) catch {
        cancelled = true;
    };
    if (token) |origin| cancelled = cancelled or origin.isRequested();
    const protection = env.io.swapCancelProtection(.blocked);
    defer _ = env.io.swapCancelProtection(protection);
    if (cancelled and !task.done.load(.acquire)) future.cancel(env.io) else future.await(env.io);
    // A late owning success must reach acquireRelease/scopeAcquire. Those
    // operators install ownership before publishing the parent's interruption.
    return task.result;
}
