//! Shared native execution through Zig's `std.Io` concurrency surface.

const std = @import("std");
pub const cancellation = @import("interop/cancel.zig");

var init_mutex: std.atomic.Mutex = .unlocked;
var initialized: std.atomic.Value(bool) = .init(false);
var threaded: std.Io.Threaded = undefined;
var detached_group: std.Io.Group = .init;

fn lock(mutex: *std.atomic.Mutex) void {
    while (!mutex.tryLock()) std.atomic.spinLoopHint();
}

/// Returns the process-lifetime native execution context. The pool is created
/// once and reused by every FFI environment and native workflow.
pub fn shared() std.Io {
    if (initialized.load(.acquire)) return threaded.io();
    lock(&init_mutex);
    defer init_mutex.unlock();
    if (!initialized.load(.monotonic)) {
        threaded = std.Io.Threaded.init(std.heap.c_allocator, .{});
        initialized.store(true, .release);
    }
    return threaded.io();
}

/// A scoped heterogeneous task group. Tasks write into caller-owned result
/// storage and every successful submission is joined or cancelled before that
/// storage is released.
pub const Scope = struct {
    io: std.Io,
    group: std.Io.Group = .init,
    open: bool = true,

    pub fn init(io: std.Io) Scope {
        return .{ .io = io };
    }

    pub fn concurrent(
        self: *Scope,
        comptime function: anytype,
        arguments: std.meta.ArgsTuple(@TypeOf(function)),
    ) std.Io.ConcurrentError!void {
        std.debug.assert(self.open);
        try self.group.concurrent(self.io, function, arguments);
    }

    pub fn join(self: *Scope) std.Io.Cancelable!void {
        std.debug.assert(self.open);
        self.open = false;
        return self.group.await(self.io);
    }

    pub fn cancel(self: *Scope) void {
        if (!self.open) return;
        self.open = false;
        self.group.cancel(self.io);
    }
};

/// Runs an arbitrary homogeneous batch. `max_in_flight` bounds concurrent work
/// without limiting the total task count. Results retain input index order.
pub fn concurrentMap(
    comptime Input: type,
    comptime Output: type,
    allocator: std.mem.Allocator,
    io: std.Io,
    inputs: []const Input,
    outputs: []Output,
    max_in_flight: usize,
    comptime operation: fn (Input) anyerror!Output,
) anyerror!void {
    if (outputs.len != inputs.len) return error.OutputLengthMismatch;
    if (inputs.len == 0) return;
    if (max_in_flight == 0) return error.InvalidConcurrencyBound;

    const window_size = @min(max_in_flight, inputs.len);
    var futures = try allocator.alloc(std.Io.Future(anyerror!Output), window_size);
    defer allocator.free(futures);

    var base: usize = 0;
    while (base < inputs.len) {
        const count = @min(window_size, inputs.len - base);
        var submitted: usize = 0;
        while (submitted < count) : (submitted += 1) {
            futures[submitted] = io.concurrent(operation, .{inputs[base + submitted]}) catch |failure| {
                for (futures[0..submitted]) |*future| _ = future.await(io) catch {};
                return failure;
            };
        }

        var first_error: ?anyerror = null;
        for (futures[0..count], 0..) |*future, index| {
            outputs[base + index] = future.await(io) catch |failure| {
                if (first_error == null) first_error = failure;
                continue;
            };
        }
        if (first_error) |failure| return failure;
        base += count;
    }
}

/// Submits process-lifetime detached work to the shared executor. Individual
/// task resources are released when the task returns; the group lives with the
/// native library and is never exposed through an FFI boundary.
pub fn submit(comptime function: anytype, arguments: std.meta.ArgsTuple(@TypeOf(function))) std.Io.ConcurrentError!void {
    // Threaded.groupConcurrent protects its queue and atomically counts group
    // members. A second, spinning admission lock only adds contention.
    try detached_group.concurrent(shared(), function, arguments);
}

