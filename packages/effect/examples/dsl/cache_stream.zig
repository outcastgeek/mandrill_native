//! EX-03: scope one cache, read it through a pull-driven stream, and fold prices.
const std = @import("std");
const fx = @import("effect");
const Error = enum { unavailable };
const R = struct { allocator: std.mem.Allocator, io: std.Io, async_engine: fx.Executor, loaded: *std.atomic.Value(usize), closed: *usize };
const Number = fx.Effect(u32, Error, R);
const Unit = fx.Effect(void, Error, R);
const Pull = fx.Effect(fx.Chunk(u32), Error, R);
const Cache = fx.Cache(u32, u32, Error);
const Operations = fx.EffectCache(u32, u32, Error, R);
const Reader = struct { allocator: std.mem.Allocator, cache: *Cache, next: u32 = 1, value: [1]u32 = undefined, fail_at: u32 };
const Roles = struct {
    fn configure(_: anytype, _: *const R) fx.CacheConfig(u32, Error) {
        return .{ .keys = .{ .userdata = null, .equal = equal, .retain = retain, .release = releaseKey }, .time_to_live = null, .capacity = 4, .maximum_inflight = 2, .cache_failure = cacheFailure };
    }
    fn equal(_: ?*anyopaque, a: u32, b: u32) bool {
        return a == b;
    }
    fn retain(_: ?*anyopaque, value: u32) error{OutOfMemory}!u32 {
        return value;
    }
    fn releaseKey(_: ?*anyopaque, _: *u32) void {}
    fn cacheFailure(_: Error) bool {
        return false;
    }
    fn load(input: anytype, value: u32, env: *const R) Number {
        _ = env.loaded.fetchAdd(1, .monotonic);
        return Number.succeed(value * input.factor);
    }
    fn open(input: anytype, _: *const R) fx.Effect(*Reader, Error, R) {
        return fx.Dsl(Error, R).outcomeBorrowed(input, struct {
            fn allocate(config: @TypeOf(input), current: *const R) fx.Exit(*Reader, Error) {
                const reader = current.allocator.create(Reader) catch return .{ .die = fx.Defect.out_of_memory };
                reader.* = .{ .allocator = current.allocator, .cache = config.cache, .fail_at = config.fail_at };
                return .{ .success = reader };
            }
        }.allocate);
    }
    fn pull(_: anytype, reader: *Reader, env: *const R) Pull {
        if (reader.next == reader.fail_at) return Pull.fromOwnedOutcome(env, .{ .fail = .unavailable });
        if (reader.next == 5) return Pull.fromOwnedOutcome(env, .{ .success = .end });
        const next = reader.next;
        reader.next += 1;
        var lookup = Operations.get(reader.cache, next);
        return lookup.flatMapCapturedOwned(env, fx.Chunk(u32), reader, struct {
            fn found(pointer: *const *Reader, value: u32, current: *const R) Pull {
                pointer.*.value[0] = value;
                return Pull.fromOwnedOutcome(current, .{ .success = fx.Chunk(u32).borrowed(&pointer.*.value) });
            }
        }.found);
    }
    fn close(_: anytype, reader: *Reader, _: *const R) Unit {
        return Unit.fromBorrowed(Reader, reader, struct {
            fn dispose(value: *const Reader, env: *const R) fx.Exit(void, Error) {
                env.closed.* += 1;
                value.allocator.destroy(@constCast(value));
                return .{ .success = {} };
            }
        }.dispose);
    }
    fn initialize(_: *const R) u32 {
        return 0;
    }
    fn step(total: *u32, value: *const u32, _: *const R) fx.SinkBuilder.Control {
        total.* += value.*;
        return if (total.* >= 9) .stop else .proceed;
    }
    fn finish(total: *u32, _: *const R) u32 {
        return total.*;
    }
    fn consume(source: anytype, env: *const R) Number {
        return fx.SinkBuilder.fold(.{ .initialize = initialize, .step = step, .finish = finish }).runWith(env, source);
    }
    fn body(input: anytype, scope: *fx.Scope(Error), env: *const R) Number {
        var making = fx.dsl.makeCache(env, scope, .{ .factor = input.factor }, .{ .configure = configure, .load = load });
        return making.flatMapCapturedOwned(env, u32, input, struct {
            fn made(config: *const @TypeOf(input), cache: *Cache, current: *const R) Number {
                return fx.dsl.streams.scoped(current, .{ .cache = cache, .fail_at = config.*.fail_at }, .{ .open = open, .pull = pull, .release = close, .body = consume });
            }
        }.made);
    }
};
pub fn prices(env: *const R, input: anytype) Number {
    return fx.dsl.scopedCaptured(env, input, .{ .body = Roles.body });
}
fn check(allocator: std.mem.Allocator, io: std.Io) !void {
    var engine: fx.NativeIoExecutor = .{ .allocator = allocator, .io = io, .capabilities = .{ .independent = true, .cpu = false, .blocking = false }, .maximum_owned = 4 };
    defer engine.deinit();
    var loaded: std.atomic.Value(usize) = .init(0);
    var closed: usize = 0;
    const env: R = .{ .allocator = allocator, .io = io, .async_engine = engine.executor(), .loaded = &loaded, .closed = &closed };
    for ([_]u32{ 0, 2 }) |fail_at| {
        var description = prices(&env, .{ .factor = @as(u32, 3), .fail_at = fail_at });
        var result = fx.run(&env, description);
        description.deinit();
        defer result.deinit();
        if (fail_at == 0) try std.testing.expect(result == .success and result.success == 9) else try std.testing.expect(result == .fail and result.fail == .unavailable);
    }
    try std.testing.expectEqual(@as(usize, 2), closed);
    try std.testing.expectEqual(@as(usize, 3), loaded.load(.acquire));
}
pub fn main(init: std.process.Init) !void {
    try check(init.gpa, init.io);
}
test "EX-03 scoped cached prices fold a stream with early close and preserve pull failures" {
    try check(std.testing.allocator, std.testing.io);
}
