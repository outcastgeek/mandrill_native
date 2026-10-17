//! EX-07: a price total tracks only the selected price source. Nested helpers
//! contribute reads to one evaluation and a failed refresh preserves its graph.
const std = @import("std");
const fx = @import("effect");
const Error = enum { unavailable, registry };
pub const R = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    registry: *fx.AtomRegistry,
    owner: []const u8,
    selection: []const u8,
    retail: []const u8,
    wholesale: []const u8,
    bulk: bool,
    fail: bool = false,
};
const A = fx.AtomProgram;
const Roles = struct {
    fn owner(env: *const R) []const u8 {
        return env.owner;
    }
    fn selection(env: *const R) []const u8 {
        return env.selection;
    }
    fn retail(env: *const R) []const u8 {
        return env.retail;
    }
    fn wholesale(env: *const R) []const u8 {
        return env.wholesale;
    }
    fn choose(env: *const R) fx.Effect(bool, Error, R) {
        return fx.Effect(bool, Error, R).succeed(env.bulk);
    }
    fn regular(_: *const R) fx.Effect(u32, Error, R) {
        return fx.Effect(u32, Error, R).succeed(12);
    }
    fn discounted(env: *const R) fx.Effect(u32, Error, R) {
        return if (env.fail) fx.Effect(u32, Error, R).fail(.unavailable) else fx.Effect(u32, Error, R).succeed(9);
    }
    fn registry(env: *const R) *fx.AtomRegistry {
        return env.registry;
    }
    fn errorMap(_: fx.AtomRegistryError) Error {
        return .registry;
    }
    fn tax(value: u32) u32 {
        return value + 2;
    }
    const selector = fx.Atom(bool, Error, R).readOnly("bulk", 1, selection, &.{}, choose);
    const regular_price = fx.Atom(u32, Error, R).readOnly("retail", 1, retail, &.{}, regular);
    const bulk_price = fx.Atom(u32, Error, R).readOnly("wholesale", 1, wholesale, &.{}, discounted);
    const total = fx.Atom(u32, Error, R).readOnly("total", 1, owner, &.{}, regular);
    fn retailTotal() @TypeOf(A.map(A.read(regular_price), tax)) {
        return A.map(A.read(regular_price), tax);
    }
    fn bulkTotal() @TypeOf(A.map(A.read(bulk_price), tax)) {
        return A.map(A.read(bulk_price), tax);
    }
    const body = A.branch(A.read(selector), bulkTotal(), retailTotal());
};
pub fn price() fx.Effect(A.Published(u32), Error, R) {
    return A.evaluate(Roles.total, Roles.body, Roles.registry, Roles.errorMap);
}
fn key(env: anytype, name: []const u8) !fx.OwnedSlice(u8) {
    const result = fx.PrimaryKey.encodeDirect(env, "price", &.{.{ .text = name }});
    if (result != .success) return error.OutOfMemory;
    return result.success;
}
fn check(allocator: std.mem.Allocator, io: std.Io) !void {
    const runtime = .{ .allocator = allocator, .io = io };
    var owner = try key(&runtime, "total");
    defer owner.deinit();
    var selector = try key(&runtime, "bulk");
    defer selector.deinit();
    var retail = try key(&runtime, "retail");
    defer retail.deinit();
    var wholesale = try key(&runtime, "wholesale");
    defer wholesale.deinit();
    var made = fx.AtomRegistry.create(&runtime, .{ .max_nodes = 8, .max_dependencies_per_node = 4 }, 8);
    if (made != .success) {
        made.deinit();
        return error.OutOfMemory;
    }
    const registry = made.success;
    defer registry.deinit();
    var env: R = .{ .allocator = allocator, .io = io, .registry = registry, .owner = owner.items, .selection = selector.items, .retail = retail.items, .wholesale = wholesale.items, .bulk = false };
    const description = comptime price();
    var first = fx.run(&env, description);
    defer first.deinit();
    try std.testing.expect(first == .success and first.success.value.current == 14);
    env.bulk = true;
    var second = fx.run(&env, description);
    defer second.deinit();
    try std.testing.expect(second == .success and second.success.value.current == 11);
    var old = registry.graph.dependents(retail.items);
    defer old.deinit();
    var current = registry.graph.dependents(wholesale.items);
    defer current.deinit();
    try std.testing.expectEqual(@as(usize, 0), old.success.items.len);
    try std.testing.expectEqual(@as(usize, 1), current.success.items.len);
    const generation = registry.graph.generation;
    env.fail = true;
    var failed = fx.run(&env, description);
    defer failed.deinit();
    try std.testing.expect(failed == .fail and failed.fail == .unavailable);
    try std.testing.expectEqual(generation, registry.graph.generation);
}
pub fn main(init: std.process.Init) !void {
    try check(init.gpa, init.io);
}
test "EX-07 nested branch-sensitive tracked prices update dependencies and preserve committed edges after failure" {
    try check(std.testing.allocator, std.testing.io);
}
