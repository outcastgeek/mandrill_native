const std = @import("std");
const fx = @import("effect");
const gates = @import("support/dsl_gate.zig");
const E = enum { expected, cleanup };
const C = fx.SinkBuilder.Control;
const Env = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, async_engine: fx.Executor, step_gate: *gates.Gate(C, E), finish_gate: *gates.Gate(void, E), alive: *std.atomic.Value(usize), readers: *std.atomic.Value(usize) };
const Unit = fx.Effect(void, E, Env);
const Payload = struct {
    pub const effect_owned_payload = true;
    allocator: std.mem.Allocator,
    bytes: []u8,
    alive: *std.atomic.Value(usize),
    pub fn deinit(self: *@This()) void {
        self.allocator.free(self.bytes);
        _ = self.alive.fetchSub(1, .acq_rel);
    }
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        const bytes = try self.allocator.dupe(u8, self.bytes);
        _ = self.alive.fetchAdd(1, .acq_rel);
        return .{ .allocator = self.allocator, .bytes = bytes, .alive = self.alive };
    }
};
const Iterator = struct { allocator: std.mem.Allocator, readers: *std.atomic.Value(usize), done: bool = false };
const Adapter = struct {
    fn open(env: *const Env) fx.Exit(*Iterator, E) {
        const item = env.allocator.create(Iterator) catch return .{ .die = fx.Defect.out_of_memory };
        item.* = .{ .allocator = env.allocator, .readers = env.readers };
        _ = env.readers.fetchAdd(1, .acq_rel);
        return .{ .success = item };
    }
    fn next(item: *Iterator, _: *const Env) fx.Exit(?u32, E) {
        if (item.done) return .{ .success = null };
        item.done = true;
        return .{ .success = 1 };
    }
    fn close(item: *Iterator) Unit {
        return Unit.fromBorrowed(Iterator, item, struct {
            fn run(value: *const Iterator, _: *const Env) fx.Exit(void, E) {
                _ = value.readers.fetchSub(1, .acq_rel);
                value.allocator.destroy(@constCast(value));
                return .{ .success = {} };
            }
        }.run);
    }
    fn initialize(_: *const Env) fx.Effect(Payload, E, Env) {
        return fx.Effect(Payload, E, Env).fromOutcome(struct {
            fn run(env: *const Env) fx.Exit(Payload, E) {
                const bytes = env.allocator.dupe(u8, "pending result") catch return .{ .die = fx.Defect.out_of_memory };
                _ = env.alive.fetchAdd(1, .acq_rel);
                return .{ .success = .{ .allocator = env.allocator, .bytes = bytes, .alive = env.alive } };
            }
        }.run);
    }
    fn step(_: *Payload, _: *const u32, _: *const Env) fx.Effect(C, E, Env) {
        return fx.Effect(C, E, Env).fromAsync(struct {
            fn start(env: *const Env) fx.Async(C, E).Start {
                return env.step_gate.start();
            }
        }.start);
    }
    fn before(_: *Payload, _: *const Env) Unit {
        return Unit.fromAsync(struct {
            fn start(env: *const Env) fx.Async(void, E).Start {
                return env.finish_gate.start();
            }
        }.start);
    }
};
const source = fx.Stream(u32, E, Env).fromIterable(Iterator, Adapter.open, Adapter.next, Adapter.close);
const sink = fx.SinkBuilder.fold(.{ .initialize = Adapter.initialize, .step = Adapter.step, .finish = fx.SinkBuilder.takeStateAfter(Adapter.before) });

test "SINK-02 LIFE-11 pending transitions and final result retain owners through controlled resume" {
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = true }, .maximum_owned = 4 };
    defer engine.deinit();
    var step: gates.Gate(C, E) = .{ .result = .{ .success = .proceed } };
    var finish: gates.Gate(void, E) = .{ .result = .{ .success = {} } };
    var alive: std.atomic.Value(usize) = .init(0);
    var readers: std.atomic.Value(usize) = .init(0);
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor(), .step_gate = &step, .finish_gate = &finish, .alive = &alive, .readers = &readers };
    var pending = fx.runAsync(&env, comptime sink.run(source));
    defer pending.deinit();
    step.started.waitUncancelable(env.io);
    try std.testing.expectEqual(@as(usize, 1), alive.load(.acquire));
    try std.testing.expectEqual(@as(usize, 1), readers.load(.acquire));
    step.proceed.set(env.io);
    finish.started.waitUncancelable(env.io);
    try std.testing.expectEqual(@as(usize, 1), alive.load(.acquire));
    finish.proceed.set(env.io);
    var result = pending.join();
    defer result.deinit();
    try std.testing.expectEqualStrings("pending result", result.success.bytes);
    try std.testing.expectEqual(@as(usize, 0), readers.load(.acquire));
    try std.testing.expect(step.destroyed.load(.acquire) and finish.destroyed.load(.acquire));
}

test "SINK-03 LIFE-12 cancellation joins pending transition before reader and state cleanup" {
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = true }, .maximum_owned = 4 };
    defer engine.deinit();
    var step: gates.Gate(C, E) = .{ .result = .{ .success = .proceed } };
    var finish: gates.Gate(void, E) = .{ .result = .{ .success = {} } };
    var alive: std.atomic.Value(usize) = .init(0);
    var readers: std.atomic.Value(usize) = .init(0);
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor(), .step_gate = &step, .finish_gate = &finish, .alive = &alive, .readers = &readers };
    var pending = fx.runAsync(&env, comptime sink.run(source));
    defer pending.deinit();
    step.started.waitUncancelable(env.io);
    pending.request();
    var result = pending.join();
    defer result.deinit();
    try std.testing.expect(result == .interrupted);
    try std.testing.expect(step.joined.load(.acquire) and step.destroyed.load(.acquire));
    try std.testing.expect(!finish.joined.load(.acquire));
    try std.testing.expectEqual(@as(usize, 0), alive.load(.acquire));
    try std.testing.expectEqual(@as(usize, 0), readers.load(.acquire));
}
