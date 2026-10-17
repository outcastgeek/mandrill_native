const std = @import("std");
const fx = @import("effect");
const gates = @import("support/dsl_gate.zig");
const R = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io };
fn key(env: *const R, name: []const u8) !fx.OwnedSlice(u8) {
    const encoded = fx.PrimaryKey.encodeDirect(env, "dsl-atoms", &.{.{ .text = name }});
    if (encoded != .success) return error.OutOfMemory;
    return encoded.success;
}

test "ATOM-05 stale revisions and copied evaluation generations cannot publish" {
    const env: R = .{ .allocator = std.testing.allocator };
    const registry = fx.AtomRegistry.create(&env, .{ .max_nodes = 16, .max_dependencies_per_node = 8 }, 8).success;
    defer registry.deinit();
    var owner = try key(&env, "owner");
    defer owner.deinit();
    var dependency = try key(&env, "dependency");
    defer dependency.deinit();
    const ticket = registry.beginEvaluation(owner.items).success;
    const revision = registry.observedRevision(dependency.items).success;
    var changed = registry.invalidate(dependency.items);
    changed.deinit();
    var stale = registry.publishEvaluation(owner.items, ticket, &.{.{ .key = dependency.items, .revision = revision }});
    defer stale.deinit();
    try std.testing.expect(stale == .success and stale.success == .stale);
    const newer = registry.beginEvaluation(owner.items).success;
    var published = registry.publishEvaluation(owner.items, newer, &.{});
    defer published.deinit();
    try std.testing.expect(published == .success and published.success == .committed);
    var duplicate = registry.publishEvaluation(owner.items, newer, &.{});
    defer duplicate.deinit();
    try std.testing.expect(duplicate == .success and duplicate.success == .stale);
}

const E = enum { expected, registry };
const never_cancel: fx.Cancellation = .{};
const TrackedEnv = struct {
    cancellation: *const fx.Cancellation = &never_cancel,
    released: ?*usize = null,
    allocator: std.mem.Allocator,
    io: std.Io = std.testing.io,
    registry: *fx.AtomRegistry,
    owner: []const u8,
    predicate: []const u8,
    b: []const u8,
    c: []const u8,
    choose_b: bool,
    fail_b: bool = false,
    change_b_during_read: bool = false,
    gate: ?*gates.Gate(u32, E) = null,
    async_engine: fx.Executor = undefined,
};
const TestAtoms = struct {
    fn predicateKey(env: *const TrackedEnv) []const u8 {
        return env.predicate;
    }
    fn bKey(env: *const TrackedEnv) []const u8 {
        return env.b;
    }
    fn cKey(env: *const TrackedEnv) []const u8 {
        return env.c;
    }
    fn ownerKey(env: *const TrackedEnv) []const u8 {
        return env.owner;
    }
    fn predicateRead(_: *const TrackedEnv) fx.Effect(bool, E, TrackedEnv) {
        return fx.Effect(bool, E, TrackedEnv).sync(struct {
            fn run(env: *const TrackedEnv) bool {
                return env.choose_b;
            }
        }.run);
    }
    fn bRead(current: *const TrackedEnv) fx.Effect(u32, E, TrackedEnv) {
        if (current.gate != null) return fx.Effect(u32, E, TrackedEnv).fromAsync(struct {
            fn start(env: *const TrackedEnv) fx.Async(u32, E).Start {
                return env.gate.?.start();
            }
        }.start);
        return fx.Effect(u32, E, TrackedEnv).fromOutcome(struct {
            fn run(env: *const TrackedEnv) fx.Exit(u32, E) {
                if (env.fail_b) return .{ .fail = .expected };
                if (env.change_b_during_read) {
                    var changed = env.registry.invalidate(env.b);
                    changed.deinit();
                }
                return .{ .success = 11 };
            }
        }.run);
    }
    fn cRead(_: *const TrackedEnv) fx.Effect(u32, E, TrackedEnv) {
        return fx.Effect(u32, E, TrackedEnv).succeed(23);
    }
    fn registry(env: *const TrackedEnv) *fx.AtomRegistry {
        return env.registry;
    }
    fn mapError(_: fx.AtomRegistryError) E {
        return .registry;
    }
    const predicate = fx.Atom(bool, E, TrackedEnv).readOnly("predicate", 1, predicateKey, &.{}, predicateRead);
    const b = fx.Atom(u32, E, TrackedEnv).readOnly("b", 1, bKey, &.{}, bRead);
    const c = fx.Atom(u32, E, TrackedEnv).readOnly("c", 1, cKey, &.{}, cRead);
    const owner = fx.Atom(u32, E, TrackedEnv).readOnly("owner", 1, ownerKey, &.{}, cRead);
    const program = fx.AtomProgram.branch(fx.AtomProgram.read(predicate), fx.AtomProgram.read(b), fx.AtomProgram.read(c));
};

