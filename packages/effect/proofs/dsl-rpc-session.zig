const std = @import("std");
const fx = @import("effect");
const Threads = @import("support/thread_executor.zig").ThreadExecutor;
const PullGate = @import("support/dsl_gate.zig").Gate(void, i64);
const Counts = struct { acquired: usize = 0, projected: usize = 0, constructed: usize = 0, readers: usize = 0, reader_closed: usize = 0, environment_closed: usize = 0, release_calls: usize = 0, service_closed: usize = 0 };
const Root = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, async_engine: fx.Executor, counts: *Counts, gate: ?*PullGate = null };
const Service = struct {
    pub const effect_owned_payload = true;
    counts: *Counts,
    value: i64,
    pub fn deinit(self: *@This()) void {
        std.debug.assert(self.counts.environment_closed == self.counts.projected);
        self.counts.service_closed += 1;
    }
};
const DomainScope = fx.Scope(i64);
const HandlerEnv = struct {
    pub const effect_owned_payload = true;
    allocator: std.mem.Allocator,
    io: std.Io,
    async_engine: fx.Executor,
    counts: *Counts,
    service: *const Service,
    request: *const i64,
    scope_slot: *?*DomainScope,
    gate: ?*PullGate,
    reader_failure: bool,
    pub fn deinit(self: *@This()) void {
        std.debug.assert(self.counts.readers == self.counts.reader_closed);
        std.debug.assert(self.counts.service_closed == 0);
        self.counts.environment_closed += 1;
    }
};
const operation = fx.dsl.rpcStreamDefinition(.{ .name = "managed.stream", .version = 1, .request = fx.JsonCodec.signed_integer, .chunk = fx.JsonCodec.signed_integer, .terminal = fx.JsonCodec.null_value, .failure = fx.JsonCodec.signed_integer });
const Event = @TypeOf(operation).Event;
const Stream = fx.Stream(Event, i64, HandlerEnv);
const Pull = fx.Effect(fx.Chunk(Event), i64, HandlerEnv);
const Iterator = struct {
    service: *const Service,
    events: [2]Event,
    index: usize = 0,
};
const Streaming = struct {
    fn allocate(env: *const HandlerEnv) fx.Exit(*Iterator, i64) {
        std.debug.assert(env.counts.service_closed == 0);
        const iterator = env.allocator.create(Iterator) catch return .{ .die = fx.Defect.out_of_memory };
        iterator.* = .{ .service = env.service, .events = .{ .{ .chunk = env.service.value + env.request.* }, .{ .terminal = {} } } };
        env.counts.readers += 1;
        return .{ .success = iterator };
    }
    fn emit(iterator: *const Iterator, _: void, _: *const HandlerEnv) Pull {
        return Pull.fromBorrowed(Iterator, iterator, struct {
            fn read(current: *const Iterator, supplied: *const HandlerEnv) fx.Exit(fx.Chunk(Event), i64) {
                std.debug.assert(current.service.counts.service_closed == 0);
                if (current.index == current.events.len) return .{ .success = .end };
                const mutable = @constCast(current);
                const index = mutable.index;
                mutable.index += 1;
                _ = supplied;
                return .{ .success = fx.Chunk(Event).borrowed(current.events[index .. index + 1]) };
            }
        }.read);
    }
    fn next(iterator: *Iterator) Pull {
        return Pull.deferBorrowed(Iterator, iterator, struct {
            fn start(current: *const Iterator, env: *const HandlerEnv) Pull {
                const Unit = fx.Effect(void, i64, HandlerEnv);
                var seed = if (env.gate != null and current.index == 0) Unit.fromAsync(struct {
                    fn pending(supplied: *const HandlerEnv) fx.Async(void, i64).Start {
                        return supplied.gate.?.start();
                    }
                }.pending) else Unit.succeed({});
                return seed.flatMapCapturedOwned(env, fx.Chunk(Event), current, struct {
                    fn resumed(stored: *const *const Iterator, _: void, supplied: *const HandlerEnv) Pull {
                        return emit(stored.*, {}, supplied);
                    }
                }.resumed);
            }
        }.start);
    }
    fn release(iterator: *Iterator) fx.Effect(void, i64, HandlerEnv) {
        return fx.Effect(void, i64, HandlerEnv).fromBorrowed(Iterator, iterator, struct {
            fn close(current: *const Iterator, env: *const HandlerEnv) fx.Exit(void, i64) {
                std.debug.assert(current.service.counts.service_closed == 0);
                env.counts.reader_closed += 1;
                env.allocator.destroy(@constCast(current));
                return if (env.reader_failure) .{ .fail = 66 } else .{ .success = {} };
            }
        }.close);
    }
    fn construct(_: *const i64, env: *const HandlerEnv) Stream {
        env.counts.constructed += 1;
        return Stream.fromPull(fx.Effect(*Iterator, i64, HandlerEnv).fromOutcome(allocate), next, release);
    }
};
const handler = operation.handledBy(HandlerEnv, Streaming.construct);
fn Wire(comptime T: type) type {
    return struct {
        fn decode(_: *const fx.OwnedSlice(u8)) fx.Effect(T, fx.SchemaIssue.Issue, HandlerEnv) {
            return fx.Effect(T, fx.SchemaIssue.Issue, HandlerEnv).die(fx.Defect.init("wire decoding is outside this open/reader proof"));
        }
        fn encode(_: *const T) fx.Effect(fx.OwnedSlice(u8), fx.SchemaIssue.Issue, HandlerEnv) {
            return fx.Effect(fx.OwnedSlice(u8), fx.SchemaIssue.Issue, HandlerEnv).die(fx.Defect.init("wire encoding is outside this open/reader proof"));
        }
        const value = fx.RpcWireCodec(T, HandlerEnv).init(decode, encode);
    };
}
const Server = fx.RpcStreamServer(handler, Wire(i64).value, Wire(i64).value, Wire(void).value, Wire(i64).value, u8, struct {
    fn select(_: *const HandlerEnv) *const fx.RpcSession(HandlerEnv, u8) {
        unreachable;
    }
}.select, struct {
    fn select(_: *const HandlerEnv) fx.RpcSessionPolicy {
        unreachable;
    }
}.select);
const Body = fx.Effect(u32, Server.Failure, HandlerEnv);
const ConfigurationOwner = struct {
    pub const effect_owned_payload = true;
    allocator: std.mem.Allocator,
    bytes: []u8,
    live: *usize,
    fn create(allocator: std.mem.Allocator, live: *usize) !@This() {
        const bytes = try allocator.dupe(u8, "managed session");
        live.* += 1;
        return .{ .allocator = allocator, .bytes = bytes, .live = live };
    }
    pub fn deinit(self: *@This()) void {
        self.allocator.free(self.bytes);
        self.live.* -= 1;
    }
};
const Callbacks = struct {
    fn acquire(_: anytype, _: *const Root) fx.Effect(Service, u8, Root) {
        return fx.Effect(Service, u8, Root).fromOutcome(struct {
            fn create(env: *const Root) fx.Exit(Service, u8) {
                env.counts.acquired += 1;
                return .{ .success = .{ .counts = env.counts, .value = 41 } };
            }
        }.create);
    }
    fn project(configuration: anytype, service: *const Service, env: *const Root) HandlerEnv {
        env.counts.projected += 1;
        return .{ .allocator = env.allocator, .io = env.io, .async_engine = env.async_engine, .counts = env.counts, .service = service, .request = &configuration.request, .scope_slot = configuration.scope_slot, .gate = env.gate, .reader_failure = configuration.reader_failure };
    }
    fn domain(problem: i64) Server.Failure {
        return .{ .domain = problem };
    }
    const Loop = struct { pull: Pull, count: u32, early: bool };
    fn consume(pull: Pull, count: u32, early: bool, env: *const HandlerEnv) Body {
        var pending = pull.borrow();
        var mapped = pending.mapErrorOwned(env, domain);
        return mapped.flatMapCapturedOwned(env, u32, Loop{ .pull = pull.borrow(), .count = count, .early = early }, struct {
            fn read(stored: *const Loop, chunk: fx.Chunk(Event), current: *const HandlerEnv) Body {
                var owned = chunk;
                defer owned.deinit();
                if (owned == .end) return Body.succeed(stored.count);
                const next_count = stored.count + @as(u32, @intCast(owned.items.values.len));
                std.debug.assert(current.counts.constructed == 1);
                std.debug.assert(current.counts.service_closed == 0);
                if (stored.early) return Body.succeed(next_count);
                return consume(stored.pull, next_count, false, current);
            }
        }.read);
    }
    fn use(configuration: anytype, scope: *DomainScope, env: *const HandlerEnv) Body {
        std.debug.assert(std.mem.eql(u8, configuration.owner.bytes, "managed session"));
        configuration.scope_slot.* = scope;
        var opened = Server.open(struct {
            fn request(current: *const HandlerEnv) *const i64 {
                return current.request;
            }
        }.request, struct {
            fn selected(current: *const HandlerEnv) *DomainScope {
                return current.scope_slot.*.?;
            }
        }.selected, struct {
            fn context(_: *const HandlerEnv) fx.RpcContext {
                return fx.RpcContext.empty;
            }
        }.context);
        const C = @typeInfo(@TypeOf(configuration)).pointer.child;
        return opened.acquireReleaseCapturedOwned(env, configuration, struct {
            fn run(stored: *const *const C, reader: Server.Reader, current: *const HandlerEnv) Body {
                return consume(reader.pull.borrow(), 0, stored.*.early, current);
            }
        }.run, struct {
            fn close(_: *const *const C, reader: Server.Reader, _: fx.PrimaryOutcome(Server.Failure), _: *const HandlerEnv) fx.Effect(void, Server.Failure, HandlerEnv) {
                var owned = reader;
                owned.deinit();
                return fx.Effect(void, Server.Failure, HandlerEnv).succeed({});
            }
        }.close);
    }
    fn release(configuration: anytype, service: *const Service, _: anytype, env: *const Root) fx.Effect(void, u8, Root) {
        std.debug.assert(std.mem.eql(u8, configuration.owner.bytes, "managed session"));
        std.debug.assert(service.counts.readers == service.counts.reader_closed);
        std.debug.assert(service.counts.environment_closed == service.counts.projected);
        env.counts.release_calls += 1;
        return fx.Effect(void, u8, Root).succeed({});
    }
};

