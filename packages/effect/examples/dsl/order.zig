//! EX-02: reserve inventory, retain earlier totals, choose shipping, and apply
//! a bounded number of loyalty credits before returning an order total.
const std = @import("std");
const fx = @import("effect");
const p = fx.dsl.program;
const Error = enum { unavailable };
const Reservation = struct { allocator: std.mem.Allocator, order: u32, amount: u32 };
pub const Environment = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    acquired: *usize,
    released: *usize,
};
const Number = fx.Effect(u32, Error, Environment);
const Unit = fx.Effect(void, Error, Environment);
const Steps = struct {
    fn subtotal(input: anytype, _: *const Environment) u32 {
        var total: u32 = 0;
        for (input.lines.items) |price| total += price;
        return total;
    }
    fn reserve(input: anytype, amount: *const u32, env: *const Environment) fx.Effect(*Reservation, Error, Environment) {
        return fx.dsl.owned(env, .{ .order = input.order, .amount = amount.* }, struct {
            fn create(values: anytype, _: *const Environment) fx.Effect(*Reservation, Error, Environment) {
                return fx.Dsl(Error, Environment).outcomeBorrowed(values, struct {
                    fn allocate(data: @TypeOf(values), current: *const Environment) fx.Exit(*Reservation, Error) {
                        const reservation = current.allocator.create(Reservation) catch return .{ .die = fx.Defect.out_of_memory };
                        reservation.* = .{ .allocator = current.allocator, .order = data.order, .amount = data.amount };
                        current.acquired.* += 1;
                        return .{ .success = reservation };
                    }
                }.allocate);
            }
        }.create);
    }
    fn release(reservation: *Reservation) Unit {
        return Unit.fromBorrowed(Reservation, reservation, struct {
            fn dispose(value: *const Reservation, env: *const Environment) fx.Exit(void, Error) {
                env.released.* += 1;
                value.allocator.destroy(@constCast(value));
                return .{ .success = {} };
            }
        }.dispose);
    }
    fn expedited(input: anytype, _: *const u32, _: *const Environment) bool {
        return input.expedited;
    }
    fn express(_: anytype, subtotal_value: *const u32, _: *const Environment) Number {
        return Number.succeed(if (subtotal_value.* == 0) 0 else 8);
    }
    fn standard(_: anytype, subtotal_value: *const u32, _: *const Environment) Number {
        return Number.succeed(if (subtotal_value.* == 0) 0 else 3);
    }
    fn free(subtotal_value: *const u32, _: *const Environment) bool {
        return subtotal_value.* == 0;
    }
    const Credits = struct { remaining: u32, total: u32, reservation: *Reservation };
    fn initialize(input: anytype, subtotal_value: *const u32, shipping_value: *const u32, reservation: *const *Reservation, _: *const Environment) Credits {
        std.debug.assert(reservation.*.order == input.order and reservation.*.amount == subtotal_value.*);
        return .{ .remaining = input.credits, .total = subtotal_value.* + shipping_value.*, .reservation = reservation.* };
    }
    fn condition(state: *const Credits, _: *const Environment) bool {
        return state.remaining > 0 and state.total > 0;
    }
    fn credit(view: anytype, _: *const Environment) fx.Effect(p.LoopControl(u32), Error, Environment) {
        const state = view.state();
        std.debug.assert(state.reservation.amount <= state.total + state.remaining + 8);
        state.total -= 1;
        state.remaining -= 1;
        return fx.Effect(p.LoopControl(u32), Error, Environment).succeed(.again);
    }
    fn finish(state: anytype, _: *const Environment) u32 {
        return state.state().total;
    }
};
/// Input fields are inferred from this one runtime record. Lines are moved into
/// generated ownership; the reservation is a lexical resource, not a result.
pub fn order(env: *const Environment, input: anytype) Number {
    return p.captured(env, input, .{
        p.native("subtotal", .{"inputs"}, Steps.subtotal),
        p.resource("reservation", .{ "inputs", "subtotal" }, Steps.reserve, Steps.release),
        p.branch("shipping", .{ "inputs", "subtotal" }, Steps.expedited, Steps.express, Steps.standard),
        p.returnWhen("free", .{"subtotal"}, Steps.free, "shipping"),
        p.loop("total", .{ "inputs", "subtotal", "shipping", "reservation" }, Steps.initialize, Steps.condition, Steps.credit, Steps.finish),
    });
}
fn check(allocator: std.mem.Allocator, io: std.Io) !void {
    for ([_]bool{ false, true }) |empty| {
        for ([_]bool{ false, true }) |expedited| {
            var acquired: usize = 0;
            var released: usize = 0;
            const env: Environment = .{ .allocator = allocator, .io = io, .acquired = &acquired, .released = &released };
            const lines = try allocator.dupe(u32, if (empty) &.{} else &.{ 10, 20, 7 });
            var description = order(&env, .{ .order = @as(u32, 42), .lines = fx.OwnedSlice(u32){ .items = lines, .allocator = allocator }, .expedited = expedited, .credits = @as(u32, 4) });
            var result = fx.run(&env, description);
            description.deinit();
            defer result.deinit();
            try std.testing.expectEqual(acquired, released);
            if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expect(result == .success);
            try std.testing.expectEqual(@as(u32, if (empty) 0 else if (expedited) 41 else 36), result.success);
            try std.testing.expectEqual(@as(usize, 1), acquired);
        }
    }
}
pub fn main(init: std.process.Init) !void {
    try check(init.gpa, init.io);
}
test "EX-02 structured order retains earlier values and reservation across a loop and early return including OOM" {
    try check(std.testing.allocator, std.testing.io);
    try std.testing.checkAllAllocationFailures(std.testing.allocator, check, .{std.testing.io});
}