test "ATOM-01 ATOM-02 actual reads replace branch dependencies without a manual list" {
    const runtime: R = .{ .allocator = std.testing.allocator };
    const registry = fx.AtomRegistry.create(&runtime, .{ .max_nodes = 16, .max_dependencies_per_node = 8 }, 8).success;
    defer registry.deinit();
    var owner = try key(&runtime, "owner");
    defer owner.deinit();
    var predicate = try key(&runtime, "predicate");
    defer predicate.deinit();
    var b = try key(&runtime, "b");
    defer b.deinit();
    var c = try key(&runtime, "c");
    defer c.deinit();
    var env: TrackedEnv = .{ .allocator = std.testing.allocator, .registry = registry, .owner = owner.items, .predicate = predicate.items, .b = b.items, .c = c.items, .choose_b = true };
    const described = comptime fx.AtomProgram.evaluate(TestAtoms.owner, TestAtoms.program, TestAtoms.registry, TestAtoms.mapError);
    comptime {
        if (@TypeOf(described) != fx.Effect(fx.AtomProgram.Published(u32), E, TrackedEnv)) @compileError("tracked evaluation must be exact Effect");
    }
    var first = fx.run(&env, described);
    defer first.deinit();
    try std.testing.expectEqual(@as(u32, 11), first.success.value.current);
    var dependents = registry.graph.dependents(b.items);
    defer dependents.deinit();
    try std.testing.expectEqual(@as(usize, 1), dependents.success.items.len);
    env.choose_b = false;
    var second = fx.run(&env, described);
    defer second.deinit();
    try std.testing.expectEqual(@as(u32, 23), second.success.value.current);
    var old = registry.graph.dependents(b.items);
    defer old.deinit();
    try std.testing.expectEqual(@as(usize, 0), old.success.items.len);
    var current = registry.graph.dependents(c.items);
    defer current.deinit();
    try std.testing.expectEqual(@as(usize, 1), current.success.items.len);
}

test "ATOM-03 ATOM-05 failed and revision-stale reads preserve committed dependencies" {
    const runtime: R = .{ .allocator = std.testing.allocator };
    const registry = fx.AtomRegistry.create(&runtime, .{ .max_nodes = 16, .max_dependencies_per_node = 8 }, 8).success;
    defer registry.deinit();
    var owner = try key(&runtime, "owner");
    defer owner.deinit();
    var predicate = try key(&runtime, "predicate");
    defer predicate.deinit();
    var b = try key(&runtime, "b");
    defer b.deinit();
    var c = try key(&runtime, "c");
    defer c.deinit();
    var env: TrackedEnv = .{ .allocator = std.testing.allocator, .registry = registry, .owner = owner.items, .predicate = predicate.items, .b = b.items, .c = c.items, .choose_b = false };
    const described = comptime fx.AtomProgram.evaluate(TestAtoms.owner, TestAtoms.program, TestAtoms.registry, TestAtoms.mapError);
    var first = fx.run(&env, described);
    first.deinit();
    const generation = registry.graph.generation;
    env.choose_b = true;
    env.fail_b = true;
    var failure = fx.run(&env, described);
    defer failure.deinit();
    try std.testing.expect(failure == .fail and failure.fail == .expected);
    try std.testing.expectEqual(generation, registry.graph.generation);
    env.fail_b = false;
    env.change_b_during_read = true;
    var stale = fx.run(&env, described);
    defer stale.deinit();
    try std.testing.expect(stale.success.value == .stale);
    try std.testing.expectEqual(generation, registry.graph.generation);
}

