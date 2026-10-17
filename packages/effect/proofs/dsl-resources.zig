const std = @import("std");
const fx = @import("effect");
const gates = @import("support/dsl_gate.zig");
const E = enum { body, release };
const OutcomeMode = enum { success, fail, die, interrupted };
const State = struct {
    acquired: usize = 0,
    released: usize = 0,
    owners: std.atomic.Value(usize) = .init(0),
    invalid: bool = false,
    order: [4]usize = @splat(0),
    pending_gate: ?*gates.Gate(void, E) = null,
    primary: ?fx.PrimaryOutcome(E) = null,
};
const Env = struct {
    allocator: std.mem.Allocator,
    io: std.Io = std.testing.io,
    async_engine: fx.Executor = undefined,
    state: *State,
    scope: *fx.Scope(E),
    gate: ?*gates.Gate(void, E) = null,
    fail_acquire: bool = false,
    fail_release: bool = false,
    body_outcome: OutcomeMode = .success,
};
const Unit = fx.Effect(void, E, Env);
const Resource = struct { allocator: std.mem.Allocator, inputs: *const Inputs, number: usize };
const Acquire = fx.Effect(*Resource, E, Env);
const Owner = struct {
    pub const effect_owned_payload = true;
    state: *State,
    pub fn deinit(self: *@This()) void {
        if (self.state.pending_gate) |gate| if (!gate.destroyed.load(.acquire)) {
            self.state.invalid = true;
        };
        _ = self.state.owners.fetchSub(1, .acq_rel);
    }
};
const Inputs = struct { owner: Owner, marker: u32 };
fn inputs(state: *State) Inputs {
    _ = state.owners.fetchAdd(1, .acq_rel);
    return .{ .owner = .{ .state = state }, .marker = 31 };
}
const Scoped = struct {
    fn acquire(stored: *const Inputs, _: *const Env) Acquire {
        return Acquire.fromBorrowed(Inputs, stored, struct {
            fn run(value: *const Inputs, env: *const Env) fx.Exit(*Resource, E) {
                if (env.fail_acquire) return .{ .fail = .body };
                const resource = env.allocator.create(Resource) catch return .{ .die = fx.Defect.out_of_memory };
                env.state.acquired += 1;
                resource.* = .{ .allocator = env.allocator, .inputs = value, .number = env.state.acquired };
                return .{ .success = resource };
            }
        }.run);
    }
    fn release(_: *const Inputs, resource: *Resource, _: *const Env) Unit {
        return Unit.deferBorrowed(Resource, resource, struct {
            fn build(value: *const Resource, _: *const Env) Unit {
                const cleanup = struct {
                    fn run(item: *const Resource, current: *const Env) fx.Exit(void, E) {
                        if (current.state.owners.load(.acquire) == 0) {
                            current.state.invalid = true;
                        } else if (item.inputs.marker != 31) current.state.invalid = true;
                        current.state.order[current.state.released] = item.number;
                        current.state.released += 1;
                        item.allocator.destroy(@constCast(item));
                        return if (current.fail_release) .{ .fail = .release } else .{ .success = {} };
                    }
                }.run;
                return Unit.fromBorrowed(Resource, value, cleanup);
            }
        }.build);
    }
};
const scope_roles = .{ .acquire = Scoped.acquire, .release = Scoped.release };

