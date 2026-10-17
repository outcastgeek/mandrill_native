const std = @import("std");
const fx = @import("effect");
const gates = @import("support/dsl_gate.zig");
const E = enum { expected, release };
const Mode = enum { end, fail, die, interrupted, pending };
const R = struct {
    allocator: std.mem.Allocator,
    io: std.Io = std.testing.io,
    async_engine: fx.Executor = undefined,
    opened: *usize,
    closed: *usize,
    mode: Mode = .end,
    release_failure: bool = false,
    gate: ?*gates.Gate(fx.Chunk(u32), E) = null,
};
const Unit = fx.Effect(void, E, R);
const Acquire = fx.Effect(*Cursor, E, R);
const Pull = fx.Effect(fx.Chunk(u32), E, R);
const Cursor = struct { allocator: std.mem.Allocator, value: u32 = 0, maximum: u32 };
const Inputs = struct { maximum: u32, marker: u32 };
const Callbacks = struct {
    fn open(_: *const R) Acquire {
        return Acquire.fromOutcome(struct {
            fn run(env: *const R) fx.Exit(*Cursor, E) {
                const cursor = env.allocator.create(Cursor) catch return .{ .die = fx.Defect.out_of_memory };
                cursor.* = .{ .allocator = env.allocator, .maximum = 3 };
                env.opened.* += 1;
                return .{ .success = cursor };
            }
        }.run);
    }
    fn pull(cursor: *Cursor, _: *const R) Pull {
        return Pull.fromBorrowed(Cursor, cursor, struct {
            fn run(current: *const Cursor, env: *const R) fx.Exit(fx.Chunk(u32), E) {
                const value = @constCast(current);
                if (value.value == value.maximum) return .{ .success = .end };
                const items = env.allocator.alloc(u32, 1) catch return .{ .die = fx.Defect.out_of_memory };
                value.value += 1;
                items[0] = value.value;
                return .{ .success = fx.Chunk(u32).owned(.{ .items = items, .allocator = env.allocator }) };
            }
        }.run);
    }
    fn release(cursor: *Cursor, _: *const R) Unit {
        return Unit.fromBorrowed(Cursor, cursor, struct {
            fn run(current: *const Cursor, env: *const R) fx.Exit(void, E) {
                env.closed.* += 1;
                current.allocator.destroy(@constCast(current));
                return .{ .success = {} };
            }
        }.run);
    }
};
const Captured = struct {
    fn open(inputs: *const Inputs, _: *const R) Acquire {
        return Acquire.fromBorrowed(Inputs, inputs, struct {
            fn run(selected: *const Inputs, env: *const R) fx.Exit(*Cursor, E) {
                const cursor = env.allocator.create(Cursor) catch return .{ .die = fx.Defect.out_of_memory };
                cursor.* = .{ .allocator = env.allocator, .maximum = selected.maximum };
                env.opened.* += 1;
                return .{ .success = cursor };
            }
        }.run);
    }
    fn pull(inputs: *const Inputs, cursor: *Cursor, env: *const R) Pull {
        std.debug.assert(inputs.marker == 71);
        return Callbacks.pull(cursor, env);
    }
    fn release(inputs: *const Inputs, cursor: *Cursor, env: *const R) Unit {
        std.debug.assert(inputs.marker == 71);
        return Callbacks.release(cursor, env);
    }
};
fn check(allocator: std.mem.Allocator) !void {
    var opened: usize = 0;
    var closed: usize = 0;
    const env: R = .{ .allocator = allocator, .opened = &opened, .closed = &closed };
    const inputs: Inputs = .{ .maximum = 2, .marker = 71 };
    const source = fx.dsl.streamBorrowed(&inputs, .{ .open = Captured.open, .pull = Captured.pull, .release = Captured.release });
    try std.testing.expectEqual(@as(usize, 0), opened);
    var scope = fx.Scope(E).init(&env);
    defer scope.deinit();
    var opening = source.open(&scope);
    defer opening.deinit();
    var result = fx.run(&env, opening);
    defer if (result == .success) result.success.deinit();
    var close_result = fx.run(&env, Unit.closeScope(&scope));
    defer close_result.deinit();
    try std.testing.expectEqual(opened, closed);
    if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
    try std.testing.expect(result == .success);
}
test "STREAM-01 STREAM-03 COMP-03 named stream opening is lazy and readers isolate typed runtime inputs" {
    var opened: usize = 0;
    var closed: usize = 0;
    const env: R = .{ .allocator = std.testing.allocator, .opened = &opened, .closed = &closed };
    const source = comptime fx.dsl.stream(.{ .open = Callbacks.open, .pull = Callbacks.pull, .release = Callbacks.release });
    var scope = fx.Scope(E).init(&env);
    defer scope.deinit();
    var opening = source.open(&scope);
    defer opening.deinit();
    comptime if (@TypeOf(opening) != fx.Effect(fx.StreamReader(u32, E, R), E, R)) @compileError("opening must remain exact Effect");
    try std.testing.expectEqual(@as(usize, 0), opened);
    var first = fx.run(&env, opening);
    defer first.deinit();
    var second = fx.run(&env, opening);
    defer second.deinit();
    try std.testing.expect(first == .success and second == .success);
    for (0..2) |index| {
        var a = fx.run(&env, first.success.pull);
        defer a.deinit();
        var b = fx.run(&env, second.success.pull);
        defer b.deinit();
        try std.testing.expectEqual(@as(u32, @intCast(index + 1)), a.success.items.values[0]);
        try std.testing.expectEqual(a.success.items.values[0], b.success.items.values[0]);
        try std.testing.expectEqual(@as(usize, 0), closed);
    }
    var closing = fx.run(&env, Unit.closeScope(&scope));
    defer closing.deinit();
    try std.testing.expect(closing == .success);
    try std.testing.expectEqual(@as(usize, 2), closed);
}
test "STREAM-01 STREAM-06 named captured acquisition unwinds every reader allocation failure" {
    try check(std.testing.allocator);
    try std.testing.checkAllAllocationFailures(std.testing.allocator, check, .{});
}

