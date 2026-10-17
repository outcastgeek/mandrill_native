const std = @import("std");
const fx = @import("effect");
const Gates = @import("support/dsl_gate.zig");
const p = fx.dsl.program;
const E = enum { expected };
const State = struct { alive: std.atomic.Value(bool) = .init(false), invalid: std.atomic.Value(bool) = .init(false), releases: std.atomic.Value(usize) = .init(0), sequence: [2]u8 = .{ 0, 0 }, count: usize = 0 };
const Env = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, async_engine: fx.Executor, fail_body: bool = false, state: *State, gate: *Gates.Gate(void, E) };
const Earlier = struct {
    pub const effect_owned_payload = true;
    allocator: std.mem.Allocator,
    bytes: []u8,
    state: *State,
    pub fn deinit(self: *@This()) void {
        self.state.sequence[self.state.count] = 2;
        self.state.count += 1;
        self.state.alive.store(false, .release);
        self.allocator.free(self.bytes);
    }
};
const Resource = struct { allocator: std.mem.Allocator, earlier: *const Earlier, state: *State };
const Unit = fx.Effect(void, E, Env);
const Steps = struct {
    fn earlier(_: *const Env) fx.Effect(Earlier, E, Env) {
        return fx.Effect(Earlier, E, Env).fromOutcome(struct {
            fn run(env: *const Env) fx.Exit(Earlier, E) {
                const bytes = env.allocator.dupe(u8, "alive") catch return .{ .die = fx.Defect.out_of_memory };
                env.state.alive.store(true, .release);
                return .{ .success = .{ .allocator = env.allocator, .bytes = bytes, .state = env.state } };
            }
        }.run);
    }
    fn acquire(earlier_value: *const Earlier, _: *const Env) fx.Effect(*Resource, E, Env) {
        return fx.Dsl(E, Env).outcomeBorrowed(earlier_value, struct {
            fn run(value: *const Earlier, env: *const Env) fx.Exit(*Resource, E) {
                const resource = env.allocator.create(Resource) catch return .{ .die = fx.Defect.out_of_memory };
                resource.* = .{ .allocator = env.allocator, .earlier = value, .state = env.state };
                return .{ .success = resource };
            }
        }.run);
    }
    fn release(resource: *Resource) Unit {
        return Unit.deferBorrowed(Resource, resource, struct {
            fn build(value: *const Resource, env: *const Env) Unit {
                const Owner = struct {
                    pub const effect_owned_payload = true;
                    resource: *Resource,
                    state: *State,
                    pub fn deinit(self: *@This()) void {
                        // The independent state guard precedes access to the borrowed
                        // earlier slot, including when testing a lifetime regression.
                        if (!self.state.alive.load(.acquire)) {
                            self.state.invalid.store(true, .release);
                        } else if (!std.mem.eql(u8, self.resource.earlier.bytes, "alive")) self.state.invalid.store(true, .release);
                        self.state.sequence[self.state.count] = 1;
                        self.state.count += 1;
                        _ = self.state.releases.fetchAdd(1, .acq_rel);
                        self.resource.allocator.destroy(self.resource);
                    }
                    fn pending(_: *const @This(), _: *const Env) Unit {
                        return Unit.fromAsync(struct {
                            fn start(current: *const Env) fx.Async(void, E).Start {
                                return current.gate.start();
                            }
                        }.start);
                    }
                };
                return fx.Dsl(E, Env).owned(env, Owner{ .resource = @constCast(value), .state = env.state }, Owner.pending);
            }
        }.build);
    }
    fn result(_: *const Earlier, _: *const *Resource, env: *const Env) fx.Effect(u32, E, Env) {
        return if (env.fail_body) fx.Effect(u32, E, Env).fail(.expected) else fx.Effect(u32, E, Env).succeed(7);
    }
};
const nodes = .{ p.binding("earlier", .{}, Steps.earlier), p.resource("resource", .{"earlier"}, Steps.acquire, Steps.release), p.binding("result", .{ "earlier", "resource" }, Steps.result) };

const Mode = enum { success, body_failure, cancellation, release_failure };
fn exercise(mode: Mode) !void {
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = true }, .maximum_owned = 4 };
    defer engine.deinit();
    var state: State = .{};
    var gate: Gates.Gate(void, E) = .{ .result = if (mode == .release_failure) .{ .fail = .expected } else .{ .success = {} } };
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor(), .state = &state, .gate = &gate, .fail_body = mode == .body_failure };
    var pending = fx.runAsync(&env, comptime p.build(E, Env, nodes));
    defer pending.deinit();
    gate.started.waitUncancelable(env.io);
    try std.testing.expect(state.alive.load(.acquire));
    try std.testing.expectEqual(@as(usize, 0), state.releases.load(.acquire));
    if (mode == .cancellation) pending.request();
    gate.proceed.set(env.io);
    var result = pending.join();
    defer result.deinit();
    switch (mode) {
        .success => try std.testing.expectEqual(@as(u32, 7), result.success),
        .body_failure => try std.testing.expect(result == .fail and result.fail == .expected),
        .cancellation => try std.testing.expect(result == .interrupted),
        .release_failure => try std.testing.expect(result == .finalization_failed),
    }
    try std.testing.expect(!state.invalid.load(.acquire));
    try std.testing.expect(!state.alive.load(.acquire));
    try std.testing.expectEqualSlices(u8, &.{ 1, 2 }, &state.sequence);
    try std.testing.expectEqual(@as(usize, 1), state.releases.load(.acquire));
}

test "PROG-01 PROG-04 PROG-05 RES-01 LIFE-01 suspended release retains earlier slots on success failure cancellation and cleanup failure" {
    inline for (std.meta.tags(Mode)) |mode| try exercise(mode);
}
