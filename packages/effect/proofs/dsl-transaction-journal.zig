const std = @import("std");
const fx = @import("effect");
const T = fx.TransactionBuilder;
const E = enum { expected };
const Mapped = enum { mapped };
const R = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, domain: *fx.TransactionDomain };
fn select(env: *const R) *fx.TransactionDomain {
    return env.domain;
}
const Callbacks = struct {
    fn fail(_: void) @TypeOf(T.fail(u32, E, .expected)) {
        return T.fail(u32, E, .expected);
    }
    fn retry(_: void) @TypeOf(T.retry(u32, E)) {
        return T.retry(u32, E);
    }
    fn read(capture: anytype, _: u32) @TypeOf(T.read(u32, E, capture.cell)) {
        return T.read(u32, E, capture.cell);
    }
    fn mapError(_: E) Mapped {
        return .mapped;
    }
};
fn check(allocator: std.mem.Allocator) !void {
    const runtime = .{ .allocator = allocator, .io = std.testing.io };
    var domain = fx.TransactionDomain.init(&runtime);
    defer domain.deinit();
    var cell = fx.TransactionRef(u32).init(&domain, 7);
    var other = fx.TransactionRef(u32).init(&domain, 13);
    const env: R = .{ .allocator = allocator, .domain = &domain };
    const written_then_failed = T.bind(T.write(u32, E, &cell, 99), Callbacks.fail);
    const failed = T.mapError(written_then_failed, Callbacks.mapError);
    var failed_exit = fx.run(&env, T.atomic(&failed, select));
    defer failed_exit.deinit();
    if (failed_exit == .die and failed_exit.die.kind == .out_of_memory) return error.OutOfMemory;
    try std.testing.expect(failed_exit == .fail and failed_exit.fail == .domain and failed_exit.fail.domain == .mapped);
    try std.testing.expectEqual(@as(u32, 7), cell.value);
    const trying = T.bind(T.write(u32, E, &cell, 99), Callbacks.retry);
    const alternative = T.orElse(trying, T.read(u32, E, &cell));
    const last = T.bindCaptured(alternative, .{ .cell = &cell }, Callbacks.read);
    const zipped = T.zip(last, T.read(u32, E, &other));
    var result = fx.run(&env, T.atomic(&zipped, select));
    defer result.deinit();
    if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(u32, 7), result.success.left);
    try std.testing.expectEqual(@as(u32, 13), result.success.right);
    try std.testing.expectEqual(@as(u32, 7), cell.value);
    try std.testing.expectEqual(@as(u32, 13), other.value);
}
test "TX-01 TX-05 LIFE-07 captured bind zip alternatives and mapped failures preserve rollback and one journal at every allocation failure" {
    try check(std.testing.allocator);
    try std.testing.checkAllAllocationFailures(std.testing.allocator, check, .{});
}