test "ATOM-09 committed graph and consumed ticket survive notification allocation failure" {
    const direct: R = .{ .allocator = std.testing.allocator };
    var owner = try key(&direct, "notification-owner");
    defer owner.deinit();
    var observed_failure = false;
    var position: usize = 0;
    while (position < 64) : (position += 1) {
        var failing = std.testing.FailingAllocator.init(std.testing.allocator, .{});
        const env: R = .{ .allocator = failing.allocator() };
        const registry = fx.AtomRegistry.create(&env, .{ .max_nodes = 8, .max_dependencies_per_node = 4 }, 4).success;
        defer registry.deinit();
        const ticket = registry.beginEvaluation(owner.items).success;
        failing.fail_index = failing.alloc_index + position;
        var published = registry.publishEvaluation(owner.items, ticket, &.{});
        defer published.deinit();
        if (published == .success and published.success == .committed and published.success.committed.invalidation != .success) {
            observed_failure = true;
            const generation = registry.graph.generation;
            var repeated = registry.publishEvaluation(owner.items, ticket, &.{});
            defer repeated.deinit();
            try std.testing.expect(repeated == .success and repeated.success == .stale);
            try std.testing.expectEqual(generation, registry.graph.generation);
        }
        if (!failing.has_induced_failure) break;
    }
    try std.testing.expect(observed_failure);
}

test "COMP-04 LIFE-01 atom commit in finalizer does not mask cancellation of outer body" {
    const runtime: R = .{ .allocator = std.testing.allocator };
    const registry = fx.AtomRegistry.create(&runtime, .{ .max_nodes = 16, .max_dependencies_per_node = 8 }, 8).success;
    defer registry.deinit();
    var owner = try key(&runtime, "owner");
    defer owner.deinit();
    var predicate = try key(&runtime, "predicate");
    defer predicate.deinit();
    var b = try key(&runtime, "b");
    defer b.deinit();
    var c = try key(&runtime, "c");
    defer c.deinit();
    var cancellation: fx.Cancellation = .{};
    var released: usize = 0;
    const env: TrackedEnv = .{ .allocator = std.testing.allocator, .registry = registry, .owner = owner.items, .predicate = predicate.items, .b = b.items, .c = c.items, .choose_b = true, .cancellation = &cancellation, .released = &released };
    const Outer = fx.Effect(u8, E, TrackedEnv);
    const Unit = fx.Effect(void, E, TrackedEnv);
    const Bracket = struct {
        fn body(_: u8) Unit {
            return Unit.succeed({});
        }
        fn release(_: u8, _: fx.PrimaryOutcome(E), current: *const TrackedEnv) Unit {
            var evaluated = comptime fx.AtomProgram.evaluate(TestAtoms.owner, TestAtoms.program, TestAtoms.registry, TestAtoms.mapError);
            return evaluated.flatMapOwned(current, struct {
                fn after(value: fx.AtomProgram.Published(u32), active: *const TrackedEnv) Unit {
                    var published = value;
                    published.deinit();
                    active.released.?.* += 1;
                    @constCast(active.cancellation).request();
                    return Unit.succeed({});
                }
            }.after);
        }
    };
    var result = fx.run(&env, comptime Outer.succeed(1).acquireRelease(Bracket.body, Bracket.release));
    defer result.deinit();
    try std.testing.expect(result == .interrupted);
    try std.testing.expectEqual(@as(usize, 1), released);
    try std.testing.expect(registry.graph.generation > 0);
}