test "RES-02 COMP-03 nested using of ResourceValue preserves the single bracket owner" {
    const Binding = struct {
        fn acquire(env: *const Env) fx.Exit(u32, E) {
            env.state.acquired += 1;
            return .{ .success = 19 };
        }
        fn release(_: u32, _: fx.PrimaryOutcome(E)) Unit {
            return Unit.sync(struct {
                fn run(env: *const Env) void {
                    env.state.released += 1;
                }
            }.run);
        }
        fn outer(resource: fx.ResourceValue(u32)) fx.Effect(u32, E, Env) {
            return fx.dsl.using(resource, .{ .body = inner });
        }
        fn inner(resource: fx.ResourceValue(u32)) fx.Effect(u32, E, Env) {
            return fx.Effect(u32, E, Env).succeed(resource.value + 1);
        }
    };
    const binding = comptime fx.dsl.resourceBinding(.{ .acquire = fx.Effect(u32, E, Env).fromOutcome(Binding.acquire), .release = Binding.release });
    const described = comptime fx.dsl.using(binding, .{ .body = Binding.outer });
    comptime if (@TypeOf(described) != fx.Effect(u32, E, Env)) @compileError("using must return exact Effect");
    const composed = comptime described.map(struct {
        fn add(value: u32) u64 {
            return value + 2;
        }
    }.add);
    var state: State = .{};
    var scope = fx.Scope(E).init(.{ .allocator = std.testing.allocator, .io = std.testing.io });
    const env: Env = .{ .allocator = std.testing.allocator, .state = &state, .scope = &scope };
    try std.testing.expectEqual(@as(usize, 0), state.acquired);
    try std.testing.expectEqual(@as(u64, 22), fx.run(&env, composed).success);
    try std.testing.expectEqual(@as(usize, 1), state.acquired);
    try std.testing.expectEqual(@as(usize, 1), state.released);
    var closed = fx.run(&env, fx.Dsl(E, Env).closeScope(&scope));
    closed.deinit();
    scope.deinit();
}

test "RES-03 CAP-04 existing scope retains runtime capture after acquisition description deinit" {
    var state: State = .{};
    var scope = fx.Scope(E).init(.{ .allocator = std.testing.allocator, .io = std.testing.io });
    const env: Env = .{ .allocator = std.testing.allocator, .state = &state, .scope = &scope };
    var described = fx.dsl.scopeAcquire(&env, &scope, inputs(&state), scope_roles);
    comptime if (@TypeOf(described) != Acquire) @compileError("scopeAcquire must return exact Effect");
    try std.testing.expectEqual(@as(usize, 0), state.acquired);
    try std.testing.expect(fx.run(&env, described) == .success);
    try std.testing.expect(fx.run(&env, described) == .success);
    described.deinit();
    try std.testing.expectEqual(@as(usize, 1), state.owners.load(.acquire));
    try std.testing.expectEqual(@as(usize, 0), state.released);
    var closed = fx.run(&env, fx.Dsl(E, Env).closeScope(&scope));
    defer closed.deinit();
    try std.testing.expect(closed == .success);
    scope.deinit();
    try std.testing.expectEqualSlices(usize, &.{ 2, 1 }, state.order[0..2]);
    try std.testing.expectEqual(@as(usize, 0), state.owners.load(.acquire));
    try std.testing.expect(!state.invalid);
}

test "RES-03 LIFE-07 acquisition failure and closed registration release captures exactly once" {
    inline for (.{ true, false }) |fail_acquire| {
        var state: State = .{};
        var scope = fx.Scope(E).init(.{ .allocator = std.testing.allocator, .io = std.testing.io });
        var env: Env = .{ .allocator = std.testing.allocator, .state = &state, .scope = &scope, .fail_acquire = fail_acquire };
        if (!fail_acquire) {
            var earlier = fx.dsl.scopeAcquire(&env, &scope, inputs(&state), scope_roles);
            try std.testing.expect(fx.run(&env, earlier) == .success);
            earlier.deinit();
            var closed = fx.run(&env, fx.Dsl(E, Env).closeScope(&scope));
            closed.deinit();
        }
        var described = fx.dsl.scopeAcquire(&env, &scope, inputs(&state), scope_roles);
        var result = fx.run(&env, described);
        described.deinit();
        defer result.deinit();
        if (fail_acquire) try std.testing.expect(result == .fail and result.fail == .body) else try std.testing.expect(result == .die and result.die.kind == .invalid_configuration);
        var closed = fx.run(&env, fx.Dsl(E, Env).closeScope(&scope));
        closed.deinit();
        scope.deinit();
        try std.testing.expectEqual(state.acquired, state.released);
        try std.testing.expectEqual(@as(usize, 0), state.owners.load(.acquire));
        try std.testing.expect(!state.invalid);
    }
}