const named = fx.dsl.stream(.{ .open = Callbacks.open, .pull = Callbacks.pull, .release = Callbacks.release });
fn checkPlan(allocator: std.mem.Allocator) !void {
    var opened: usize = 0;
    var closed: usize = 0;
    const env: R = .{ .allocator = allocator, .opened = &opened, .closed = &closed };
    const source = comptime fx.dsl.streams.append(.{ named, named });
    const Fold = struct {
        fn initialize(_: *const R) u32 {
            return 0;
        }
        fn step(state: *u32, value: *const u32, _: *const R) fx.SinkBuilder.Control {
            state.* += value.*;
            return .proceed;
        }
        fn finish(state: *u32, _: *const R) u32 {
            return state.*;
        }
    };
    const sink = comptime fx.SinkBuilder.fold(.{ .initialize = Fold.initialize, .step = Fold.step, .finish = Fold.finish });
    var result = fx.run(&env, comptime sink.run(source));
    defer result.deinit();
    try std.testing.expectEqual(opened, closed);
    if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
    try std.testing.expectEqual(@as(u32, 12), result.success);
    try std.testing.expectEqual(@as(usize, 2), opened);
}
test "STREAM-02 STREAM-04 append opens on demand and keeps each resource across yields" {
    var opened: usize = 0;
    var closed: usize = 0;
    const env: R = .{ .allocator = std.testing.allocator, .opened = &opened, .closed = &closed };
    const source = comptime fx.dsl.streams.append(.{ named, named });
    var scope = fx.Scope(E).init(&env);
    defer scope.deinit();
    var opening = source.open(&scope);
    defer opening.deinit();
    var reader = fx.run(&env, opening);
    defer reader.deinit();
    try std.testing.expectEqual(@as(usize, 0), opened);
    for (0..3) |_| {
        var chunk = fx.run(&env, reader.success.pull);
        defer chunk.deinit();
        try std.testing.expect(chunk == .success and chunk.success == .items);
        try std.testing.expectEqual(@as(usize, 1), opened);
        try std.testing.expectEqual(@as(usize, 0), closed);
    }
    var closing = fx.run(&env, Unit.closeScope(&scope));
    defer closing.deinit();
    try std.testing.expect(closing == .success);
    try std.testing.expectEqual(@as(usize, 1), closed);
    try std.testing.expectEqual(@as(usize, 1), opened);
}
test "STREAM-02 STREAM-06 append consumption and partial initialization survive every allocation failure" {
    try checkPlan(std.testing.allocator);
    try std.testing.checkAllAllocationFailures(std.testing.allocator, checkPlan, .{});
}

