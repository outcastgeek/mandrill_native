const std = @import("std");
const fx = @import("effect");
const Threads = @import("support/thread_executor.zig").ThreadExecutor;
const Gate = @import("support/dsl_gate.zig").Gate(void, i64);
const Construction = struct { allocator: std.mem.Allocator };
const Counts = struct { constructed: usize = 0, readers: usize = 0, closed: usize = 0 };
const Env = struct {
    allocator: std.mem.Allocator,
    io: std.Io = std.testing.io,
    async_engine: fx.Executor,
    live: *usize,
    counts: *Counts,
    bytes_seen: *?[]const u8,
    context_seen: *?*const fx.RpcContext,
    value: *i64,
    gate: ?*Gate = null,
    request_seen: ?*?*const i64 = null,
};
const Owned = struct {
    pub const effect_owned_payload = true;
    allocator: std.mem.Allocator,
    bytes: []u8,
    live: *usize,
    fn init(allocator: std.mem.Allocator, live: *usize) !@This() {
        const bytes = try allocator.dupe(u8, "captured streaming configuration");
        live.* += 1;
        return .{ .allocator = allocator, .bytes = bytes, .live = live };
    }
    pub fn deinit(self: *@This()) void {
        self.allocator.free(self.bytes);
        self.live.* -= 1;
    }
};
const operation = fx.dsl.rpcStreamDefinition(.{ .name = "captured.stream", .version = 1, .request = fx.JsonCodec.signed_integer, .chunk = fx.JsonCodec.signed_integer, .terminal = fx.JsonCodec.null_value, .failure = fx.JsonCodec.signed_integer });
const Event = @TypeOf(operation).Event;
const Stream = fx.Stream(Event, i64, Env);
const Pull = fx.Effect(fx.Chunk(Event), i64, Env);
const Iterator = struct { events: [2]Event, index: usize = 0 };
const Native = struct {
    fn acquire(env: *const Env) fx.Exit(*Iterator, i64) {
        std.debug.assert(env.live.* == 1);
        const iterator = env.allocator.create(Iterator) catch return .{ .die = fx.Defect.out_of_memory };
        iterator.* = .{ .events = .{ .{ .chunk = env.value.* }, .{ .terminal = {} } } };
        env.counts.readers += 1;
        return .{ .success = iterator };
    }
    fn pull(iterator: *Iterator) Pull {
        return Pull.deferBorrowed(Iterator, iterator, struct {
            fn start(current: *const Iterator, env: *const Env) Pull {
                const Unit = fx.Effect(void, i64, Env);
                var ready = if (env.gate != null) Unit.fromAsync(struct {
                    fn pending(supplied: *const Env) fx.Async(void, i64).Start {
                        return supplied.gate.?.start();
                    }
                }.pending) else Unit.succeed({});
                return ready.flatMapCapturedOwned(env, fx.Chunk(Event), current, struct {
                    fn read(stored: *const *const Iterator, _: void, _: *const Env) Pull {
                        return Pull.fromBorrowed(Iterator, stored.*, struct {
                            fn emit(position: *const Iterator, active: *const Env) fx.Exit(fx.Chunk(Event), i64) {
                                std.debug.assert(active.live.* == 1);
                                std.debug.assert(std.mem.eql(u8, active.bytes_seen.*.?, "captured streaming configuration"));
                                std.debug.assert(active.context_seen.*.?.correlation == 43);
                                if (active.request_seen) |seen| std.debug.assert(seen.*.?.* == 17);
                                if (position.index == position.events.len) return .{ .success = .end };
                                const mutable = @constCast(position);
                                const index = mutable.index;
                                mutable.index += 1;
                                return .{ .success = fx.Chunk(Event).borrowed(position.events[index .. index + 1]) };
                            }
                        }.emit);
                    }
                }.read);
            }
        }.start);
    }
    fn release(iterator: *Iterator) fx.Effect(void, i64, Env) {
        return fx.Effect(void, i64, Env).fromBorrowed(Iterator, iterator, struct {
            fn close(current: *const Iterator, env: *const Env) fx.Exit(void, i64) {
                std.debug.assert(env.live.* == 1);
                std.debug.assert(std.mem.eql(u8, env.bytes_seen.*.?, "captured streaming configuration"));
                env.counts.closed += 1;
                env.allocator.destroy(@constCast(current));
                return .{ .success = {} };
            }
        }.close);
    }
};
const Callback = struct {
    fn handle(configuration: anytype, context: *const fx.RpcContext, request: *const i64, env: *const Env) Stream {
        std.debug.assert(configuration.offset == 25);
        env.counts.constructed += 1;
        env.bytes_seen.* = configuration.owner.bytes;
        env.context_seen.* = context;
        if (env.request_seen) |seen| seen.* = request;
        env.value.* = request.* + configuration.offset;
        return Stream.fromPull(fx.Effect(*Iterator, i64, Env).fromOutcome(Native.acquire), Native.pull, Native.release);
    }
};