/// Completion-driven bounded CPU batches. Workers claim independent inputs and
/// return when exhausted; no coordinator waits on futures. Context and slices
/// remain owned by the caller through `complete`, which runs exactly once after
/// every started worker returns from consumer work. Cancellation is persistent
/// consumer state and must also be checked inside long-running `run` functions.
pub fn AsyncBatch(
    comptime Context: type,
    comptime Input: type,
    comptime Output: type,
    comptime run: fn (*Context, Input) anyerror!Output,
    comptime cancelled: fn (*Context) bool,
    comptime complete: fn (*Context, ?anyerror) void,
) type {
    return struct {
        const Self = @This();
        allocator: std.mem.Allocator,
        context: *Context,
        inputs: []const Input,
        outputs: []Output,
        next: std.atomic.Value(usize) = .init(0),
        references: std.atomic.Value(usize) = .init(1),
        failure: std.atomic.Value(u16) = .init(0),

        pub fn start(allocator: std.mem.Allocator, context: *Context, inputs: []const Input, outputs: []Output, bound: usize) !void {
            if (inputs.len != outputs.len) return error.OutputLengthMismatch;
            if (inputs.len != 0 and bound == 0) return error.InvalidConcurrencyBound;
            const self = try allocator.create(Self);
            self.* = .{ .allocator = allocator, .context = context, .inputs = inputs, .outputs = outputs };
            for (0..@min(bound, inputs.len)) |_| {
                _ = self.references.fetchAdd(1, .monotonic);
                submit(worker, .{self}) catch |err| {
                    self.fail(err);
                    self.release();
                    break;
                };
            }
            self.release();
        }

        fn fail(self: *Self, err: anyerror) void {
            _ = self.failure.cmpxchgStrong(0, @intFromError(err), .acq_rel, .acquire);
        }

        fn release(self: *Self) void {
            if (self.references.fetchSub(1, .acq_rel) != 1) return;
            const failure = self.failure.load(.acquire);
            const context = self.context;
            self.allocator.destroy(self);
            complete(context, if (failure == 0) null else @errorFromInt(failure));
        }

        fn worker(self: *Self) void {
            defer self.release();
            while (self.failure.load(.acquire) == 0) {
                if (cancelled(self.context)) {
                    self.fail(error.Canceled);
                    return;
                }
                const index = self.next.fetchAdd(1, .monotonic);
                if (index >= self.inputs.len) return;
                self.outputs[index] = run(self.context, self.inputs[index]) catch |err| {
                    self.fail(err);
                    return;
                };
            }
        }
    };
}

/// Transfers an owned context to detached work. The generated worker runs the
/// consumer operation and cleanup exactly once, including context destruction.
pub fn submitOwned(
    comptime Context: type,
    value: Context,
    comptime operation: fn (*Context) void,
    comptime cleanup: fn (*Context) void,
) (std.mem.Allocator.Error || std.Io.ConcurrentError)!void {
    const allocator = std.heap.c_allocator;
    const context = try allocator.create(Context);
    errdefer allocator.destroy(context);
    context.* = value;
    try submit(struct {
        fn run(owned: *Context) std.Io.Cancelable!void {
            defer allocator.destroy(owned);
            defer cleanup(owned);
            operation(owned);
        }
    }.run, .{context});
}

pub fn submitOwnedNoCleanup(
    comptime Context: type,
    value: Context,
    comptime operation: fn (*Context) void,
) (std.mem.Allocator.Error || std.Io.ConcurrentError)!void {
    return submitOwned(Context, value, operation, struct {
        fn cleanup(_: *Context) void {}
    }.cleanup);
}

test "completion-driven batch joins by callbacks and cooperatively cancels" {
    const Context = struct {
        finished: std.Io.Event = .unset,
        stop: std.atomic.Value(bool) = .init(false),
        completed: std.atomic.Value(u32) = .init(0),
        failure: ?anyerror = null,

        fn run(_: *@This(), input: u32) anyerror!u32 {
            return input * 3;
        }
        fn cancelled(self: *@This()) bool {
            return self.stop.load(.acquire);
        }
        fn done(self: *@This(), failure: ?anyerror) void {
            self.failure = failure;
            _ = self.completed.fetchAdd(1, .acq_rel);
            self.finished.set(shared());
        }
    };
    const Batch = AsyncBatch(Context, u32, u32, Context.run, Context.cancelled, Context.done);
    for ([_]usize{ 0, 1, 2, 8, 16, 32, 19 }) |count| {
        var context: Context = .{};
        const inputs = try std.testing.allocator.alloc(u32, count);
        defer std.testing.allocator.free(inputs);
        const outputs = try std.testing.allocator.alloc(u32, count);
        defer std.testing.allocator.free(outputs);
        for (inputs, 0..) |*input, index| input.* = @intCast(index);
        try Batch.start(std.testing.allocator, &context, inputs, outputs, 3);
        // Only the synchronous test driver waits, never an async coordinator.
        context.finished.waitUncancelable(shared());
        try std.testing.expectEqual(@as(?anyerror, null), context.failure);
        try std.testing.expectEqual(@as(u32, 1), context.completed.load(.acquire));
        for (outputs, 0..) |output, index| try std.testing.expectEqual(@as(u32, @intCast(index * 3)), output);
    }
    var context: Context = .{};
    context.stop.store(true, .release);
    const inputs = [_]u32{ 1, 2, 3 };
    var outputs: [3]u32 = undefined;
    try Batch.start(std.testing.allocator, &context, &inputs, &outputs, 2);
    context.finished.waitUncancelable(shared());
    try std.testing.expectEqual(error.Canceled, context.failure.?);
    try std.testing.expectEqual(@as(u32, 1), context.completed.load(.acquire));
}