test "STREAM-02 yield bind branch and loop plans produce only demanded values" {
    const Number = fx.Effect(u32, E, R);
    const Yes = fx.Effect(bool, E, R);
    const Streams = fx.dsl.streams;
    const Single = comptime Streams.yieldEffect(Number.succeed(9));
    const Choice = comptime Streams.branch(Yes.succeed(true), Single, named);
    const Loop = struct {
        fn initialize(_: *const R) u32 {
            return 0;
        }
        fn next(state: *u32, _: *const R) ?fx.Stream(u32, E, R) {
            if (state.* == 2) return null;
            state.* += 1;
            return Choice;
        }
    };
    const source = comptime Streams.unfold(Loop.initialize, Loop.next);
    const Fold = struct {
        fn initialize(_: *const R) u32 {
            return 0;
        }
        fn step(state: *u32, item: *const u32, _: *const R) fx.SinkBuilder.Control {
            state.* += item.*;
            return .proceed;
        }
        fn finish(state: *u32, _: *const R) u32 {
            return state.*;
        }
    };
    var opened: usize = 0;
    var closed: usize = 0;
    const env: R = .{ .allocator = std.testing.allocator, .opened = &opened, .closed = &closed };
    const sink = comptime fx.SinkBuilder.fold(.{ .initialize = Fold.initialize, .step = Fold.step, .finish = Fold.finish });
    var result = fx.run(&env, comptime sink.run(source));
    defer result.deinit();
    try std.testing.expectEqual(@as(u32, 18), result.success);
    try std.testing.expectEqual(@as(usize, 0), opened);
}

const ManagedInputs = struct {
    inputs: Inputs,
    owner: struct {
        pub const effect_owned_payload = true;
        disposed: *usize,
        pub fn deinit(self: *@This()) void {
            self.disposed.* += 1;
        }
    },
};
const Managed = struct {
    fn open(inputs: *const ManagedInputs, env: *const R) Acquire {
        std.debug.assert(inputs.owner.disposed.* == 0);
        return Captured.open(&inputs.inputs, env);
    }
    fn pull(inputs: *const ManagedInputs, cursor: *Cursor, env: *const R) Pull {
        std.debug.assert(inputs.owner.disposed.* == 0);
        return Captured.pull(&inputs.inputs, cursor, env);
    }
    fn release(inputs: *const ManagedInputs, cursor: *Cursor, env: *const R) Unit {
        std.debug.assert(inputs.owner.disposed.* == 0);
        return Captured.release(&inputs.inputs, cursor, env);
    }
    fn body(source: fx.Stream(u32, E, R), env: *const R) Unit {
        return fx.dsl.streams.consume(env, source, .{ .consume = consume });
    }
    fn consume(item: *const u32, _: *const R) Unit {
        std.debug.assert(item.* > 0 and item.* <= 2);
        return Unit.succeed({});
    }
};
fn checkManaged(allocator: std.mem.Allocator) !void {
    var opened: usize = 0;
    var closed: usize = 0;
    var disposed: usize = 0;
    const env: R = .{ .allocator = allocator, .opened = &opened, .closed = &closed };
    var program = fx.dsl.streams.scoped(&env, ManagedInputs{ .inputs = .{ .maximum = 2, .marker = 71 }, .owner = .{ .disposed = &disposed } }, .{ .open = Managed.open, .pull = Managed.pull, .release = Managed.release, .body = Managed.body });
    var result = fx.run(&env, program);
    program.deinit();
    defer result.deinit();
    try std.testing.expectEqual(@as(usize, 1), disposed);
    try std.testing.expectEqual(opened, closed);
    if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
    try std.testing.expect(result == .success);
}
test "STREAM-01 STREAM-06 CAP-02 managed stream configuration needs no Clone and survives runtime consumption and OOM" {
    try checkManaged(std.testing.allocator);
    try std.testing.checkAllAllocationFailures(std.testing.allocator, checkManaged, .{});
}

fn checkMapped(allocator: std.mem.Allocator) !void {
    const Configuration = struct { factor: u64 };
    const Mapping = struct {
        fn map(inputs: *const Configuration, item: u32, _: *const R) u64 {
            return inputs.factor * item;
        }
        fn consume(item: *const u64, _: *const R) Unit {
            std.debug.assert(item.* == 3 or item.* == 6 or item.* == 9);
            return Unit.succeed({});
        }
        fn body(source: fx.Stream(u64, E, R), env: *const R) Unit {
            return fx.dsl.streams.consume(env, source, .{ .consume = consume });
        }
    };
    var opened: usize = 0;
    var closed: usize = 0;
    const env: R = .{ .allocator = allocator, .opened = &opened, .closed = &closed };
    var program = fx.dsl.streams.map(&env, named, Configuration{ .factor = 3 }, .{ .map = Mapping.map, .body = Mapping.body });
    defer program.deinit();
    try std.testing.expectEqual(@as(usize, 0), opened);
    var result = fx.run(&env, program);
    defer result.deinit();
    try std.testing.expectEqual(opened, closed);
    if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
    try std.testing.expect(result == .success);
}
test "STREAM-07 STREAM-06 captured pure mapping infers output and cleans partial chunks at every allocation failure" {
    try checkMapped(std.testing.allocator);
    try std.testing.checkAllAllocationFailures(std.testing.allocator, checkMapped, .{});
}