test "RES-03 LIFE-07 every construction and registration allocation failure preserves scope ownership" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            var state: State = .{};
            var scope = fx.Scope(E).init(.{ .allocator = allocator, .io = std.testing.io });
            const env: Env = .{ .allocator = allocator, .state = &state, .scope = &scope };
            var out_of_memory = false;
            for (0..2) |_| {
                var described = fx.dsl.scopeAcquire(&env, &scope, inputs(&state), scope_roles);
                var result = fx.run(&env, described);
                described.deinit();
                defer result.deinit();
                if (result == .die and result.die.kind == .out_of_memory) {
                    out_of_memory = true;
                    break;
                }
                try std.testing.expect(result == .success);
            }
            var closed = fx.run(&env, fx.Dsl(E, Env).closeScope(&scope));
            defer closed.deinit();
            if (closed != .success) return error.TestUnexpectedResult;
            scope.deinit();
            try std.testing.expectEqual(state.acquired, state.released);
            try std.testing.expectEqual(@as(usize, 0), state.owners.load(.acquire));
            try std.testing.expect(!state.invalid);
            if (out_of_memory) return error.OutOfMemory;
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

test "RES-03 registerFinalizer captures run in reverse order and remain scope-owned" {
    const Finalize = struct {
        fn run(stored: *const Inputs, _: *const Env) Unit {
            return Unit.fromBorrowed(Inputs, stored, struct {
                fn run(value: *const Inputs, env: *const Env) fx.Exit(void, E) {
                    if (env.state.owners.load(.acquire) == 0) env.state.invalid = true;
                    env.state.order[env.state.released] = value.marker;
                    env.state.released += 1;
                    return .{ .success = {} };
                }
            }.run);
        }
    };
    var state: State = .{};
    var scope = fx.Scope(E).init(.{ .allocator = std.testing.allocator, .io = std.testing.io });
    const env: Env = .{ .allocator = std.testing.allocator, .state = &state, .scope = &scope };
    inline for (.{ 1, 2 }) |marker| {
        var captured = inputs(&state);
        captured.marker = marker;
        var described = fx.dsl.registerFinalizer(&env, &scope, captured, .{ .run = Finalize.run });
        comptime if (@TypeOf(described) != Unit) @compileError("registerFinalizer must return exact Effect");
        try std.testing.expect(fx.run(&env, described) == .success);
        described.deinit();
    }
    try std.testing.expectEqual(@as(usize, 0), state.released);
    var closed = fx.run(&env, fx.Dsl(E, Env).closeScope(&scope));
    defer closed.deinit();
    try std.testing.expect(closed == .success);
    scope.deinit();
    try std.testing.expectEqualSlices(usize, &.{ 2, 1 }, state.order[0..2]);
    try std.testing.expectEqual(@as(usize, 0), state.owners.load(.acquire));
}

test "RES-03 LIFE-05 pending finalizer retains capture through cancellation join and teardown" {
    const Finalize = struct {
        fn run(stored: *const Inputs, _: *const Env) Unit {
            return Unit.fromAsyncBorrowed(Inputs, stored, struct {
                fn start(value: *const Inputs, current: *const Env) fx.Async(void, E).Start {
                    if (current.state.owners.load(.acquire) == 0) {
                        current.state.invalid = true;
                        return .{ .completed = .{ .die = fx.Defect.init("capture ended before release start") } };
                    }
                    if (value.marker != 31) current.state.invalid = true;
                    return current.gate.?.start();
                }
            }.start);
        }
    };
    inline for (.{ false, true }) |fail_release| {
        var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = true }, .maximum_owned = 4 };
        defer engine.deinit();
        var gate: gates.Gate(void, E) = .{ .result = if (fail_release) .{ .fail = .release } else .{ .success = {} } };
        var state: State = .{ .pending_gate = &gate };
        var scope = fx.Scope(E).init(.{ .allocator = std.testing.allocator, .io = std.testing.io });
        const env: Env = .{ .allocator = std.testing.allocator, .state = &state, .scope = &scope, .gate = &gate, .async_engine = engine.executor() };
        var described = fx.dsl.registerFinalizer(&env, &scope, inputs(&state), .{ .run = Finalize.run });
        try std.testing.expect(fx.run(&env, described) == .success);
        described.deinit();
        var pending = fx.runAsync(&env, fx.Dsl(E, Env).closeScope(&scope));
        defer pending.deinit();
        gate.started.waitUncancelable(env.io);
        try std.testing.expectEqual(@as(usize, 1), state.owners.load(.acquire));
        pending.request();
        gate.proceed.set(env.io);
        var closed = pending.join();
        try std.testing.expect(if (fail_release) closed == .finalization_failed else closed == .interrupted);
        closed.deinit();
        scope.deinit();
        try std.testing.expect(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
        try std.testing.expectEqual(@as(usize, 0), state.owners.load(.acquire));
        try std.testing.expect(!state.invalid);
    }
}

