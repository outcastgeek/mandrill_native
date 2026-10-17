const std = @import("std");
const fx = @import("effect");
const T = fx.TransactionBuilder;
const E = enum { expected };
const R = struct { allocator: std.mem.Allocator, io: std.Io, domain: *fx.TransactionDomain };
fn select(env: *const R) *fx.TransactionDomain {
    return env.domain;
}

test "TX-01 TX-03 INF-06 COMP-02 inferred transaction captures share one journal and compose as Effect" {
    const runtime = .{ .allocator = std.testing.allocator, .io = std.testing.io };
    var domain = fx.TransactionDomain.init(&runtime);
    defer domain.deinit();
    var cell = fx.TransactionRef(u32).init(&domain, 3);
    const Callback = struct {
        fn add(value: u32) u64 {
            return value + 1;
        }
        fn write(capture: anytype, value: u64) @TypeOf(T.write(u32, E, capture.cell, 0)) {
            return T.write(u32, E, capture.cell, @intCast(value));
        }
    };
    const read = T.read(u32, E, &cell);
    const mapped = T.map(read, Callback.add);
    const described = T.bindCaptured(mapped, .{ .cell = &cell }, Callback.write);
    const effect = T.atomic(&described, select);
    comptime {
        if (@TypeOf(effect) != fx.Effect(void, fx.TransactionError(E), R)) @compileError("atomic boundary must retain exact Effect");
    }
    const env: R = .{ .allocator = std.testing.allocator, .io = std.testing.io, .domain = &domain };
    try std.testing.expect(fx.run(&env, effect) == .success);
    try std.testing.expectEqual(@as(u32, 4), cell.value);
}

test "TX-01 INF-06 mapped transaction errors preserve the existing borrowed failure payload contract" {
    const Mapper = struct {
        const detail: u8 = 23;
        fn map(_: E) *const u8 {
            return &detail;
        }
    };
    const runtime = .{ .allocator = std.testing.allocator, .io = std.testing.io };
    var domain = fx.TransactionDomain.init(&runtime);
    defer domain.deinit();
    const source = T.fail(u32, E, E.expected);
    const mapped = T.mapError(source, Mapper.map);
    const described = T.atomic(&mapped, select);
    comptime {
        if (@TypeOf(described) != fx.Effect(u32, fx.TransactionError(*const u8), R)) @compileError("mapped error evidence must remain the exact ordinary Effect channel");
    }
    const env: R = .{ .allocator = std.testing.allocator, .io = std.testing.io, .domain = &domain };
    var result = fx.run(&env, described);
    defer result.deinit();
    try std.testing.expect(result == .fail and result.fail == .domain);
    try std.testing.expectEqual(&Mapper.detail, result.fail.domain);
}