test "ATOM-07 production evaluation allocates before commit and reports post-commit notification failure honestly" {
    const direct: R = .{ .allocator = std.testing.allocator };
    var owner = try key(&direct, "owner");
    defer owner.deinit();
    var predicate = try key(&direct, "predicate");
    defer predicate.deinit();
    var b = try key(&direct, "b");
    defer b.deinit();
    var c = try key(&direct, "c");
    defer c.deinit();
    const described = comptime fx.AtomProgram.evaluate(TestAtoms.owner, TestAtoms.program, TestAtoms.registry, TestAtoms.mapError);
    var position: usize = 0;
    var committed_failure_seen = false;
    while (true) : (position += 1) {
        var failing = std.testing.FailingAllocator.init(std.testing.allocator, .{});
        const runtime: R = .{ .allocator = failing.allocator() };
        const registry = fx.AtomRegistry.create(&runtime, .{ .max_nodes = 16, .max_dependencies_per_node = 8 }, 8).success;
        defer registry.deinit();
        var env: TrackedEnv = .{ .allocator = failing.allocator(), .registry = registry, .owner = owner.items, .predicate = predicate.items, .b = b.items, .c = c.items, .choose_b = false };
        var initial = fx.run(&env, described);
        initial.deinit();
        const previous = registry.graph.generation;
        env.choose_b = true;
        failing.fail_index = failing.alloc_index + position;
        var evaluated = fx.run(&env, described);
        defer evaluated.deinit();
        if (registry.graph.generation != previous) {
            try std.testing.expect(evaluated == .success and evaluated.success.value == .current);
            if (evaluated.success.update.?.invalidation != .success) committed_failure_seen = true;
        } else if (evaluated == .die) {
            try std.testing.expect(evaluated.die.kind == .out_of_memory);
        }
        // Inspection must not itself use the currently failing allocator.
        failing.fail_index = std.math.maxInt(usize);
        if (registry.graph.generation == previous) {
            var old = registry.graph.dependents(c.items);
            defer old.deinit();
            try std.testing.expectEqual(@as(usize, 1), old.success.items.len);
        }
        if (!failing.has_induced_failure) break;
    }
    try std.testing.expect(committed_failure_seen);
}

const CancelAfterCommit = struct {
    parent: std.mem.Allocator,
    registry: ?*fx.AtomRegistry = null,
    baseline: u64 = 0,
    cancellation: *fx.Cancellation,
    requested: bool = false,
    fn allocator(self: *@This()) std.mem.Allocator {
        return .{ .ptr = self, .vtable = &vtable };
    }
    fn alloc(raw: *anyopaque, len: usize, alignment: std.mem.Alignment, ra: usize) ?[*]u8 {
        const self: *@This() = @ptrCast(@alignCast(raw));
        if (self.registry) |registry| if (registry.graph.generation != self.baseline and !self.requested) {
            self.requested = true;
            self.cancellation.request();
        };
        return self.parent.rawAlloc(len, alignment, ra);
    }
    fn resize(raw: *anyopaque, memory: []u8, alignment: std.mem.Alignment, len: usize, ra: usize) bool {
        const self: *@This() = @ptrCast(@alignCast(raw));
        return self.parent.rawResize(memory, alignment, len, ra);
    }
    fn remap(raw: *anyopaque, memory: []u8, alignment: std.mem.Alignment, len: usize, ra: usize) ?[*]u8 {
        const self: *@This() = @ptrCast(@alignCast(raw));
        return self.parent.rawRemap(memory, alignment, len, ra);
    }
    fn free(raw: *anyopaque, memory: []u8, alignment: std.mem.Alignment, ra: usize) void {
        const self: *@This() = @ptrCast(@alignCast(raw));
        self.parent.rawFree(memory, alignment, ra);
    }
    const vtable: std.mem.Allocator.VTable = .{ .alloc = alloc, .resize = resize, .remap = remap, .free = free };
};

