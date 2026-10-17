//! EX-06: try the primary balance, then retry against reserve funds in the
//! same journal. A tentative audit write from a retrying branch rolls back.
const std = @import("std");
const fx = @import("effect");
const T = fx.TransactionBuilder;
const Error = enum { rejected };
const Tx = T.context(Error);
pub const Environment = struct { allocator: std.mem.Allocator, io: std.Io, domain: *fx.TransactionDomain };
const Out = fx.Effect(u32, fx.TransactionError(Error), Environment);
const Roles = struct {
    fn decide(input: anytype, journal: *fx.TransactionDomain.Journal) fx.TransactionAtomicDecision(@TypeOf(Tx.read(input.account)).Success, Error) {
        const balance = Tx.read(input.account).evaluate(journal);
        if (balance != .commit) return .retry;
        const audit = Tx.read(input.audit).evaluate(journal);
        if (audit != .commit) return .retry;
        _ = Tx.write(input.audit, audit.commit + 1).evaluate(journal);
        if (balance.commit < input.amount) return .retry;
        const remaining = balance.commit - input.amount;
        _ = Tx.write(input.account, remaining).evaluate(journal);
        return .{ .commit = remaining };
    }
    fn domain(env: *const Environment) *fx.TransactionDomain {
        return env.domain;
    }
};
fn fromAccount(input: anytype) @TypeOf(T.native(input, Roles.decide)) {
    return T.native(input, Roles.decide);
}
/// Captured cells remain caller-owned domain handles. The generated capture
/// keeps the complete description stable through atomic retries and join.
pub fn withdrawal(env: *const Environment, input: anytype) Out {
    const transaction = T.orElse(fromAccount(.{ .account = input.primary, .audit = input.audit, .amount = input.amount }), fromAccount(.{ .account = input.reserve, .audit = input.audit, .amount = input.amount }));
    return fx.dsl.owned(env, .{ .transaction = transaction }, struct {
        fn atomic(stored: anytype, _: *const Environment) Out {
            return T.atomic(&stored.transaction, Roles.domain);
        }
    }.atomic);
}
fn check(allocator: std.mem.Allocator, io: std.Io) !void {
    const runtime = .{ .allocator = allocator, .io = io };
    var domain = fx.TransactionDomain.init(&runtime);
    defer domain.deinit();
    var primary = fx.TransactionRef(u32).init(&domain, 7);
    var reserve = fx.TransactionRef(u32).init(&domain, 13);
    var audit = fx.TransactionRef(u32).init(&domain, 0);
    const env: Environment = .{ .allocator = allocator, .io = io, .domain = &domain };
    var description = withdrawal(&env, .{ .primary = &primary, .reserve = &reserve, .audit = &audit, .amount = @as(u32, 9) });
    var result = fx.run(&env, description);
    description.deinit();
    defer result.deinit();
    if (result == .die and result.die.kind == .out_of_memory) {
        try std.testing.expectEqual(@as(u32, 7), primary.value);
        try std.testing.expectEqual(@as(u32, 13), reserve.value);
        try std.testing.expectEqual(@as(u32, 0), audit.value);
        return error.OutOfMemory;
    }
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(u32, 4), result.success);
    try std.testing.expectEqual(@as(u32, 7), primary.value);
    try std.testing.expectEqual(@as(u32, 4), reserve.value);
    try std.testing.expectEqual(@as(u32, 1), audit.value);
}
pub fn main(init: std.process.Init) !void {
    try check(init.gpa, init.io);
}
test "EX-06 captured account handles retry with fresh branch locals one journal and rollback including every OOM" {
    try check(std.testing.allocator, std.testing.io);
    try std.testing.checkAllAllocationFailures(std.testing.allocator, check, .{std.testing.io});
}
