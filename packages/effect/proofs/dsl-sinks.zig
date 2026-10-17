const std = @import("std");
const fx = @import("effect");
const E = enum { expected, cleanup };
const R = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, count: usize, pulls: *usize, released: *usize };
const Unit = fx.Effect(void, E, R);
const Stream = fx.Stream(u32, E, R);
const C = fx.SinkBuilder.Control;
const Iterator = struct { allocator: std.mem.Allocator, count: usize, next: usize = 0, pulls: *usize, released: *usize };
fn open(env: *const R) fx.Exit(*Iterator, E) {
    const iterator = env.allocator.create(Iterator) catch return .{ .die = fx.Defect.out_of_memory };
    iterator.* = .{ .allocator = env.allocator, .count = env.count, .pulls = env.pulls, .released = env.released };
    return .{ .success = iterator };
}
fn next(iterator: *Iterator, _: *const R) fx.Exit(?u32, E) {
    iterator.pulls.* += 1;
    if (iterator.next == iterator.count) return .{ .success = null };
    iterator.next += 1;
    return .{ .success = @intCast(iterator.next) };
}
fn close(iterator: *Iterator) Unit {
    return Unit.fromBorrowed(Iterator, iterator, struct {
        fn run(value: *const Iterator, _: *const R) fx.Exit(void, E) {
            value.released.* += 1;
            value.allocator.destroy(@constCast(value));
            return .{ .success = {} };
        }
    }.run);
}
const source = Stream.fromIterable(Iterator, open, next, close);
const Fold = struct {
    fn initialize(_: *const R) u64 {
        return 0;
    }
    fn step(state: *u64, item: *const u32, _: *const R) C {
        state.* += item.*;
        return .proceed;
    }
    fn finish(state: *u64, _: *const R) u64 {
        return state.*;
    }
};

test "SINK-01 SINK-04 SINK-05 COMP-03 inferred pure fold has fresh state and empty semantics" {
    const sink = comptime fx.SinkBuilder.fold(.{ .initialize = Fold.initialize, .step = Fold.step, .finish = Fold.finish });
    const program = comptime sink.run(source);
    comptime {
        if (@TypeOf(program) != fx.Effect(u64, E, R)) @compileError("fold must expose exact Effect");
    }
    var pulls: usize = 0;
    var released: usize = 0;
    var env: R = .{ .allocator = std.testing.allocator, .count = 4, .pulls = &pulls, .released = &released };
    for (0..2) |_| try std.testing.expectEqual(@as(u64, 10), fx.run(&env, program).success);
    env.count = 0;
    try std.testing.expectEqual(@as(u64, 0), fx.run(&env, program).success);
    try std.testing.expectEqual(@as(usize, 3), released);
}

test "SINK-04 fold early stop issues no additional pull" {
    const Early = struct {
        fn step(state: *u64, item: *const u32, _: *const R) C {
            state.* += item.*;
            return .stop;
        }
    };
    const sink = comptime fx.SinkBuilder.fold(.{ .initialize = Fold.initialize, .step = Early.step, .finish = Fold.finish });
    var pulls: usize = 0;
    var released: usize = 0;
    const env: R = .{ .allocator = std.testing.allocator, .count = 10, .pulls = &pulls, .released = &released };
    try std.testing.expectEqual(@as(u64, 1), fx.run(&env, comptime sink.run(source)).success);
    try std.testing.expectEqual(@as(usize, 1), pulls);
    try std.testing.expectEqual(@as(usize, 1), released);
}

test "SINK-06 LIFE-07 fold allocation rollback closes initialized readers and state" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            var pulls: usize = 0;
            var released: usize = 0;
            const env: R = .{ .allocator = allocator, .count = 3, .pulls = &pulls, .released = &released };
            const sink = comptime fx.SinkBuilder.fold(.{ .initialize = Fold.initialize, .step = Fold.step, .finish = Fold.finish });
            var result = fx.run(&env, comptime sink.run(source));
            defer result.deinit();
            if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expectEqual(@as(u64, 6), result.success);
            try std.testing.expectEqual(@as(usize, 1), released);
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