test "RPC-04 RPC-05 managed RPC session uses the canonical stream server and keeps middleware alive through EOF early close and reader cleanup failure" {
    inline for (.{ .eof, .early, .cleanup_failed }) |mode| {
        var live: usize = 0;
        var counts: Counts = .{};
        var scope_slot: ?*DomainScope = null;
        var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
        defer threads.deinit();
        const env: Root = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &counts };
        var description = fx.dsl.withRpcSession(&env, .{
            .definition = fx.dsl.declareMetadata(.{ .server = Server, .acquire = Callbacks.acquire, .project = Callbacks.project, .use = Callbacks.use, .release = Callbacks.release }),
            .inputs = .{ .owner = try ConfigurationOwner.create(std.testing.allocator, &live), .request = @as(i64, @intCast(counts.acquired + 1)), .scope_slot = &scope_slot, .early = mode == .early, .reader_failure = mode == .cleanup_failed },
        });
        comptime if (@TypeOf(description) != fx.Effect(u32, fx.RpcMiddlewareFailure(Server.Failure, u8), Root)) @compileError("session must be an exact Effect");
        try std.testing.expectEqual(@as(usize, 0), counts.acquired);
        var result = fx.run(&env, description);
        description.deinit();
        defer result.deinit();
        if (mode == .cleanup_failed) {
            try std.testing.expect(result == .finalization_failed);
            try std.testing.expectEqual(@as(i64, 66), result.finalization_failed.first.release_failure.domain.domain);
        } else try std.testing.expectEqual(@as(u32, if (mode == .early) 1 else 2), result.success);
        try std.testing.expectEqual(@as(usize, 1), counts.constructed);
        try std.testing.expectEqual(@as(usize, 1), counts.reader_closed);
        try std.testing.expectEqual(@as(usize, 1), counts.environment_closed);
        try std.testing.expectEqual(@as(usize, 1), counts.release_calls);
        try std.testing.expectEqual(@as(usize, 1), counts.service_closed);
        try std.testing.expectEqual(@as(usize, 0), live);
    }
}