test "arbitrary homogeneous batches preserve result identity and nesting" {
    var local = std.Io.Threaded.init(std.testing.allocator, .{});
    defer local.deinit();
    const io = local.io();
    const operation = struct {
        fn add(value: u32) anyerror!u32 {
            return value + 1;
        }
    }.add;
    inline for (.{ 0, 1, 2, 8, 16, 32 }) |count| {
        var inputs: [count]u32 = undefined;
        var outputs: [count]u32 = undefined;
        for (&inputs, 0..) |*input, index| input.* = @intCast(index);
        try concurrentMap(u32, u32, std.testing.allocator, io, &inputs, &outputs, 4, operation);
        for (outputs, 0..) |output, index| try std.testing.expectEqual(@as(u32, @intCast(index + 1)), output);
    }

    var outer_inputs = [_]u32{ 2, 4, 6 };
    var outer_outputs: [outer_inputs.len]u32 = undefined;
    const nested = struct {
        fn run(value: u32) anyerror!u32 {
            var inner_inputs = [_]u32{ value, value + 1 };
            var inner_outputs: [2]u32 = undefined;
            try concurrentMap(u32, u32, std.heap.c_allocator, shared(), &inner_inputs, &inner_outputs, 2, operation);
            return inner_outputs[0] + inner_outputs[1];
        }
    }.run;
    try concurrentMap(u32, u32, std.testing.allocator, io, &outer_inputs, &outer_outputs, 3, nested);
    try std.testing.expectEqualSlices(u32, &.{ 7, 11, 15 }, &outer_outputs);

    const runtime_count: usize = 11;
    const dynamic_inputs = try std.testing.allocator.alloc(u32, runtime_count);
    defer std.testing.allocator.free(dynamic_inputs);
    const dynamic_outputs = try std.testing.allocator.alloc(u32, runtime_count);
    defer std.testing.allocator.free(dynamic_outputs);
    for (dynamic_inputs, 0..) |*input, index| input.* = @intCast(index * 2);
    try concurrentMap(u32, u32, std.testing.allocator, io, dynamic_inputs, dynamic_outputs, 3, operation);
    for (dynamic_outputs, 0..) |output, index| try std.testing.expectEqual(@as(u32, @intCast(index * 2 + 1)), output);
}

test "heterogeneous scope joins distinct result storage" {
    var local = std.Io.Threaded.init(std.testing.allocator, .{});
    defer local.deinit();
    var scope = Scope.init(local.io());
    var integer: u32 = 0;
    var record: struct {
        name: []const u8 = "",
        flags: [2]bool = .{ false, false },
        nested: struct { count: u32 = 0 } = .{},
    } = .{};
    try scope.concurrent(struct {
        fn run(output: *u32) std.Io.Cancelable!void {
            output.* = 42;
        }
    }.run, .{&integer});
    try scope.concurrent(struct {
        fn run(output: *@TypeOf(record)) std.Io.Cancelable!void {
            output.* = .{ .name = "native", .flags = .{ true, false }, .nested = .{ .count = 7 } };
        }
    }.run, .{&record});
    try scope.join();
    try std.testing.expectEqual(@as(u32, 42), integer);
    try std.testing.expectEqualStrings("native", record.name);
    try std.testing.expect(record.flags[0]);
    try std.testing.expectEqual(@as(u32, 7), record.nested.count);
}