test "RPC-02 DECL-03 CAP-02 runtime captured streaming handler infers channels and retains noncloneable configuration through native reader scope cleanup" {
    var live: usize = 0;
    var counts: Counts = .{};
    var bytes_seen: ?[]const u8 = null;
    var context_seen: ?*const fx.RpcContext = null;
    var value: i64 = 0;
    var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
    defer threads.deinit();
    const creation: Construction = .{ .allocator = std.testing.allocator };
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .live = &live, .counts = &counts, .bytes_seen = &bytes_seen, .context_seen = &context_seen, .value = &value };
    var description = fx.dsl.makeRpcHandler(&creation, .{
        .definition = fx.dsl.declareMetadata(.{ .operation = operation, .handler = Callback.handle }),
        .inputs = .{ .owner = try Owned.init(std.testing.allocator, &live), .offset = @as(i64, 25) },
    });
    try std.testing.expectEqual(@as(usize, 0), counts.constructed);
    var made = fx.run(&creation, description);
    description.deinit();
    const Handler = @TypeOf(made).Success;
    comptime if (@TypeOf(description) != fx.Effect(Handler, i64, Construction)) @compileError("stream handler construction must be an exact Effect with independent construction environment");
    var scope: fx.Scope(i64) = .init(&env);
    const request: i64 = 17;
    var context = fx.RpcContext.empty;
    context.correlation = 43;
    var opening = made.success.openWith(&scope, context, &request);
    comptime if (@TypeOf(opening) != fx.Effect(fx.StreamReader(Event, i64, Env), i64, Env)) @compileError("captured stream opening must return the existing reader Effect");
    made.deinit();
    var opened = fx.run(&env, opening);
    opening.deinit();
    try std.testing.expectEqual(@as(usize, 1), live);
    var chunk = fx.run(&env, opened.success.pull.borrow());
    try std.testing.expectEqual(@as(i64, 42), chunk.success.items.values[0].chunk);
    chunk.deinit();
    opened.deinit();
    try std.testing.expectEqual(@as(usize, 1), live);
    var closed = fx.run(&env, fx.Effect(void, i64, Env).closeScope(&scope));
    closed.deinit();
    scope.deinit();
    try std.testing.expectEqual(@as(usize, 1), counts.closed);
    try std.testing.expectEqual(@as(usize, 0), live);
}