test "RPC-04 LIFE-12 managed session destroys pending stream work before reader environment service and configuration release" {
    inline for (.{ .success, .fail, .die, .interrupted, .destroy }) |mode| {
        var live: usize = 0;
        var counts: Counts = .{};
        var scope_slot: ?*DomainScope = null;
        var gate: PullGate = .{ .result = switch (mode) {
            .success, .destroy => .{ .success = {} },
            .fail => .{ .fail = 77 },
            .die => .{ .die = fx.Defect.init("session pull") },
            .interrupted => .interrupted,
            else => unreachable,
        } };
        var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
        defer threads.deinit();
        const env: Root = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &counts, .gate = &gate };
        var description = fx.dsl.withRpcSession(&env, .{
            .definition = fx.dsl.declareMetadata(.{ .server = Server, .acquire = Callbacks.acquire, .project = Callbacks.project, .use = Callbacks.use, .release = Callbacks.release }),
            .inputs = .{ .owner = try ConfigurationOwner.create(std.testing.allocator, &live), .request = @as(i64, @intCast(counts.acquired + 1)), .scope_slot = &scope_slot, .early = false, .reader_failure = false },
        });
        var pending = fx.runAsync(&env, description);
        description.deinit();
        gate.started.waitUncancelable(env.io);
        try std.testing.expectEqual(@as(usize, 1), counts.constructed);
        try std.testing.expectEqual(@as(usize, 1), counts.readers);
        try std.testing.expectEqual(@as(usize, 0), counts.reader_closed);
        try std.testing.expectEqual(@as(usize, 0), counts.environment_closed);
        try std.testing.expectEqual(@as(usize, 0), counts.service_closed);
        try std.testing.expectEqual(@as(usize, 1), live);
        if (mode == .destroy) {
            pending.deinit();
            try std.testing.expect(gate.cancelled.load(.acquire));
        } else {
            gate.proceed.set(env.io);
            var result = pending.join();
            pending.deinit();
            defer result.deinit();
            switch (mode) {
                .success => try std.testing.expectEqual(@as(u32, 2), result.success),
                .fail => try std.testing.expectEqual(@as(i64, 77), result.fail.domain.domain),
                .die => try std.testing.expect(result == .die),
                .interrupted => try std.testing.expect(result == .interrupted),
                else => unreachable,
            }
        }
        try std.testing.expect(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
        try std.testing.expectEqual(@as(usize, 1), counts.reader_closed);
        try std.testing.expectEqual(@as(usize, 1), counts.environment_closed);
        try std.testing.expectEqual(@as(usize, 1), counts.release_calls);
        try std.testing.expectEqual(@as(usize, 1), counts.service_closed);
        try std.testing.expectEqual(@as(usize, 0), live);
    }
}