test "bounded batches join task errors and never exceed the selected bound" {
    var local = std.Io.Threaded.init(std.testing.allocator, .{});
    defer local.deinit();
    var active: std.atomic.Value(u32) = .init(0);
    var maximum: std.atomic.Value(u32) = .init(0);
    var completed: std.atomic.Value(u32) = .init(0);
    const Input = struct {
        value: u32,
        active: *std.atomic.Value(u32),
        maximum: *std.atomic.Value(u32),
        completed: *std.atomic.Value(u32),
        fail: bool,
    };
    const operation = struct {
        fn run(input: Input) anyerror!u32 {
            const current = input.active.fetchAdd(1, .acq_rel) + 1;
            defer _ = input.active.fetchSub(1, .acq_rel);
            defer _ = input.completed.fetchAdd(1, .acq_rel);
            var observed = input.maximum.load(.acquire);
            while (current > observed)
                observed = input.maximum.cmpxchgWeak(observed, current, .acq_rel, .acquire) orelse break;
            // A completed worker decrements active; rendezvous must observe a
            // persistent arrival condition rather than miss the brief value 3.
            while (input.maximum.load(.acquire) < 3) std.atomic.spinLoopHint();
            if (input.fail) return error.ExpectedTaskFailure;
            return input.value;
        }
    }.run;
    var inputs: [8]Input = undefined;
    var outputs: [8]u32 = undefined;
    for (&inputs, 0..) |*input, index| input.* = .{
        .value = @intCast(index),
        .active = &active,
        .maximum = &maximum,
        .completed = &completed,
        .fail = index == 1,
    };
    try std.testing.expectError(error.ExpectedTaskFailure, concurrentMap(Input, u32, std.testing.allocator, local.io(), &inputs, &outputs, 3, operation));
    try std.testing.expectEqual(@as(u32, 3), completed.load(.acquire));
    try std.testing.expectEqual(@as(u32, 3), maximum.load(.acquire));
}

test "partial submission failure joins every task that started" {
    var local = std.Io.Threaded.init(std.testing.allocator, .{ .concurrent_limit = .limited(2) });
    defer local.deinit();
    const io = local.io();
    var completed: std.atomic.Value(u32) = .init(0);
    const Input = struct { io: std.Io, completed: *std.atomic.Value(u32), value: u32 };
    const operation = struct {
        fn run(input: Input) anyerror!u32 {
            defer _ = input.completed.fetchAdd(1, .acq_rel);
            _ = try input.io.sleep(.fromMilliseconds(20), .awake);
            return input.value;
        }
    }.run;
    const inputs = [_]Input{
        .{ .io = io, .completed = &completed, .value = 1 },
        .{ .io = io, .completed = &completed, .value = 2 },
        .{ .io = io, .completed = &completed, .value = 3 },
        .{ .io = io, .completed = &completed, .value = 4 },
    };
    var outputs: [inputs.len]u32 = undefined;
    try std.testing.expectError(error.ConcurrencyUnavailable, concurrentMap(Input, u32, std.testing.allocator, io, &inputs, &outputs, inputs.len, operation));
    try std.testing.expectEqual(@as(u32, 2), completed.load(.acquire));
}

test "scoped cancellation joins submitted work before returning" {
    var local = std.Io.Threaded.init(std.testing.allocator, .{});
    defer local.deinit();
    const io = local.io();
    var event: std.Io.Event = .unset;
    var started: std.atomic.Value(bool) = .init(false);
    var finished: std.atomic.Value(bool) = .init(false);
    var scope = Scope.init(io);
    try scope.concurrent(struct {
        fn wait(task_io: std.Io, signal: *std.Io.Event, did_start: *std.atomic.Value(bool), did_finish: *std.atomic.Value(bool)) std.Io.Cancelable!void {
            did_start.store(true, .release);
            defer did_finish.store(true, .release);
            try signal.wait(task_io);
        }
    }.wait, .{ io, &event, &started, &finished });
    while (!started.load(.acquire)) std.atomic.spinLoopHint();
    scope.cancel();
    try std.testing.expect(finished.load(.acquire));
}

test "detached owned context cleanup runs exactly once" {
    const Context = struct { cleaned: *std.atomic.Value(u32) };
    var cleaned: std.atomic.Value(u32) = .init(0);
    try submitOwned(Context, .{ .cleaned = &cleaned }, struct {
        fn run(_: *Context) void {}
    }.run, struct {
        fn cleanup(context: *Context) void {
            _ = context.cleaned.fetchAdd(1, .acq_rel);
        }
    }.cleanup);
    while (cleaned.load(.acquire) == 0) std.atomic.spinLoopHint();
    try std.testing.expectEqual(@as(u32, 1), cleaned.load(.acquire));
}