const Failing = struct {
    fn pull(_: *Cursor, env: *const R) Pull {
        return switch (env.mode) {
            .end => Pull.fromOutcome(struct {
                fn run(_: *const R) fx.Exit(fx.Chunk(u32), E) {
                    return .{ .success = .end };
                }
            }.run),
            .fail => Pull.fail(.expected),
            .die => Pull.die(.{ .kind = .invalid_configuration, .message = "pull defect" }),
            .interrupted => Pull.interrupted(),
            .pending => Pull.fromAsync(struct {
                fn start(current: *const R) fx.Async(fx.Chunk(u32), E).Start {
                    return current.gate.?.start();
                }
            }.start),
        };
    }
    fn release(cursor: *Cursor, _: *const R) Unit {
        return Unit.fromBorrowed(Cursor, cursor, struct {
            fn run(value: *const Cursor, env: *const R) fx.Exit(void, E) {
                if (env.gate) |gate| std.debug.assert(gate.destroyed.load(.acquire));
                env.closed.* += 1;
                value.allocator.destroy(@constCast(value));
                return if (env.release_failure) .{ .fail = .release } else .{ .success = {} };
            }
        }.run);
    }
};
const failing = fx.dsl.stream(.{ .open = Callbacks.open, .pull = Failing.pull, .release = Failing.release });
test "STREAM-05 stream plans preserve primary and release failures for every terminal channel" {
    @setEvalBranchQuota(10000);
    inline for (.{ Mode.end, Mode.fail, Mode.die, Mode.interrupted }) |mode| {
        inline for (.{ false, true }) |release_failure| {
            var opened: usize = 0;
            var closed: usize = 0;
            const env: R = .{ .allocator = std.testing.allocator, .opened = &opened, .closed = &closed, .mode = mode, .release_failure = release_failure };
            const source = comptime fx.dsl.streams.append(.{failing});
            var result = fx.run(&env, comptime source.runCollect(1));
            defer result.deinit();
            try std.testing.expectEqual(@as(usize, 1), opened);
            try std.testing.expectEqual(@as(usize, 1), closed);
            if (release_failure) {
                try std.testing.expect(result == .finalization_failed);
                try std.testing.expect(result.finalization_failed.first == .release_failure);
                try std.testing.expectEqual(E.release, result.finalization_failed.first.release_failure);
                switch (mode) {
                    .end => try std.testing.expect(result.finalization_failed.primary == .succeeded),
                    .fail => try std.testing.expect(result.finalization_failed.primary == .failed and result.finalization_failed.primary.failed == .expected),
                    .die => try std.testing.expect(result.finalization_failed.primary == .defected),
                    .interrupted => try std.testing.expect(result.finalization_failed.primary == .was_interrupted),
                    else => unreachable,
                }
            } else switch (mode) {
                .end => try std.testing.expect(result == .success),
                .fail => try std.testing.expect(result == .fail and result.fail == .expected),
                .die => try std.testing.expect(result == .die),
                .interrupted => try std.testing.expect(result == .interrupted),
                else => unreachable,
            }
        }
    }
}
test "STREAM-05 LIFE-12 cancellation drains a pending plan pull before release and skips appended work" {
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = true }, .maximum_owned = 4 };
    defer engine.deinit();
    var gate: gates.Gate(fx.Chunk(u32), E) = .{ .result = .{ .success = .end } };
    var opened: usize = 0;
    var closed: usize = 0;
    const env: R = .{ .allocator = std.testing.allocator, .opened = &opened, .closed = &closed, .async_engine = engine.executor(), .mode = .pending, .gate = &gate };
    const source = comptime fx.dsl.streams.append(.{ failing, named });
    var pending = fx.runAsync(&env, comptime source.runCollect(3));
    defer pending.deinit();
    gate.started.waitUncancelable(env.io);
    try std.testing.expectEqual(@as(usize, 1), opened);
    try std.testing.expectEqual(@as(usize, 0), closed);
    pending.request();
    var result = pending.join();
    defer result.deinit();
    try std.testing.expect(result == .interrupted);
    try std.testing.expect(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
    try std.testing.expectEqual(@as(usize, 1), opened);
    try std.testing.expectEqual(@as(usize, 1), closed);
}