test "RPC-04 LIFE-12 managed request scope drains admitted child work after the serving body returns before middleware release" {
    const Child = struct {
        fn use(_: anytype, scope: *DomainScope, env: *const HandlerEnv) Body {
            const Unit = fx.Effect(void, i64, HandlerEnv);
            const pending = comptime Unit.fromAsync(struct {
                fn start(current: *const HandlerEnv) fx.Async(void, i64).Start {
                    std.debug.assert(current.counts.service_closed == 0);
                    return current.gate.?.start();
                }
            }.start);
            var forked = pending.fork(scope);
            var mapped = forked.mapErrorOwned(env, Callbacks.domain);
            return mapped.flatMapOwned(env, struct {
                fn returned(_: *fx.Fiber(void, i64), current: *const HandlerEnv) Body {
                    current.gate.?.started.waitUncancelable(current.io);
                    std.debug.assert(current.counts.environment_closed == 0);
                    return Body.succeed(5);
                }
            }.returned);
        }
    };
    var live: usize = 0;
    var counts: Counts = .{};
    var scope_slot: ?*DomainScope = null;
    var gate: PullGate = .{ .result = .{ .success = {} } };
    var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = true, .cpu = false, .blocking = false } };
    defer threads.deinit();
    const env: Root = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &counts, .gate = &gate };
    var description = fx.dsl.withRpcSession(&env, .{
        .definition = fx.dsl.declareMetadata(.{ .server = Server, .acquire = Callbacks.acquire, .project = Callbacks.project, .use = Child.use, .release = Callbacks.release }),
        .inputs = .{ .owner = try ConfigurationOwner.create(std.testing.allocator, &live), .request = @as(i64, @intCast(counts.acquired + 1)), .scope_slot = &scope_slot, .early = false, .reader_failure = false },
    });
    var result = fx.run(&env, description);
    description.deinit();
    defer result.deinit();
    try std.testing.expectEqual(@as(u32, 5), result.success);
    try std.testing.expect(gate.cancelled.load(.acquire) and gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
    try std.testing.expectEqual(@as(usize, 1), counts.environment_closed);
    try std.testing.expectEqual(@as(usize, 1), counts.release_calls);
    try std.testing.expectEqual(@as(usize, 1), counts.service_closed);
    try std.testing.expectEqual(@as(usize, 0), live);
}