test "RES-02 CAP-04 captured using body retains inputs without taking a second resource owner" {
    const Nested = struct {
        fn body(resource: fx.ResourceValue(u32), env: *const Env) fx.Effect(u32, E, Env) {
            return fx.dsl.usingCaptured(env, resource, inputs(env.state), .{ .body = combine });
        }
        fn combine(stored: *const Inputs, resource: fx.ResourceValue(u32), env: *const Env) fx.Effect(u32, E, Env) {
            // The deferred body still uses its owning inputs after the factory.
            const P = fx.Effect(u32, E, Env);
            const Captured = struct { stored: *const Inputs, resource: u32 };
            const callback = struct {
                fn run(value: *const Captured, current: *const Env) fx.Exit(u32, E) {
                    if (current.state.owners.load(.acquire) == 0) return .{ .die = fx.Defect.init("using inputs ended before deferred body") };
                    return .{ .success = value.stored.marker + value.resource };
                }
            }.run;
            // Move this small borrowed view into the returned description.
            return P.fromOwned(env, Captured{ .stored = stored, .resource = resource.value }, callback);
        }
    };
    const AcquireValue = fx.Effect(u32, E, Env);
    const described = comptime AcquireValue.succeed(7).acquireRelease(struct {
        fn body(value: u32, env: *const Env) AcquireValue {
            return Nested.body(.{ .value = value }, env);
        }
    }.body, struct {
        fn release(_: u32, _: fx.PrimaryOutcome(E)) Unit {
            return Unit.sync(struct {
                fn run(env: *const Env) void {
                    env.state.released += 1;
                }
            }.run);
        }
    }.release);
    var state: State = .{};
    var scope = fx.Scope(E).init(.{ .allocator = std.testing.allocator, .io = std.testing.io });
    const env: Env = .{ .allocator = std.testing.allocator, .state = &state, .scope = &scope };
    try std.testing.expectEqual(@as(u32, 38), fx.run(&env, described).success);
    try std.testing.expectEqual(@as(usize, 1), state.released);
    try std.testing.expectEqual(@as(usize, 0), state.owners.load(.acquire));
    var closed = fx.run(&env, fx.Dsl(E, Env).closeScope(&scope));
    closed.deinit();
    scope.deinit();
}