const Owned = struct {
    pub const effect_owned_payload = true;
    allocator: std.mem.Allocator,
    bytes: []u8,
    released: *usize,
    pub fn deinit(self: *@This()) void {
        self.allocator.free(self.bytes);
        self.released.* += 1;
    }
};
const OwningFold = struct {
    fn initialize(_: *const R) fx.Effect(Owned, E, R) {
        return fx.Effect(Owned, E, R).fromOutcome(struct {
            fn run(current: *const R) fx.Exit(Owned, E) {
                const bytes = current.allocator.dupe(u8, "result") catch return .{ .die = fx.Defect.out_of_memory };
                return .{ .success = .{ .allocator = current.allocator, .bytes = bytes, .released = current.released } };
            }
        }.run);
    }
    fn step(_: *Owned, _: *const u32, _: *const R) fx.Effect(C, E, R) {
        return fx.Effect(C, E, R).deferEffect(struct {
            fn later(_: *const R) fx.Effect(C, E, R) {
                return fx.Effect(C, E, R).succeed(.proceed);
            }
        }.later);
    }
    fn before(_: *Owned, _: *const R) Unit {
        return Unit.deferEffect(struct {
            fn later(_: *const R) Unit {
                return Unit.succeed({});
            }
        }.later);
    }
};

test "SINK-02 LIFE-11 owning state transfers after deferred step and finalizer and survives frame cleanup" {
    var pulls: usize = 0;
    var released: usize = 0;
    const env: R = .{ .allocator = std.testing.allocator, .count = 2, .pulls = &pulls, .released = &released };
    const sink = comptime fx.SinkBuilder.fold(.{ .initialize = OwningFold.initialize, .step = OwningFold.step, .finish = fx.SinkBuilder.takeStateAfter(OwningFold.before) });
    var result = fx.run(&env, comptime sink.run(source));
    try std.testing.expect(result == .success);
    try std.testing.expectEqualStrings("result", result.success.bytes);
    // Iterator has closed, but the transferred state belongs to the result.
    try std.testing.expectEqual(@as(usize, 1), released);
    result.deinit();
    try std.testing.expectEqual(@as(usize, 2), released);
}

test "SINK-03 owning state is cleaned when deferred final-result callback fails" {
    const Failure = struct {
        fn before(_: *Owned, _: *const R) Unit {
            return Unit.fail(.expected);
        }
    };
    var pulls: usize = 0;
    var released: usize = 0;
    const env: R = .{ .allocator = std.testing.allocator, .count = 2, .pulls = &pulls, .released = &released };
    const sink = comptime fx.SinkBuilder.fold(.{ .initialize = OwningFold.initialize, .step = OwningFold.step, .finish = fx.SinkBuilder.takeStateAfter(Failure.before) });
    var result = fx.run(&env, comptime sink.run(source));
    defer result.deinit();
    try std.testing.expect(result == .fail and result.fail == .expected);
    try std.testing.expectEqual(@as(usize, 2), released);
}

test "SINK-02 LIFE-11 generated field transfer releases only remaining composite state" {
    const State = struct { answer: Owned, scratch: Owned };
    const Composite = struct {
        fn initialize(_: *const R) fx.Effect(State, E, R) {
            return fx.Effect(State, E, R).fromOutcome(struct {
                fn run(env: *const R) fx.Exit(State, E) {
                    const answer = env.allocator.dupe(u8, "answer") catch return .{ .die = fx.Defect.out_of_memory };
                    const scratch = env.allocator.dupe(u8, "scratch") catch {
                        env.allocator.free(answer);
                        return .{ .die = fx.Defect.out_of_memory };
                    };
                    return .{ .success = .{ .answer = .{ .allocator = env.allocator, .bytes = answer, .released = env.released }, .scratch = .{ .allocator = env.allocator, .bytes = scratch, .released = env.released } } };
                }
            }.run);
        }
        fn step(_: *State, _: *const u32, _: *const R) C {
            return .proceed;
        }
    };
    var pulls: usize = 0;
    var released: usize = 0;
    const env: R = .{ .allocator = std.testing.allocator, .count = 1, .pulls = &pulls, .released = &released };
    const folded = comptime fx.SinkBuilder.fold(.{ .initialize = Composite.initialize, .step = Composite.step, .finish = fx.SinkBuilder.takeField("answer") });
    var result = fx.run(&env, comptime folded.run(source));
    try std.testing.expectEqualStrings("answer", result.success.bytes);
    try std.testing.expectEqual(@as(usize, 2), released); // reader + scratch
    result.deinit();
    try std.testing.expectEqual(@as(usize, 3), released);
}