test "ATOM-05 acknowledged publication survives cancellation during notification and cleanup" {
    const direct: R = .{ .allocator = std.testing.allocator };
    var owner = try key(&direct, "owner");
    defer owner.deinit();
    var predicate = try key(&direct, "predicate");
    defer predicate.deinit();
    var b = try key(&direct, "b");
    defer b.deinit();
    var c = try key(&direct, "c");
    defer c.deinit();
    var cancellation: fx.Cancellation = .{};
    var hook: CancelAfterCommit = .{ .parent = std.testing.allocator, .cancellation = &cancellation };
    const runtime: R = .{ .allocator = hook.allocator() };
    const registry = fx.AtomRegistry.create(&runtime, .{ .max_nodes = 16, .max_dependencies_per_node = 8 }, 8).success;
    defer registry.deinit();
    hook.registry = registry;
    const env: TrackedEnv = .{ .allocator = hook.allocator(), .registry = registry, .owner = owner.items, .predicate = predicate.items, .b = b.items, .c = c.items, .choose_b = true, .cancellation = &cancellation };
    var result = fx.run(&env, comptime fx.AtomProgram.evaluate(TestAtoms.owner, TestAtoms.program, TestAtoms.registry, TestAtoms.mapError));
    defer result.deinit();
    try std.testing.expect(hook.requested and cancellation.isRequested());
    try std.testing.expect(result == .success and result.success.value == .current);
    try std.testing.expect(result.success.update.?.graph_generation == registry.graph.generation);
}

const Repeated = struct {
    fn second(_: u32) @TypeOf(fx.AtomProgram.read(TestAtoms.b)) {
        return fx.AtomProgram.read(TestAtoms.b);
    }
    const program = fx.AtomProgram.bind(fx.AtomProgram.read(TestAtoms.b), second);
};
test "ATOM-01 ATOM-04 ATOM-08 nested repeated reads deduplicate conflicting revisions stay stale and cycles or explicit limits preserve committed edges" {
    const Mode = enum { repeated, conflict, cycle, limit };
    const runtime: R = .{ .allocator = std.testing.allocator };
    var owner = try key(&runtime, "owner");
    defer owner.deinit();
    var predicate = try key(&runtime, "predicate");
    defer predicate.deinit();
    var b = try key(&runtime, "b");
    defer b.deinit();
    var c = try key(&runtime, "c");
    defer c.deinit();
    inline for (comptime std.meta.tags(Mode)) |mode| {
        const registry = fx.AtomRegistry.create(&runtime, .{ .max_nodes = 16, .max_dependencies_per_node = 1 }, 8).success;
        defer registry.deinit();
        var env: TrackedEnv = .{ .allocator = runtime.allocator, .registry = registry, .owner = owner.items, .predicate = predicate.items, .b = b.items, .c = c.items, .choose_b = true };
        var baseline = fx.run(&env, comptime fx.AtomProgram.evaluate(TestAtoms.owner, fx.AtomProgram.read(TestAtoms.c), TestAtoms.registry, TestAtoms.mapError));
        defer baseline.deinit();
        try std.testing.expect(baseline == .success and baseline.success.value == .current);
        const generation = registry.graph.generation;
        if (mode == .cycle) env.b = owner.items;
        if (mode == .conflict) env.change_b_during_read = true;
        const selected = comptime if (mode == .limit) TestAtoms.program else Repeated.program;
        var evaluated = fx.run(&env, comptime fx.AtomProgram.evaluate(TestAtoms.owner, selected, TestAtoms.registry, TestAtoms.mapError));
        defer evaluated.deinit();
        if (mode == .repeated) {
            try std.testing.expect(evaluated == .success and evaluated.success.value == .current);
            var dependents = registry.graph.dependents(b.items);
            defer dependents.deinit();
            try std.testing.expectEqual(@as(usize, 1), dependents.success.items.len);
        } else {
            if (mode == .conflict) try std.testing.expect(evaluated == .success and evaluated.success.value == .stale) else try std.testing.expect(evaluated == .fail and evaluated.fail == .registry);
            try std.testing.expectEqual(generation, registry.graph.generation);
            var previous = registry.graph.dependents(c.items);
            defer previous.deinit();
            try std.testing.expectEqual(@as(usize, 1), previous.success.items.len);
        }
    }
}