test "RES-01 LIFE-01 captured resource roles preserve primary outcomes and exactly-once release" {
    const Named = struct {
        fn body(stored: *const Inputs, _: *Resource, _: *const Env) fx.Effect(u32, E, Env) {
            return fx.Effect(u32, E, Env).fromBorrowed(Inputs, stored, struct {
                fn run(value: *const Inputs, env: *const Env) fx.Exit(u32, E) {
                    if (env.state.owners.load(.acquire) == 0) return .{ .die = fx.Defect.init("resource capture ended during body") };
                    return switch (env.body_outcome) {
                        .success => .{ .success = value.marker },
                        .fail => .{ .fail = .body },
                        .die => .{ .die = fx.Defect.init("body defect") },
                        .interrupted => .interrupted,
                    };
                }
            }.run);
        }
        fn release(stored: *const Inputs, resource: *Resource, primary: fx.PrimaryOutcome(E), env: *const Env) Unit {
            env.state.primary = primary;
            return Scoped.release(stored, resource, env);
        }
    };
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            inline for (std.meta.tags(OutcomeMode)) |mode| {
                inline for (.{ false, true }) |cleanup_failure| {
                    var state: State = .{};
                    var scope = fx.Scope(E).init(.{ .allocator = allocator, .io = std.testing.io });
                    const env: Env = .{ .allocator = allocator, .state = &state, .scope = &scope, .body_outcome = mode, .fail_release = cleanup_failure };
                    var described = fx.dsl.resourceCaptured(&env, inputs(&state), .{ .acquire = Scoped.acquire, .body = Named.body, .release = Named.release });
                    comptime if (@TypeOf(described) != fx.Effect(u32, E, Env)) @compileError("resourceCaptured must return exact Effect");
                    var result = fx.run(&env, described);
                    described.deinit();
                    defer result.deinit();
                    var closed = fx.run(&env, fx.Dsl(E, Env).closeScope(&scope));
                    closed.deinit();
                    scope.deinit();
                    try std.testing.expectEqual(state.acquired, state.released);
                    try std.testing.expectEqual(@as(usize, 0), state.owners.load(.acquire));
                    try std.testing.expect(!state.invalid);
                    if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
                    if (cleanup_failure) {
                        try std.testing.expect(result == .finalization_failed);
                        try std.testing.expectEqual(E.release, result.finalization_failed.first.release_failure);
                    } else switch (mode) {
                        .success => try std.testing.expectEqual(@as(u32, 31), result.success),
                        .fail => try std.testing.expect(result == .fail and result.fail == .body),
                        .die => try std.testing.expect(result == .die),
                        .interrupted => try std.testing.expect(result == .interrupted),
                    }
                    try std.testing.expect(state.primary != null);
                    switch (mode) {
                        .success => try std.testing.expect(state.primary.? == .succeeded),
                        .fail => try std.testing.expect(state.primary.? == .failed),
                        .die => try std.testing.expect(state.primary.? == .defected),
                        .interrupted => try std.testing.expect(state.primary.? == .was_interrupted),
                    }
                }
            }
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

test "RES-03 CAP-12 named scoped body closes its registered resources with runtime inputs" {
    const Body = struct {
        fn run(stored: *const Inputs, scope: *fx.Scope(E), env: *const Env) fx.Effect(u32, E, Env) {
            // The nested capture only borrows the outer input. The lexical
            // scope drains before the outer owning capture is destroyed.
            var acquired = fx.dsl.scopeAcquire(env, scope, .{ .borrowed = stored }, .{
                .acquire = struct {
                    fn acquire(value: anytype, current: *const Env) Acquire {
                        return Scoped.acquire(value.borrowed, current);
                    }
                }.acquire,
                .release = struct {
                    fn release(value: anytype, resource: *Resource, current: *const Env) Unit {
                        return Scoped.release(value.borrowed, resource, current);
                    }
                }.release,
            });
            return acquired.mapOwned(env, struct {
                fn value(resource: *Resource) u32 {
                    return resource.inputs.marker;
                }
            }.value);
        }
    };
    var state: State = .{};
    var scope = fx.Scope(E).init(.{ .allocator = std.testing.allocator, .io = std.testing.io });
    const env: Env = .{ .allocator = std.testing.allocator, .state = &state, .scope = &scope };
    var described = fx.dsl.scopedCaptured(&env, inputs(&state), .{ .body = Body.run });
    var result = fx.run(&env, described);
    defer result.deinit();
    described.deinit();
    try std.testing.expectEqual(@as(u32, 31), result.success);
    try std.testing.expectEqual(@as(usize, 1), state.released);
    try std.testing.expectEqual(@as(usize, 0), state.owners.load(.acquire));
    try std.testing.expect(!state.invalid);
    var closed = fx.run(&env, fx.Dsl(E, Env).closeScope(&scope));
    closed.deinit();
    scope.deinit();
}

test "RES-05 explicit release type adapter generates the same resource binding" {
    const Adapter = struct {
        pub fn release(_: u32, _: fx.PrimaryOutcome(E)) Unit {
            return Unit.sync(struct {
                fn run(env: *const Env) void {
                    env.state.released += 1;
                }
            }.run);
        }
    };
    const binding = comptime fx.dsl.resourceBinding(.{ .acquire = fx.Effect(u32, E, Env).succeed(9), .adapter = Adapter });
    const described = comptime fx.dsl.using(binding, .{ .body = struct {
        fn run(value: fx.ResourceValue(u32)) fx.Effect(u32, E, Env) {
            return fx.Effect(u32, E, Env).succeed(value.value);
        }
    }.run });
    var state: State = .{};
    var scope = fx.Scope(E).init(.{ .allocator = std.testing.allocator, .io = std.testing.io });
    const env: Env = .{ .allocator = std.testing.allocator, .state = &state, .scope = &scope };
    try std.testing.expectEqual(@as(u32, 9), fx.run(&env, described).success);
    try std.testing.expectEqual(@as(usize, 1), state.released);
    var closed = fx.run(&env, fx.Dsl(E, Env).closeScope(&scope));
    closed.deinit();
    scope.deinit();
}

test "RES-01 named static resource roles match the direct bracket and expose primary outcome" {
    const Roles = struct {
        fn body(value: u32) fx.Effect(u32, E, Env) {
            return fx.Effect(u32, E, Env).succeed(value + 2);
        }
        fn release(_: u32, primary: fx.PrimaryOutcome(E), env: *const Env) Unit {
            env.state.primary = primary;
            return Unit.sync(struct {
                fn run(current: *const Env) void {
                    current.state.released += 1;
                }
            }.run);
        }
    };
    const acquire = comptime fx.Effect(u32, E, Env).succeed(7);
    const named = comptime fx.dsl.resource(.{ .acquire = acquire, .body = Roles.body, .release = Roles.release });
    const direct = comptime acquire.acquireRelease(Roles.body, Roles.release);
    var state: State = .{};
    var scope = fx.Scope(E).init(.{ .allocator = std.testing.allocator, .io = std.testing.io });
    const env: Env = .{ .allocator = std.testing.allocator, .state = &state, .scope = &scope };
    try std.testing.expectEqual(@as(usize, 0), state.released);
    try std.testing.expectEqual(fx.run(&env, direct).success, fx.run(&env, named).success);
    try std.testing.expectEqual(@as(usize, 2), state.released);
    try std.testing.expect(state.primary.? == .succeeded);
    var closed = fx.run(&env, fx.Dsl(E, Env).closeScope(&scope));
    closed.deinit();
    scope.deinit();
}

test "RES-01 LIFE-11 delayed release borrows stable primary storage while owning error remains live" {
    const Failure = struct {
        pub const effect_owned_payload = true;
        allocator: std.mem.Allocator,
        bytes: []u8,
        live: *std.atomic.Value(usize),
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            const copied = try self.allocator.dupe(u8, self.bytes);
            _ = self.live.fetchAdd(1, .acq_rel);
            return .{ .allocator = self.allocator, .bytes = copied, .live = self.live };
        }
        pub fn deinit(self: *@This()) void {
            self.allocator.free(self.bytes);
            _ = self.live.fetchSub(1, .acq_rel);
        }
    };
    const Primary = fx.PrimaryOutcome(Failure);
    const Runtime = struct {
        allocator: std.mem.Allocator,
        io: std.Io,
        async_engine: fx.Executor,
        live: *std.atomic.Value(usize),
        seen: *bool,
        gate: *gates.Gate(void, Failure),
    };
    const P = fx.Effect(void, Failure, Runtime);
    const Named = struct {
        fn acquire(_: *const u8, _: *const Runtime) P {
            return P.succeed({});
        }
        fn body(_: *const u8, _: void, _: *const Runtime) P {
            return P.fromOutcome(struct {
                fn run(env: *const Runtime) fx.Exit(void, Failure) {
                    const bytes = env.allocator.dupe(u8, "owning primary") catch return .{ .die = fx.Defect.out_of_memory };
                    _ = env.live.fetchAdd(1, .acq_rel);
                    return .{ .fail = .{ .allocator = env.allocator, .bytes = bytes, .live = env.live } };
                }
            }.run);
        }
        fn release(_: *const u8, _: void, primary: *const Primary, env: *const Runtime) P {
            var waiting = P.fromAsync(struct {
                fn start(current: *const Runtime) fx.Async(void, Failure).Start {
                    return current.gate.start();
                }
            }.start);
            return waiting.flatMapCapturedOwned(env, void, primary, struct {
                fn after(stored: *const *const Primary, _: void, _: *const Runtime) P {
                    return P.fromBorrowed(Primary, stored.*, struct {
                        fn inspect(value: *const Primary, current: *const Runtime) fx.Exit(void, Failure) {
                            if (current.live.load(.acquire) == 0) return .{ .die = fx.Defect.init("primary payload ended before delayed release") };
                            if (value.* != .failed or !std.mem.eql(u8, value.failed.bytes, "owning primary")) return .{ .die = fx.Defect.init("delayed release lost primary outcome") };
                            current.seen.* = true;
                            return .{ .success = {} };
                        }
                    }.inspect);
                }
            }.after);
        }
    };
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = true }, .maximum_owned = 4 };
    defer engine.deinit();
    var live: std.atomic.Value(usize) = .init(0);
    var seen = false;
    var gate: gates.Gate(void, Failure) = .{ .result = .{ .success = {} } };
    const env: Runtime = .{ .allocator = std.testing.allocator, .io = std.testing.io, .async_engine = engine.executor(), .live = &live, .seen = &seen, .gate = &gate };
    var described = fx.dsl.resourceCaptured(&env, @as(u8, 0), .{ .acquire = Named.acquire, .body = Named.body, .release = Named.release });
    var pending = fx.runAsync(&env, described);
    gate.started.waitUncancelable(env.io);
    try std.testing.expectEqual(@as(usize, 1), live.load(.acquire));
    gate.proceed.set(env.io);
    var result = pending.join();
    try std.testing.expect(result == .fail);
    try std.testing.expectEqualStrings("owning primary", result.fail.bytes);
    try std.testing.expect(seen);
    result.deinit();
    pending.deinit();
    described.deinit();
    try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
}

