//! Allocation/runtime pairs use identical application work and native providers.
const std = @import("std");
const fx = @import("effect");
const R = struct { allocator: std.mem.Allocator, io: std.Io, input: u32 };
const P = fx.Effect(u32, void, R);
fn require(condition: bool) void {
    if (!condition) @panic("performance fixture semantic or ownership mismatch");
}
const Mode = enum { owned_direct, owned_dsl, frame_direct, frame_dsl, stream_direct, stream_dsl, pending_direct, pending_dsl };
fn read(env: *const R) u32 {
    return env.input;
}
fn add(value: u32) u32 {
    return value +% 1;
}
fn then(value: u32) P {
    return P.succeed(add(value));
}
fn binding(_: *const R) P {
    return P.sync(read).flatMap(then);
}
fn last(value: *const u32, _: *const R) P {
    return P.succeed(value.*);
}
const increment = .{ .increment = @as(u32, 1) };
fn captured(input: *const @TypeOf(increment), value: u32, _: *const R) P {
    return P.succeed(value +% input.increment);
}
const Iterator = struct { allocator: std.mem.Allocator, index: u32 = 0 };
fn open(env: *const R) fx.Exit(*Iterator, void) {
    const iterator = env.allocator.create(Iterator) catch return .{ .die = fx.Defect.out_of_memory };
    iterator.* = .{ .allocator = env.allocator };
    return .{ .success = iterator };
}
fn openEffect(_: *const R) fx.Effect(*Iterator, void, R) {
    return fx.Effect(*Iterator, void, R).fromOutcome(open);
}
fn next(iterator: *Iterator, _: *const R) fx.Exit(?u32, void) {
    if (iterator.index == 4) return .{ .success = null };
    iterator.index += 1;
    return .{ .success = iterator.index };
}
fn close(iterator: *Iterator) fx.Effect(void, void, R) {
    return fx.Effect(void, void, R).fromBorrowed(Iterator, iterator, struct {
        fn finish(value: *const Iterator, _: *const R) fx.Exit(void, void) {
            value.allocator.destroy(@constCast(value));
            return .{ .success = {} };
        }
    }.finish);
}
fn initial(_: *const R) u32 {
    return 0;
}
fn step(state: *u32, value: *const u32, _: *const R) fx.SinkBuilder.Control {
    state.* += value.*;
    return .proceed;
}
fn finish(state: *u32, _: *const R) u32 {
    return state.*;
}
const sink = fx.SinkBuilder.fold(.{ .initialize = initial, .step = step, .finish = finish });
const native_stream = fx.Stream(u32, void, R).fromIterable(Iterator, open, next, close);
const dsl_stream = fx.dsl.streams.iterator(.{ .open = openEffect, .next = next, .release = close });
const pending_vtable: fx.Async(u32, void).Operation.VTable = .{
    .request = struct {
        fn request(_: *anyopaque) ?fx.Defect {
            return null;
        }
    }.request,
    .join = struct {
        fn join(data: *anyopaque, _: bool) fx.Async(u32, void).JoinResult {
            const value: *const u32 = @ptrCast(@alignCast(data));
            return .{ .exit = .{ .success = value.* } };
        }
    }.join,
    .destroy = struct {
        fn destroy(_: *anyopaque) void {}
    }.destroy,
};
fn start(env: *const R) fx.Async(u32, void).Start {
    return .{ .pending = .{ .userdata = @ptrCast(@constCast(&env.input)), .vtable = &pending_vtable } };
}
fn make(comptime mode: Mode, env: *const R) P {
    return switch (mode) {
        .owned_direct => blk: {
            var source = P.sync(read);
            var mapped = source.mapOwned(env, add);
            break :blk mapped.flatMapCapturedOwned(env, u32, increment, captured);
        },
        .owned_dsl => blk: {
            var source = P.sync(read);
            break :blk fx.dsl.pipeline.owned(&source, env, .{ fx.dsl.pipeline.map(add), fx.dsl.pipeline.captured(increment, captured) });
        },
        .frame_direct => comptime P.deferEffect(binding),
        .frame_dsl => comptime fx.dsl.program.describe(.{ fx.dsl.program.binding("value", .{}, binding), fx.dsl.program.binding("result", .{"value"}, last) }),
        .stream_direct => comptime sink.run(native_stream),
        .stream_dsl => comptime sink.run(dsl_stream),
        .pending_direct => comptime P.fromAsync(start),
        .pending_dsl => comptime fx.Dsl(void, R).async(.{ .run = start }),
    };
}
const Measurement = struct { allocations: usize, allocated_bytes: usize, freed_bytes: usize, elapsed_ns: i96, checksum: u32 };
fn measure(comptime mode: Mode, allocator: std.mem.Allocator, io: std.Io, iterations: usize) Measurement {
    var counter = std.testing.FailingAllocator.init(allocator, .{});
    var env: R = .{ .allocator = counter.allocator(), .io = io, .input = 17 };
    var checksum: u32 = 0;
    const started = std.Io.Timestamp.now(io, .awake);
    for (0..iterations) |_| {
        std.mem.doNotOptimizeAway(&env);
        var program = make(mode, &env);
        var result = fx.run(&env, program);
        require(result == .success);
        const expected: u32 = switch (mode) {
            .stream_direct, .stream_dsl => 10,
            .owned_direct, .owned_dsl => env.input +% 2,
            .frame_direct, .frame_dsl => env.input +% 1,
            else => env.input,
        };
        require(result.success == expected);
        checksum +%= result.success;
        env.input +%= 1;
        result.deinit();
        program.deinit();
    }
    return .{ .allocations = counter.allocations, .allocated_bytes = counter.allocated_bytes, .freed_bytes = counter.freed_bytes, .elapsed_ns = started.durationTo(std.Io.Timestamp.now(io, .awake)).nanoseconds, .checksum = checksum };
}
pub fn main(init: std.process.Init) void {
    const iterations: usize = 1000;
    std.debug.print("{{\"iterations\":{d},\"pairs\":[", .{iterations});
    inline for (comptime std.meta.tags(Mode), 0..) |mode, index| {
        const result = measure(mode, init.gpa, init.io, iterations);
        require(result.allocated_bytes == result.freed_bytes);
        if (index != 0) std.debug.print(",", .{});
        std.debug.print("{{\"name\":\"{s}\",\"allocations\":{d},\"allocated_bytes\":{d},\"elapsed_ns\":{d},\"checksum\":{d}}}", .{ @tagName(mode), result.allocations, result.allocated_bytes, result.elapsed_ns, result.checksum });
    }
    std.debug.print("]}}\n", .{});
}
fn checkOom(allocator: std.mem.Allocator, comptime mode: Mode) !void {
    const env: R = .{ .allocator = allocator, .io = std.testing.io, .input = 17 };
    var program = make(mode, &env);
    var result = fx.run(&env, program);
    program.deinit();
    defer result.deinit();
    if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
    try std.testing.expect(result == .success);
}
test "PERF-03 PERF-05 representative direct and DSL allocation pairs execute equivalent work with complete reclamation" {
    inline for (comptime std.meta.tags(Mode)) |mode| {
        const result = measure(mode, std.testing.allocator, std.testing.io, 4);
        try std.testing.expectEqual(result.allocated_bytes, result.freed_bytes);
    }
}
test "PERF-06 optimized owning pipelines generated frames and stream folds keep every allocation failure rollback" {
    inline for (comptime std.meta.tags(Mode)) |mode| {
        try std.testing.checkAllAllocationFailures(std.testing.allocator, struct {
            fn check(allocator: std.mem.Allocator) !void {
                return checkOom(allocator, mode);
            }
        }.check, .{});
    }
}