const Conflict = struct {
    const Counts = struct { attempts: usize = 0, starts: usize = 0, finishes: usize = 0 };
    const Capture = struct { cell: *fx.TransactionRef(u32), counts: *Counts, ready: *std.Io.Event, proceed: *std.Io.Event, io: std.Io };
    fn evaluate(capture: *const Capture, journal: *fx.TransactionDomain.Journal) fx.TransactionAtomicDecision(u32, E) {
        const Local = struct {
            counts: *Counts,
            value: u32 = 0,
            fn deinit(self: *@This()) void {
                self.counts.finishes += 1;
            }
        };
        capture.counts.attempts += 1;
        capture.counts.starts += 1;
        var local: Local = .{ .counts = capture.counts };
        defer local.deinit();
        std.debug.assert(local.value == 0);
        const read = journal.read(u32, capture.cell);
        if (read != .ok) {
            capture.ready.set(capture.io);
            return .retry;
        }
        local.value = read.ok;
        journal.write(u32, capture.cell, local.value + 1);
        if (capture.counts.attempts == 1) {
            capture.ready.set(capture.io);
            capture.proceed.waitUncancelable(capture.io);
        }
        return .{ .commit = local.value + 1 };
    }
    fn writer(env: *const R, cell: *fx.TransactionRef(u32), ready: *std.Io.Event, proceed: *std.Io.Event) void {
        ready.waitUncancelable(env.io);
        const update = T.write(u32, E, cell, 41);
        var result = fx.run(env, T.atomic(&update, select));
        defer result.deinit();
        std.debug.assert(result == .success);
        proceed.set(env.io);
    }
};
test "TX-02 TX-06 a real competing commit restarts the inferred native description with fresh local state and one captured cell identity" {
    const runtime = .{ .allocator = std.testing.allocator, .io = std.testing.io };
    var domain = fx.TransactionDomain.init(&runtime);
    defer domain.deinit();
    var cell = fx.TransactionRef(u32).init(&domain, 7);
    const env: R = .{ .allocator = std.testing.allocator, .domain = &domain };
    var counts: Conflict.Counts = .{};
    var ready: std.Io.Event = .unset;
    var proceed: std.Io.Event = .unset;
    const described = T.native(Conflict.Capture{ .cell = &cell, .counts = &counts, .ready = &ready, .proceed = &proceed, .io = env.io }, Conflict.evaluate);
    const writer = try std.Thread.spawn(.{}, Conflict.writer, .{ &env, &cell, &ready, &proceed });
    var writer_joined = false;
    defer if (!writer_joined) {
        ready.set(env.io);
        writer.join();
    };
    var result = fx.run(&env, T.atomic(&described, select));
    defer result.deinit();
    ready.set(env.io);
    writer.join();
    writer_joined = true;
    try std.testing.expectEqual(@as(u32, 42), result.success);
    try std.testing.expectEqual(@as(u32, 42), cell.value);
    try std.testing.expectEqual(@as(usize, 2), counts.attempts);
    try std.testing.expectEqual(counts.attempts, counts.starts);
    try std.testing.expectEqual(counts.starts, counts.finishes);
}

const Waiting = struct {
    const Env = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, domain: *fx.TransactionDomain, cancellation: *fx.Cancellation };
    const Capture = struct { cell: *fx.TransactionRef(u32), ready: *std.Io.Event, io: std.Io, attempts: *usize };
    fn selectDomain(env: *const Env) *fx.TransactionDomain {
        return env.domain;
    }
    fn evaluate(capture: *const Capture, journal: *fx.TransactionDomain.Journal) fx.TransactionAtomicDecision(u32, E) {
        capture.attempts.* += 1;
        const read = journal.read(u32, capture.cell);
        if (read != .ok) {
            capture.ready.set(capture.io);
            return .retry;
        }
        if (read.ok == 0) {
            capture.ready.set(capture.io);
            return .retry;
        }
        return .{ .commit = read.ok };
    }
    fn change(env: *const Env, cell: *fx.TransactionRef(u32), ready: *std.Io.Event, cancel: bool) void {
        ready.waitUncancelable(env.io);
        if (cancel) {
            env.cancellation.request();
            return;
        }
        const plain: R = .{ .allocator = env.allocator, .domain = env.domain };
        const update = T.write(u32, E, cell, 23);
        var result = fx.run(&plain, T.atomic(&update, select));
        defer result.deinit();
        std.debug.assert(result == .success);
    }
};
test "TX-02 TX-05 LIFE-01 retry observes a committed read-set change and cancellation unregisters its wait without publishing a value" {
    inline for (.{ false, true }) |cancel| {
        const runtime = .{ .allocator = std.testing.allocator, .io = std.testing.io };
        var domain = fx.TransactionDomain.init(&runtime);
        defer domain.deinit();
        var cell = fx.TransactionRef(u32).init(&domain, 0);
        var cancellation: fx.Cancellation = .{};
        const env: Waiting.Env = .{ .allocator = std.testing.allocator, .domain = &domain, .cancellation = &cancellation };
        var ready: std.Io.Event = .unset;
        var attempts: usize = 0;
        const described = T.native(Waiting.Capture{ .cell = &cell, .ready = &ready, .io = env.io, .attempts = &attempts }, Waiting.evaluate);
        const changer = try std.Thread.spawn(.{}, Waiting.change, .{ &env, &cell, &ready, cancel });
        var changer_joined = false;
        defer if (!changer_joined) {
            ready.set(env.io);
            changer.join();
        };
        var result = fx.run(&env, T.atomic(&described, Waiting.selectDomain));
        defer result.deinit();
        ready.set(env.io);
        changer.join();
        changer_joined = true;
        if (cancel) {
            try std.testing.expect(result == .interrupted);
            try std.testing.expectEqual(@as(u32, 0), cell.value);
            try std.testing.expectEqual(@as(usize, 1), attempts);
        } else {
            try std.testing.expectEqual(@as(u32, 23), result.success);
            try std.testing.expectEqual(@as(usize, 2), attempts);
        }
        try std.testing.expectEqual(@as(usize, 0), domain.active_waiters);
    }
}