fn checkCaptureSlot(allocator: std.mem.Allocator) !void {
    var opened: usize = 0;
    var closed: usize = 0;
    const env: R = .{ .allocator = allocator, .opened = &opened, .closed = &closed };
    var scope = fx.Scope(E).init(&env);
    defer scope.deinit();
    const PullInputs = struct { marker: u32 };
    const original: PullInputs = .{ .marker = 79 };
    const CursorAcquire = fx.Effect(*Cursor, E, R);
    const acquisition = comptime CursorAcquire.deferEffect(Callbacks.open);
    var acquired = acquisition.scopeAcquire(&scope, Callbacks.release);
    var opening = fx.Stream(u32, E, R).openAcquiredCaptured(&acquired, env.allocator, &original, struct {
        fn pull(inputs: *const *const PullInputs, _: *Cursor, _: *const R) Pull {
            return Pull.fromBorrowed(*const PullInputs, inputs, struct {
                fn read(stored: *const *const PullInputs, _: *const R) fx.Exit(fx.Chunk(u32), E) {
                    std.debug.assert(stored.*.marker == 79);
                    return .{ .success = .end };
                }
            }.read);
        }
    }.pull);
    var result = fx.run(&env, opening);
    opening.deinit();
    defer result.deinit();
    if (result != .success) {
        var closing = fx.run(&env, Unit.closeScope(&scope));
        defer closing.deinit();
        try std.testing.expectEqual(opened, closed);
        if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
        return error.UnexpectedResult;
    }
    try std.testing.expect(result == .success);
    var pulled = fx.run(&env, result.success.pull);
    defer pulled.deinit();
    var closing = fx.run(&env, Unit.closeScope(&scope));
    defer closing.deinit();
    try std.testing.expectEqual(@as(usize, 1), closed);
    if (pulled == .die and pulled.die.kind == .out_of_memory) return error.OutOfMemory;
    try std.testing.expect(pulled == .success and pulled.success == .end);
}
test "CAP-04 STREAM-01 STREAM-06 generated pull capture outlives opening and survives every allocation failure" {
    try checkCaptureSlot(std.testing.allocator);
    try std.testing.checkAllAllocationFailures(std.testing.allocator, checkCaptureSlot, .{});
}

fn checkErrorMapping(allocator: std.mem.Allocator) !void {
    const Mapping = struct {
        fn map(value: E) u16 {
            return if (value == .expected) 31 else 37;
        }
    };
    const source = comptime failing.mapError(Mapping.map);
    var opened: usize = 0;
    var closed: usize = 0;
    const env: R = .{ .allocator = allocator, .opened = &opened, .closed = &closed, .mode = .fail, .release_failure = true };
    var result = fx.run(&env, comptime source.runCollect(1));
    defer result.deinit();
    try std.testing.expectEqual(opened, closed);
    if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
    if (result == .finalization_failed and result.finalization_failed.primary == .defected and result.finalization_failed.primary.defected.kind == .out_of_memory) return error.OutOfMemory;
    try std.testing.expect(result == .finalization_failed);
    try std.testing.expectEqual(@as(u16, 31), result.finalization_failed.primary.failed);
    try std.testing.expectEqual(@as(u16, 37), result.finalization_failed.first.release_failure);
}
test "STREAM-07 STREAM-06 error mapping preserves pull and cleanup failures through allocation failures" {
    try checkErrorMapping(std.testing.allocator);
    try std.testing.checkAllAllocationFailures(std.testing.allocator, checkErrorMapping, .{});
}