test "RES-02 CAP-08 canonical ResourceValue fields stay borrowed inside generated captures" {
    const Buffer = struct {
        pub const effect_owned_payload = true;
        allocator: std.mem.Allocator,
        bytes: []u8,
        state: *State,
        pub fn deinit(self: *@This()) void {
            // Record a duplicate drop without ever freeing the same bytes twice.
            if (self.state.owners.swap(0, .acq_rel) != 1) {
                self.state.invalid = true;
                return;
            }
            self.allocator.free(self.bytes);
            self.state.released += 1;
        }
    };
    const Roles = struct {
        fn acquire(env: *const Env) fx.Exit(Buffer, E) {
            const bytes = env.allocator.dupe(u8, "borrowed") catch return .{ .die = fx.Defect.out_of_memory };
            env.state.owners.store(1, .release);
            return .{ .success = .{ .allocator = env.allocator, .bytes = bytes, .state = env.state } };
        }
        fn body(value: Buffer, env: *const Env) fx.Effect(u32, E, Env) {
            return fx.dsl.owned(env, .{ .resource = fx.ResourceValue(Buffer){ .value = value } }, struct {
                fn run(captured: anytype, current: *const Env) fx.Effect(u32, E, Env) {
                    const P = fx.Effect(u32, E, Env);
                    if (current.state.owners.load(.acquire) != 1) return P.die(fx.Defect.init("resource owner ended before captured body"));
                    return P.succeed(@intCast(captured.resource.value.bytes.len));
                }
            }.run);
        }
        fn release(value: Buffer, _: fx.PrimaryOutcome(E), env: *const Env) Unit {
            if (env.state.owners.load(.acquire) != 1) env.state.invalid = true;
            return Unit.fromOwned(env, value, struct {
                fn run(_: *const Buffer, _: *const Env) fx.Exit(void, E) {
                    return .{ .success = {} };
                }
            }.run);
        }
    };
    const described = comptime fx.dsl.resource(.{ .acquire = fx.Effect(Buffer, E, Env).fromOutcome(Roles.acquire), .body = Roles.body, .release = Roles.release });
    var state: State = .{};
    var scope = fx.Scope(E).init(.{ .allocator = std.testing.allocator, .io = std.testing.io });
    const env: Env = .{ .allocator = std.testing.allocator, .state = &state, .scope = &scope };
    var result = fx.run(&env, described);
    defer result.deinit();
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(u32, 8), result.success);
    try std.testing.expect(!state.invalid);
    try std.testing.expectEqual(@as(usize, 1), state.released);
    var closed = fx.run(&env, fx.Dsl(E, Env).closeScope(&scope));
    closed.deinit();
    scope.deinit();
}