test "INF-06 INF-13 TX-01 a single transaction error anchor infers canonical cell values reads writes and pure success types" {
    const runtime = .{ .allocator = std.testing.allocator, .io = std.testing.io };
    var domain = fx.TransactionDomain.init(&runtime);
    defer domain.deinit();
    var cell = fx.TransactionRef(u32).init(&domain, 7);
    const C = T.context(E);
    const written = C.write(&cell, 19);
    const combined = T.zip(written, C.read(&cell));
    comptime {
        if (@TypeOf(C.read(&cell)).Success != u32) @compileError("cell value type must be inferred");
        if (@TypeOf(C.succeed(@as(u64, 11))).Success != u64) @compileError("native value type must be inferred");
    }
    const env: R = .{ .allocator = std.testing.allocator, .domain = &domain };
    var result = fx.run(&env, T.atomic(&combined, select));
    defer result.deinit();
    try std.testing.expectEqual(@as(u32, 19), result.success.right);
    try std.testing.expectEqual(@as(u32, 19), cell.value);
}

test "TX-03 TX-05 cross domain cells exhausted versions and empty retry preserve the native atomic boundary without tentative publication" {
    const runtime = .{ .allocator = std.testing.allocator, .io = std.testing.io };
    var first = fx.TransactionDomain.init(&runtime);
    defer first.deinit();
    var second = fx.TransactionDomain.init(&runtime);
    defer second.deinit();
    var owned = fx.TransactionRef(u32).init(&first, 7);
    var foreign = fx.TransactionRef(u32).init(&second, 13);
    const C = T.context(E);
    const env: R = .{ .allocator = std.testing.allocator, .domain = &first };
    const mixed = T.zip(C.write(&owned, 99), C.read(&foreign));
    var crossing = fx.run(&env, T.atomic(&mixed, select));
    defer crossing.deinit();
    try std.testing.expect(crossing == .fail and crossing.fail == .cross_domain);
    try std.testing.expectEqual(@as(u32, 7), owned.value);
    try std.testing.expectEqual(@as(u32, 13), foreign.value);
    owned.version = std.math.maxInt(u64);
    const write = C.write(&owned, 99);
    var exhausted = fx.run(&env, T.atomic(&write, select));
    defer exhausted.deinit();
    try std.testing.expect(exhausted == .fail and exhausted.fail == .version_exhausted);
    try std.testing.expectEqual(@as(u32, 7), owned.value);
    const retry = C.retry(u32);
    var empty = fx.run(&env, T.atomic(&retry, select));
    defer empty.deinit();
    try std.testing.expect(empty == .fail and empty.fail == .empty_retry_read_set);
}