test "STREAM-01 STREAM-02 named iterator and loop infer state and release every iteration" {
    const Iteration = struct {
        fn next(cursor: *Cursor, _: *const R) fx.Exit(?u32, E) {
            if (cursor.value == cursor.maximum) return .{ .success = null };
            cursor.value += 1;
            return .{ .success = cursor.value };
        }
    };
    const iterator = comptime fx.dsl.streams.iterator(.{ .open = Callbacks.open, .next = Iteration.next, .release = Callbacks.release });
    const Loop = struct {
        fn initialize(_: *const R) u32 {
            return 0;
        }
        fn condition(state: *const u32, _: *const R) bool {
            return state.* < 2;
        }
        fn body(_: *u32, _: *const R) fx.Stream(u32, E, R) {
            return iterator;
        }
        fn advance(state: *u32, _: *const R) void {
            state.* += 1;
        }
    };
    const source = comptime fx.dsl.streams.loop(.{ .initialize = Loop.initialize, .condition = Loop.condition, .body = Loop.body, .advance = Loop.advance });
    var opened: usize = 0;
    var closed: usize = 0;
    const env: R = .{ .allocator = std.testing.allocator, .opened = &opened, .closed = &closed };
    var result = fx.run(&env, comptime source.runCollect(6));
    defer result.deinit();
    try std.testing.expect(result == .success);
    try std.testing.expectEqualSlices(u32, &.{ 1, 2, 3, 1, 2, 3 }, result.success.items);
    try std.testing.expectEqual(@as(usize, 2), opened);
    try std.testing.expectEqual(opened, closed);
}

fn checkCapturedErrorMapping(allocator: std.mem.Allocator) !void {
    const Mapping = struct {
        fn map(offset: *const u16, err: E) u16 {
            return offset.* + if (err == .expected) @as(u16, 31) else 37;
        }
        fn consume(_: *const u32, _: *const R) fx.Effect(void, u16, R) {
            return fx.Effect(void, u16, R).succeed({});
        }
        fn body(source: fx.Stream(u32, u16, R), env: *const R) fx.Effect(void, u16, R) {
            return fx.dsl.streams.consume(env, source, .{ .consume = consume });
        }
    };
    var opened: usize = 0;
    var closed: usize = 0;
    const env: R = .{ .allocator = allocator, .opened = &opened, .closed = &closed, .mode = .fail, .release_failure = true };
    var program = fx.dsl.streams.mapError(&env, failing, @as(u16, 10), .{ .map_error = Mapping.map, .body = Mapping.body });
    defer program.deinit();
    var result = fx.run(&env, program);
    defer result.deinit();
    try std.testing.expectEqual(opened, closed);
    if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
    if (result == .finalization_failed and result.finalization_failed.primary == .defected and result.finalization_failed.primary.defected.kind == .out_of_memory) return error.OutOfMemory;
    try std.testing.expect(result == .finalization_failed);
    try std.testing.expectEqual(@as(u16, 41), result.finalization_failed.primary.failed);
    try std.testing.expectEqual(@as(u16, 47), result.finalization_failed.first.release_failure);
}
test "STREAM-07 captured error mapping preserves the reader and primary cleanup channel across OOM" {
    try checkCapturedErrorMapping(std.testing.allocator);
    try std.testing.checkAllAllocationFailures(std.testing.allocator, checkCapturedErrorMapping, .{});
}

fn checkProjection(allocator: std.mem.Allocator) !void {
    const Large = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, inner: *const R, observed: *usize };
    const Projection = struct {
        fn project(marker: *const u32, env: *const Large) *const R {
            std.debug.assert(marker.* == 83);
            return env.inner;
        }
        fn consume(item: *const u32, env: *const Large) fx.Effect(void, E, Large) {
            env.observed.* += item.*;
            return fx.Effect(void, E, Large).succeed({});
        }
        fn body(source: fx.Stream(u32, E, Large), env: *const Large) fx.Effect(void, E, Large) {
            return fx.dsl.streams.consume(env, source, .{ .consume = consume });
        }
    };
    var opened: usize = 0;
    var closed: usize = 0;
    var observed: usize = 0;
    const inner: R = .{ .allocator = allocator, .opened = &opened, .closed = &closed };
    const outer: Large = .{ .allocator = allocator, .inner = &inner, .observed = &observed };
    var program = fx.dsl.streams.project(&outer, named, @as(u32, 83), .{ .project = Projection.project, .body = Projection.body });
    defer program.deinit();
    comptime if (@TypeOf(program) != fx.Effect(void, E, Large)) @compileError("stream projection must expose exact inferred Effect");
    var result = fx.run(&outer, program);
    defer result.deinit();
    try std.testing.expectEqual(opened, closed);
    if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(usize, 6), observed);
}
test "STREAM-07 captured environment projection preserves source scope providers and survives every allocation failure" {
    try checkProjection(std.testing.allocator);
    try std.testing.checkAllAllocationFailures(std.testing.allocator, checkProjection, .{});
}