test "RPC-02 CAP-04 LIFE-12 captured stream scope retains configuration and pointer context after every handler description is released while pulls are pending" {
    inline for (.{ .success, .fail, .die, .interrupted, .finalization_failed, .destroy }) |mode| {
        var live: usize = 0;
        var counts: Counts = .{};
        var bytes_seen: ?[]const u8 = null;
        var context_seen: ?*const fx.RpcContext = null;
        var value: i64 = 0;
        var gate: Gate = .{ .result = switch (mode) {
            .success, .destroy => .{ .success = {} },
            .fail => .{ .fail = 78 },
            .die => .{ .die = fx.Defect.init("captured pull") },
            .interrupted => .interrupted,
            .finalization_failed => .{ .finalization_failed = .{ .primary = .{ .failed = 79 }, .first = .{ .release_failure = 80 } } },
            else => unreachable,
        } };
        var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
        defer threads.deinit();
        const creation: Construction = .{ .allocator = std.testing.allocator };
        const env: Env = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .live = &live, .counts = &counts, .bytes_seen = &bytes_seen, .context_seen = &context_seen, .value = &value, .gate = &gate };
        var described = fx.dsl.makeRpcHandler(&creation, .{
            .definition = fx.dsl.declareMetadata(.{ .operation = operation, .handler = Callback.handle }),
            .inputs = .{ .owner = try Owned.init(std.testing.allocator, &live), .offset = @as(i64, 25) },
        });
        var made = fx.run(&creation, described);
        described.deinit();
        var scope: fx.Scope(i64) = .init(&env);
        const request: i64 = 17;
        var context = fx.RpcContext.empty;
        context.correlation = 43;
        var opening = made.success.openWith(&scope, context, &request);
        made.deinit();
        var opened = fx.run(&env, opening);
        opening.deinit();
        var pending = fx.runAsync(&env, opened.success.pull.borrow());
        gate.started.waitUncancelable(env.io);
        try std.testing.expectEqual(@as(usize, 1), live);
        try std.testing.expectEqual(@as(usize, 0), counts.closed);
        if (mode == .destroy) {
            pending.deinit();
            try std.testing.expect(gate.cancelled.load(.acquire));
        } else {
            gate.proceed.set(env.io);
            var result = pending.join();
            pending.deinit();
            defer result.deinit();
            switch (mode) {
                .success => try std.testing.expectEqual(@as(i64, 42), result.success.items.values[0].chunk),
                .fail => try std.testing.expectEqual(@as(i64, 78), result.fail),
                .die => try std.testing.expect(result == .die),
                .interrupted => try std.testing.expect(result == .interrupted),
                .finalization_failed => {
                    try std.testing.expectEqual(@as(i64, 79), result.finalization_failed.primary.failed);
                    try std.testing.expectEqual(@as(i64, 80), result.finalization_failed.first.release_failure);
                },
                else => unreachable,
            }
        }
        try std.testing.expect(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
        opened.deinit();
        try std.testing.expectEqual(@as(usize, 1), live);
        var closed = fx.run(&env, fx.Effect(void, i64, Env).closeScope(&scope));
        closed.deinit();
        scope.deinit();
        try std.testing.expectEqual(@as(usize, 1), counts.closed);
        try std.testing.expectEqual(@as(usize, 0), live);
    }
}