test "ATOM-03 ATOM-06 ATOM-07 concurrent tracked contexts preserve committed edges on suspended failure defect and interruption" {
    const Mode = enum { fail, die, interrupted, cancelled };
    const runtime: R = .{ .allocator = std.testing.allocator };
    var owner = try key(&runtime, "owner");
    defer owner.deinit();
    var other = try key(&runtime, "other");
    defer other.deinit();
    var predicate = try key(&runtime, "predicate");
    defer predicate.deinit();
    var b = try key(&runtime, "b");
    defer b.deinit();
    var c = try key(&runtime, "c");
    defer c.deinit();
    inline for (comptime std.meta.tags(Mode)) |mode| {
        const registry = fx.AtomRegistry.create(&runtime, .{ .max_nodes = 16, .max_dependencies_per_node = 8 }, 8).success;
        defer registry.deinit();
        var engine: fx.NativeIoExecutor = .{ .allocator = runtime.allocator, .io = runtime.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = true }, .maximum_owned = 8 };
        defer engine.deinit();
        var first_cancel: fx.Cancellation = .{};
        var second_cancel: fx.Cancellation = .{};
        var env: TrackedEnv = .{ .allocator = runtime.allocator, .registry = registry, .owner = owner.items, .predicate = predicate.items, .b = b.items, .c = c.items, .choose_b = false, .async_engine = engine.executor(), .cancellation = &first_cancel };
        const described = comptime fx.AtomProgram.evaluate(TestAtoms.owner, TestAtoms.program, TestAtoms.registry, TestAtoms.mapError);
        var baseline = fx.run(&env, described);
        baseline.deinit();
        var first_gate: gates.Gate(u32, E) = .{ .result = switch (mode) {
            .fail => .{ .fail = .expected },
            .die => .{ .die = fx.Defect.init("read defect") },
            .interrupted => .interrupted,
            .cancelled => .{ .success = 11 },
        } };
        env.gate = &first_gate;
        env.choose_b = true;
        var first = fx.runAsync(&env, described);
        defer first.deinit();
        first_gate.started.waitUncancelable(env.io);
        // A distinct evaluation commits concurrently while the first owns its
        // partially collected observations. No tracking context is global.
        var second_gate: gates.Gate(u32, E) = .{ .result = .{ .success = 37 } };
        var second_env = env;
        second_env.owner = other.items;
        second_env.cancellation = &second_cancel;
        second_env.gate = &second_gate;
        var second = fx.runAsync(&second_env, described);
        defer second.deinit();
        second_gate.started.waitUncancelable(env.io);
        second_gate.proceed.set(env.io);
        var completed = second.join();
        defer completed.deinit();
        try std.testing.expect(completed == .success and completed.success.value.current == 37);
        const generation = registry.graph.generation;
        if (mode == .cancelled) first.request() else first_gate.proceed.set(env.io);
        var abandoned = first.join();
        defer abandoned.deinit();
        switch (mode) {
            .fail => try std.testing.expect(abandoned == .fail and abandoned.fail == .expected),
            .die => try std.testing.expect(abandoned == .die),
            .interrupted, .cancelled => try std.testing.expect(abandoned == .interrupted),
        }
        try std.testing.expectEqual(generation, registry.graph.generation);
        var preserved = registry.graph.dependents(c.items);
        defer preserved.deinit();
        try std.testing.expectEqual(@as(usize, 1), preserved.success.items.len);
        try std.testing.expectEqualSlices(u8, owner.items, preserved.success.items[0].items);
        var committed = registry.graph.dependents(b.items);
        defer committed.deinit();
        try std.testing.expectEqual(@as(usize, 1), committed.success.items.len);
        try std.testing.expectEqualSlices(u8, other.items, committed.success.items[0].items);
        try std.testing.expect(first_gate.joined.load(.acquire) and first_gate.destroyed.load(.acquire));
        try std.testing.expect(second_gate.joined.load(.acquire) and second_gate.destroyed.load(.acquire));
    }
}