test "RPC-04 RPC-05 LIFE-07 managed RPC session rolls back every allocation failure and preserves reader cleanup errors" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator, early: bool, reader_failure: bool) !void {
            var live: usize = 0;
            defer std.debug.assert(live == 0);
            var counts: Counts = .{};
            var scope_slot: ?*DomainScope = null;
            var threads: Threads = .{ .allocator = allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
            defer threads.deinit();
            const env: Root = .{ .allocator = allocator, .async_engine = threads.executor(), .counts = &counts };
            var description = fx.dsl.withRpcSession(&env, .{
                .definition = fx.dsl.declareMetadata(.{ .server = Server, .acquire = Callbacks.acquire, .project = Callbacks.project, .use = Callbacks.use, .release = Callbacks.release }),
                .inputs = .{ .owner = try ConfigurationOwner.create(allocator, &live), .request = @as(i64, @intCast(counts.acquired + 1)), .scope_slot = &scope_slot, .early = early, .reader_failure = reader_failure },
            });
            defer description.deinit();
            var result = fx.run(&env, description);
            defer result.deinit();
            try std.testing.expectEqual(counts.readers, counts.reader_closed);
            try std.testing.expectEqual(counts.projected, counts.environment_closed);
            try std.testing.expectEqual(counts.acquired, counts.release_calls);
            try std.testing.expectEqual(counts.acquired, counts.service_closed);
            if (allocationFailure(result)) return error.OutOfMemory;
            if (reader_failure) {
                try std.testing.expect(result == .finalization_failed);
                try std.testing.expectEqual(@as(i64, 66), result.finalization_failed.first.release_failure.domain.domain);
            } else try std.testing.expectEqual(@as(u32, if (early) 1 else 2), result.success);
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{ false, false });
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{ true, false });
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{ false, true });
}

fn allocationFailure(result: anytype) bool {
    if (result == .die) return result.die.kind == .out_of_memory;
    if (result != .finalization_failed) return false;
    const detail = result.finalization_failed;
    if (detail.primary == .defected and detail.primary.defected.kind == .out_of_memory) return true;
    if (detail.first == .release_defect and detail.first.release_defect.kind == .out_of_memory) return true;
    var cursor = detail.rest;
    while (cursor) |node| {
        if (node.failure == .release_defect and node.failure.release_defect.kind == .out_of_memory) return true;
        cursor = node.next;
    }
    return false;
}