test "RPC-02 LIFE-07 captured streaming construction opening and native reader allocation failures release initialized storage exactly once" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            var live: usize = 0;
            defer std.debug.assert(live == 0);
            var counts: Counts = .{};
            var bytes_seen: ?[]const u8 = null;
            var context_seen: ?*const fx.RpcContext = null;
            var value: i64 = 0;
            var threads: Threads = .{ .allocator = allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
            defer threads.deinit();
            const creation: Construction = .{ .allocator = allocator };
            const env: Env = .{ .allocator = allocator, .async_engine = threads.executor(), .live = &live, .counts = &counts, .bytes_seen = &bytes_seen, .context_seen = &context_seen, .value = &value };
            var described = fx.dsl.makeRpcHandler(&creation, .{
                .definition = fx.dsl.declareMetadata(.{ .operation = operation, .handler = Callback.handle }),
                .inputs = .{ .owner = try Owned.init(allocator, &live), .offset = @as(i64, 25) },
            });
            defer described.deinit();
            var made = fx.run(&creation, described);
            defer made.deinit();
            if (made == .die and made.die.kind == .out_of_memory) return error.OutOfMemory;
            var scope: fx.Scope(i64) = .init(&env);
            const request: i64 = 17;
            var context = fx.RpcContext.empty;
            context.correlation = 43;
            var opening = made.success.openWith(&scope, context, &request);
            defer opening.deinit();
            var opened = fx.run(&env, opening);
            defer opened.deinit();
            var closed = fx.run(&env, fx.Effect(void, i64, Env).closeScope(&scope));
            closed.deinit();
            scope.deinit();
            try std.testing.expectEqual(counts.readers, counts.closed);
            if (opened == .die and opened.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expect(opened == .success);
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

test "RPC-02 CAP-04 unused captured stream opening and rejected closed scope run no handler and discard generated captures" {
    inline for (.{ false, true }) |execute| {
        var live: usize = 0;
        var counts: Counts = .{};
        var bytes_seen: ?[]const u8 = null;
        var context_seen: ?*const fx.RpcContext = null;
        var value: i64 = 0;
        var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
        defer threads.deinit();
        const creation: Construction = .{ .allocator = std.testing.allocator };
        const env: Env = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .live = &live, .counts = &counts, .bytes_seen = &bytes_seen, .context_seen = &context_seen, .value = &value };
        var described = fx.dsl.makeRpcHandler(&creation, .{
            .definition = fx.dsl.declareMetadata(.{ .operation = operation, .handler = Callback.handle }),
            .inputs = .{ .owner = try Owned.init(std.testing.allocator, &live), .offset = @as(i64, 25) },
        });
        var made = fx.run(&creation, described);
        described.deinit();
        var scope: fx.Scope(i64) = .init(&env);
        var closed = fx.run(&env, fx.Effect(void, i64, Env).closeScope(&scope));
        closed.deinit();
        const request: i64 = 17;
        var opening = made.success.open(&scope, &request);
        made.deinit();
        if (execute) {
            var rejected = fx.run(&env, opening);
            defer rejected.deinit();
            try std.testing.expect(rejected == .die);
        }
        opening.deinit();
        scope.deinit();
        try std.testing.expectEqual(@as(usize, 0), counts.constructed);
        try std.testing.expectEqual(@as(usize, 0), counts.readers);
        try std.testing.expectEqual(@as(usize, 0), live);
    }
}

fn RegistryUse(comptime selected: anytype) type {
    return struct {
        fn run(registry: anytype, inputs: anytype, env: *const Env) fx.Effect(i64, i64, Env) {
            env.context_seen.* = &inputs.context;
            return fx.dsl.scopedCaptured(env, .{ .registry = registry, .request = inputs.request, .context = inputs.context }, .{ .body = struct {
                fn scoped(stored: anytype, scope: *fx.Scope(i64), current: *const Env) fx.Effect(i64, i64, Env) {
                    var opening = stored.registry.openWith(scope, selected, stored.context, stored.request);
                    return opening.acquireReleaseCapturedOwned(current, {}, struct {
                        fn read(_: *const void, reader: fx.StreamReader(Event, i64, Env), supplied: *const Env) fx.Effect(i64, i64, Env) {
                            var pull = reader.pull.borrow();
                            return pull.mapOwned(supplied, struct {
                                fn value(chunk: fx.Chunk(Event)) i64 {
                                    var owned = chunk;
                                    defer owned.deinit();
                                    return owned.items.values[0].chunk;
                                }
                            }.value);
                        }
                    }.read, struct {
                        fn close(_: *const void, reader: fx.StreamReader(Event, i64, Env), _: fx.PrimaryOutcome(i64), _: *const Env) fx.Effect(void, i64, Env) {
                            var owned = reader;
                            owned.deinit();
                            return fx.Effect(void, i64, Env).succeed({});
                        }
                    }.close);
                }
            }.scoped });
        }
    };
}
const static_operation = blk: {
    var result = operation;
    result.name = "captured.stream.static";
    break :blk result;
};
const Static = struct {
    fn handle(_: fx.RpcContext, request: *const i64, env: *const Env) Stream {
        env.counts.constructed += 1;
        env.bytes_seen.* = "captured streaming configuration";
        env.value.* = request.* + 100;
        return Stream.fromPull(fx.Effect(*Iterator, i64, Env).fromOutcome(Native.acquire), Native.pull, Native.release);
    }
};

test "RPC-02 DECL-04 runtime registry opens direct and original static stream bindings through one closed protocol and generated request scope" {
    inline for (.{ operation, static_operation }) |selected| {
        var live: usize = 0;
        var counts: Counts = .{};
        var bytes_seen: ?[]const u8 = null;
        var context_seen: ?*const fx.RpcContext = null;
        var value: i64 = 0;
        var context = fx.RpcContext.empty;
        context.correlation = 43;
        var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
        defer threads.deinit();
        const env: Env = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .live = &live, .counts = &counts, .bytes_seen = &bytes_seen, .context_seen = &context_seen, .value = &value };
        var described = fx.dsl.withRpcRegistry(&env, .{
            .definition = fx.dsl.declareMetadata(.{ .use = RegistryUse(selected).run }),
            .inputs = .{ .bindings = .{
                .{ .definition = fx.dsl.declareMetadata(.{ .operation = operation, .handler = Callback.handle }), .inputs = .{ .owner = try Owned.init(std.testing.allocator, &live), .offset = @as(i64, 25) } },
                fx.dsl.rpcStatic(static_operation.handledByContext(Env, Static.handle)),
            }, .request = @as(i64, @intCast(counts.constructed + 17)), .context = context },
        });
        var result = fx.run(&env, described);
        described.deinit();
        defer result.deinit();
        try std.testing.expectEqual(@as(i64, if (std.mem.eql(u8, selected.name, operation.name)) 42 else 117), result.success);
        try std.testing.expectEqual(@as(usize, 1), counts.constructed);
        try std.testing.expectEqual(@as(usize, 1), counts.closed);
        try std.testing.expectEqual(@as(usize, 0), live);
    }
}

test "RPC-02 DECL-04 runtime registry retains a constructed streaming handler after every external handler owner is released" {
    var live: usize = 0;
    var counts: Counts = .{};
    var bytes_seen: ?[]const u8 = null;
    var context_seen: ?*const fx.RpcContext = null;
    var value: i64 = 0;
    var context = fx.RpcContext.empty;
    context.correlation = 43;
    var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
    defer threads.deinit();
    const creation: Construction = .{ .allocator = std.testing.allocator };
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .live = &live, .counts = &counts, .bytes_seen = &bytes_seen, .context_seen = &context_seen, .value = &value };
    var source = fx.dsl.makeRpcHandler(&creation, .{
        .definition = fx.dsl.declareMetadata(.{ .operation = operation, .handler = Callback.handle }),
        .inputs = .{ .owner = try Owned.init(std.testing.allocator, &live), .offset = @as(i64, 25) },
    });
    var made = fx.run(&creation, source);
    source.deinit();
    var described = fx.dsl.withRpcRegistry(&env, .{
        .definition = fx.dsl.declareMetadata(.{ .use = RegistryUse(operation).run }),
        .inputs = .{ .bindings = .{made.success.retain()}, .request = @as(i64, @intCast(counts.constructed + 17)), .context = context },
    });
    made.deinit();
    var result = fx.run(&env, described);
    described.deinit();
    defer result.deinit();
    try std.testing.expectEqual(@as(i64, 42), result.success);
    try std.testing.expectEqual(@as(usize, 1), counts.closed);
    try std.testing.expectEqual(@as(usize, 0), live);
}

const ReturnReader = struct {
    fn run(registry: anytype, inputs: anytype, _: *const Env) @TypeOf(registry.openWith(inputs.scope, operation, inputs.context, inputs.request)) {
        return registry.openWith(inputs.scope, operation, inputs.context, inputs.request);
    }
};

test "RPC-02 LIFE-11 DECL-04 returned registry reader retains configuration and value request storage after registry description and reader scope close" {
    var live: usize = 0;
    var counts: Counts = .{};
    var bytes_seen: ?[]const u8 = null;
    var context_seen: ?*const fx.RpcContext = null;
    var request_seen: ?*const i64 = null;
    var value: i64 = 0;
    var context = fx.RpcContext.empty;
    context.correlation = 43;
    var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
    defer threads.deinit();
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .live = &live, .counts = &counts, .bytes_seen = &bytes_seen, .context_seen = &context_seen, .request_seen = &request_seen, .value = &value };
    var scope: fx.Scope(i64) = .init(&env);
    var described = fx.dsl.withRpcRegistry(&env, .{
        .definition = fx.dsl.declareMetadata(.{ .use = ReturnReader.run }),
        .inputs = .{ .bindings = .{.{ .definition = fx.dsl.declareMetadata(.{ .operation = operation, .handler = Callback.handle }), .inputs = .{ .owner = try Owned.init(std.testing.allocator, &live), .offset = @as(i64, 25) } }}, .request = @as(i64, @intCast(counts.constructed + 17)), .context = context, .scope = &scope },
    });
    var opened = fx.run(&env, described);
    described.deinit();
    try std.testing.expectEqual(@as(usize, 1), live);
    var chunk = fx.run(&env, opened.success.pull.borrow());
    try std.testing.expectEqual(@as(i64, 42), chunk.success.items.values[0].chunk);
    chunk.deinit();
    var closed = fx.run(&env, fx.Effect(void, i64, Env).closeScope(&scope));
    closed.deinit();
    scope.deinit();
    try std.testing.expectEqual(@as(usize, 1), counts.closed);
    try std.testing.expectEqual(@as(usize, 1), live);
    try std.testing.expectEqual(@as(i64, 17), request_seen.?.*);
    try std.testing.expectEqual(@as(u64, 43), context_seen.?.correlation);
    var copy = try opened.success.clone();
    opened.deinit();
    try std.testing.expectEqual(@as(usize, 1), live);
    try std.testing.expectEqual(@as(i64, 17), request_seen.?.*);
    copy.deinit();
    try std.testing.expectEqual(@as(usize, 0), live);
}

test "RPC-02 LIFE-07 DECL-04 every runtime stream registry allocation failure releases only initialized call scope reader and capture owners" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            var live: usize = 0;
            defer std.debug.assert(live == 0);
            var counts: Counts = .{};
            var bytes_seen: ?[]const u8 = null;
            var context_seen: ?*const fx.RpcContext = null;
            var value: i64 = 0;
            var context = fx.RpcContext.empty;
            context.correlation = 43;
            var threads: Threads = .{ .allocator = allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
            defer threads.deinit();
            const env: Env = .{ .allocator = allocator, .async_engine = threads.executor(), .live = &live, .counts = &counts, .bytes_seen = &bytes_seen, .context_seen = &context_seen, .value = &value };
            var scope: fx.Scope(i64) = .init(&env);
            var described = fx.dsl.withRpcRegistry(&env, .{
                .definition = fx.dsl.declareMetadata(.{ .use = ReturnReader.run }),
                .inputs = .{ .bindings = .{.{ .definition = fx.dsl.declareMetadata(.{ .operation = operation, .handler = Callback.handle }), .inputs = .{ .owner = try Owned.init(allocator, &live), .offset = @as(i64, 25) } }}, .request = @as(i64, @intCast(counts.constructed + 17)), .context = context, .scope = &scope },
            });
            defer described.deinit();
            var opened = fx.run(&env, described);
            defer opened.deinit();
            var closed = fx.run(&env, fx.Effect(void, i64, Env).closeScope(&scope));
            closed.deinit();
            scope.deinit();
            try std.testing.expectEqual(counts.readers, counts.closed);
            if (opened == .die and opened.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expect(opened == .success);
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

test "RPC-02 LIFE-11 reader capture retention preserves the original projected execution allocator while keeping capture storage separate" {
    const R = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io };
    const S = fx.Stream(i64, void, R);
    const P = fx.Effect(fx.Chunk(i64), void, R);
    const It = struct { env: *const R, value: i64 = 42 };
    const NativeReader = struct {
        fn acquire(env: *const R) fx.Exit(*It, void) {
            const it = env.allocator.create(It) catch return .{ .die = fx.Defect.out_of_memory };
            it.* = .{ .env = env };
            return .{ .success = it };
        }
        fn pull(it: *It) P {
            var described = P.fromBorrowed(It, it, struct {
                fn read(current: *const It, _: *const R) fx.Exit(fx.Chunk(i64), void) {
                    return .{ .success = fx.Chunk(i64).borrowed(@as(*const [1]i64, &current.value)) };
                }
            }.read);
            return described.provideOwned(it.env, R, it.env);
        }
        fn release(it: *It) fx.Effect(void, void, R) {
            return fx.Effect(void, void, R).fromBorrowed(It, it, struct {
                fn close(current: *const It, env: *const R) fx.Exit(void, void) {
                    env.allocator.destroy(@constCast(current));
                    return .{ .success = {} };
                }
            }.close);
        }
    };
    const Extra = struct {
        pub const effect_owned_payload = true;
        live: *usize,
        pub fn deinit(self: *@This()) void {
            self.live.* -= 1;
        }
    };
    var live: usize = 1;
    const good: R = .{ .allocator = std.testing.allocator };
    var failing = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 0 });
    const caller: R = .{ .allocator = failing.allocator() };
    var scope: fx.Scope(void) = .init(&good);
    const stream = comptime S.fromPull(fx.Effect(*It, void, R).fromOutcome(NativeReader.acquire), NativeReader.pull, NativeReader.release);
    var opening = stream.open(&scope);
    var opened = fx.run(&good, opening);
    opening.deinit();
    var retained = try fx.StreamReader(i64, void, R).retainCaptureAllocated(std.testing.allocator, opened.success, Extra{ .live = &live });
    opened = .interrupted; // ownership moved to the retained reader
    var value = fx.run(&caller, retained.pull.borrow());
    try std.testing.expectEqual(@as(i64, 42), value.success.items.values[0]);
    value.deinit();
    retained.deinit();
    try std.testing.expectEqual(@as(usize, 0), live);
    var closed = fx.run(&good, fx.Effect(void, void, R).closeScope(&scope));
    closed.deinit();
    scope.deinit();
}
