const std = @import("std");
const fx = @import("effect");
const gates = @import("support/dsl_gate.zig");
const Threads = @import("support/thread_executor.zig").ThreadExecutor;
const Gate = gates.Gate([]const u8, []const u8);
const Construction = struct { allocator: std.mem.Allocator };
const rejected: fx.Executor = .{
    .userdata = null,
    .capabilities = .{ .independent = false, .cpu = false, .blocking = false },
    .submit_fn = struct {
        fn submit(_: ?*anyopaque, _: fx.ExecutionKind, _: fx.ExecutionJob) fx.Executor.Submission {
            return .{ .rejected = fx.Defect.init("fixture admission rejected") };
        }
    }.submit,
};
const Env = struct {
    allocator: std.mem.Allocator,
    io: std.Io = std.testing.io,
    async_engine: fx.Executor = rejected,
    gate: ?*Gate = null,
    calls: *usize,
};
const Body = fx.Effect([]const u8, []const u8, Env);
const Owned = struct {
    pub const effect_owned_payload = true;
    allocator: std.mem.Allocator,
    bytes: []u8,
    live: *std.atomic.Value(usize),
    fn init(allocator: std.mem.Allocator, live: *std.atomic.Value(usize)) !@This() {
        const bytes = try allocator.dupe(u8, "runtime RPC configuration");
        _ = live.fetchAdd(1, .acq_rel);
        return .{ .allocator = allocator, .bytes = bytes, .live = live };
    }
    pub fn deinit(self: *@This()) void {
        self.allocator.free(self.bytes);
        _ = self.live.fetchSub(1, .acq_rel);
    }
};
const operation = fx.dsl.rpcUnaryDefinition(.{ .name = "captured.rpc", .version = 1, .request = fx.JsonCodec.signed_integer, .success = fx.JsonCodec.borrowed_string, .failure = fx.JsonCodec.borrowed_string });
const Callbacks = struct {
    fn handle(configuration: anytype, input: anytype, env: *const Env) Body {
        env.calls.* += 1;
        std.debug.assert(configuration.threshold == 37);
        const bytes = configuration.owned.bytes;
        if (env.gate) |gate| {
            switch (gate.result) {
                .success => gate.result.success = bytes,
                .fail => gate.result.fail = bytes,
                .finalization_failed => {
                    gate.result.finalization_failed.primary.failed = bytes;
                    gate.result.finalization_failed.first.release_failure = bytes;
                },
                else => {},
            }
            return Body.fromAsync(struct {
                fn start(current: *const Env) fx.Async([]const u8, []const u8).Start {
                    return current.gate.?.start();
                }
            }.start);
        }
        return if (input.* == 0) Body.fail(bytes) else Body.succeed(bytes);
    }
    fn contextual(configuration: anytype, context: *const fx.RpcContext, input: anytype, env: *const Env) Body {
        configuration.context_seen.?.* = context;
        return handle(configuration, input, env);
    }
};
fn make(allocator: std.mem.Allocator, configuration: anytype, comptime callback: anytype) @TypeOf(fx.dsl.makeRpcHandler(&@as(Construction, undefined), .{ .definition = fx.dsl.declareMetadata(.{ .operation = operation, .handler = callback }), .inputs = configuration })) {
    const env: Construction = .{ .allocator = allocator };
    return fx.dsl.makeRpcHandler(&env, .{ .definition = fx.dsl.declareMetadata(.{ .operation = operation, .handler = callback }), .inputs = configuration });
}

test "RPC-01 DECL-03 CAP-02 CAP-04 generated unary handler captures noncloneable anonymous configuration and returned outcomes outlive all descriptions" {
    var live: std.atomic.Value(usize) = .init(0);
    var calls: usize = 0;
    const construction: Construction = .{ .allocator = std.testing.allocator };
    const env: Env = .{ .allocator = std.testing.allocator, .calls = &calls };
    var described = make(std.testing.allocator, .{ .owned = try Owned.init(std.testing.allocator, &live), .threshold = @as(usize, 37), .context_seen = @as(?**const fx.RpcContext, null) }, Callbacks.handle);
    try std.testing.expectEqual(@as(usize, 0), calls);
    var made = fx.run(&construction, described);
    const H = @TypeOf(made).Success;
    comptime {
        if (@TypeOf(described) != fx.Effect(H, H.Error, Construction)) @compileError("captured handler creation must remain exact Effect");
        if (H.Environment != Env) @compileError("handler R must be independent of construction services");
    }
    const input: i64 = 11;
    var invocation = made.success.invoke(&input);
    comptime if (@TypeOf(invocation) != fx.Effect(H.Success, H.Error, Env)) @compileError("captured invocation must compose as exact Effect");
    var result = fx.run(&env, invocation);
    invocation.deinit();
    const bad: i64 = 0;
    var failure = made.success.invoke(&bad);
    var failed = fx.run(&env, failure);
    failure.deinit();
    made.deinit();
    described.deinit();
    try std.testing.expectEqualStrings("runtime RPC configuration", result.success.value);
    result.deinit();
    try std.testing.expectEqualStrings("runtime RPC configuration", failed.fail.value);
    failed.deinit();
    try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
    try std.testing.expectEqual(@as(usize, 2), calls);
}

test "RPC-01 LIFE-11 LIFE-12 captured unary invocation retains configuration across every pending outcome and cancellation" {
    inline for (.{ .success, .fail, .die, .interrupted, .finalization_failed, .cancel }) |mode| {
        var live: std.atomic.Value(usize) = .init(0);
        var calls: usize = 0;
        var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
        defer threads.deinit();
        var gate: Gate = .{ .result = switch (mode) {
            .success, .cancel => .{ .success = "" },
            .fail => .{ .fail = "" },
            .die => .{ .die = fx.Defect.init("RPC handler defect") },
            .interrupted => .interrupted,
            .finalization_failed => .{ .finalization_failed = .{ .primary = .{ .failed = "" }, .first = .{ .release_failure = "" } } },
            else => unreachable,
        } };
        const construction: Construction = .{ .allocator = std.testing.allocator };
        const env: Env = .{ .allocator = std.testing.allocator, .calls = &calls, .async_engine = threads.executor(), .gate = &gate };
        var described = make(std.testing.allocator, .{ .owned = try Owned.init(std.testing.allocator, &live), .threshold = @as(usize, 37), .context_seen = @as(?**const fx.RpcContext, null) }, Callbacks.handle);
        var made = fx.run(&construction, described);
        described.deinit();
        const input: i64 = 11;
        var invocation = made.success.invoke(&input);
        var pending = fx.runAsync(&env, invocation);
        invocation.deinit();
        made.deinit();
        gate.started.waitUncancelable(env.io);
        try std.testing.expectEqual(@as(usize, 1), live.load(.acquire));
        if (mode == .cancel) pending.request() else gate.proceed.set(env.io);
        var result = pending.join();
        pending.deinit();
        try std.testing.expect(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
        switch (mode) {
            .success => try std.testing.expectEqualStrings("runtime RPC configuration", result.success.value),
            .fail => try std.testing.expectEqualStrings("runtime RPC configuration", result.fail.value),
            .finalization_failed => {
                try std.testing.expectEqualStrings("runtime RPC configuration", result.finalization_failed.primary.failed.value);
                try std.testing.expectEqualStrings("runtime RPC configuration", result.finalization_failed.first.release_failure.value);
            },
            .die => try std.testing.expect(result == .die),
            .interrupted, .cancel => try std.testing.expect(result == .interrupted),
            else => unreachable,
        }
        result.deinit();
        try std.testing.expectEqual(@as(usize, 1), calls);
        try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
    }
}

test "RPC-01 LIFE-07 every captured handler construction invocation and failure allocation unwinds initialized owners" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator, input: i64) !void {
            var live: std.atomic.Value(usize) = .init(0);
            defer std.debug.assert(live.load(.acquire) == 0);
            var calls: usize = 0;
            const construction: Construction = .{ .allocator = allocator };
            const env: Env = .{ .allocator = allocator, .calls = &calls };
            var described = make(allocator, .{ .owned = try Owned.init(allocator, &live), .threshold = @as(usize, 37), .context_seen = @as(?**const fx.RpcContext, null) }, Callbacks.handle);
            defer described.deinit();
            var made = fx.run(&construction, described);
            defer made.deinit();
            if (made == .die and made.die.kind == .out_of_memory) return error.OutOfMemory;
            var invocation = made.success.invoke(&input);
            defer invocation.deinit();
            var result = fx.run(&env, invocation);
            defer result.deinit();
            if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
            if (input == 0) try std.testing.expectEqualStrings("runtime RPC configuration", result.fail.value) else try std.testing.expectEqualStrings("runtime RPC configuration", result.success.value);
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{@as(i64, 11)});
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{@as(i64, 0)});
}

test "RPC-01 CAP-04 pending pointer context stays in generated storage after handler and description release" {
    var live: std.atomic.Value(usize) = .init(0);
    var calls: usize = 0;
    var seen: *const fx.RpcContext = undefined;
    var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
    defer threads.deinit();
    var gate: Gate = .{ .result = .{ .success = "" } };
    const construction: Construction = .{ .allocator = std.testing.allocator };
    const env: Env = .{ .allocator = std.testing.allocator, .calls = &calls, .async_engine = threads.executor(), .gate = &gate };
    var described = make(std.testing.allocator, .{ .owned = try Owned.init(std.testing.allocator, &live), .threshold = @as(usize, 37), .context_seen = @as(?**const fx.RpcContext, &seen) }, Callbacks.contextual);
    var made = fx.run(&construction, described);
    described.deinit();
    const input: i64 = 11;
    var invocation = made.success.invokeWith(.{ .correlation = 19, .metadata = &.{.{ .key = "trace", .value = "held context" }} }, &input);
    var pending = fx.runAsync(&env, invocation);
    invocation.deinit();
    made.deinit();
    gate.started.waitUncancelable(env.io);
    try std.testing.expectEqual(@as(u128, 19), seen.correlation.?);
    try std.testing.expectEqualStrings("held context", seen.metadata[0].value);
    gate.proceed.set(env.io);
    var result = pending.join();
    pending.deinit();
    try std.testing.expectEqualStrings("runtime RPC configuration", result.success.value);
    result.deinit();
    try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
}

test "RPC-01 DECL-03 DECL-04 runtime registry consumes declaration records once and owns results after the complete scope ends" {
    const Use = struct {
        fn run(registry: anytype, inputs: anytype, _: *const Env) @TypeOf(registry.invoke(operation, inputs.request)) {
            std.debug.assert(registry.resolve("captured.rpc", 1) == .ok);
            std.debug.assert(registry.resolve("unknown", 1) == .err);
            return registry.invoke(operation, inputs.request);
        }
    };
    var live: std.atomic.Value(usize) = .init(0);
    var calls: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .calls = &calls };
    var described = fx.dsl.withRpcRegistry(&env, .{
        .definition = fx.dsl.declareMetadata(.{ .use = Use.run }),
        .inputs = .{ .bindings = .{.{
            .definition = fx.dsl.declareMetadata(.{ .operation = operation, .handler = Callbacks.handle }),
            .inputs = .{ .owned = try Owned.init(std.testing.allocator, &live), .threshold = @as(usize, 37) },
        }}, .request = @as(i64, 11) },
    });
    try std.testing.expectEqual(@as(usize, 0), calls);
    var result = fx.run(&env, described);
    described.deinit();
    try std.testing.expectEqualStrings("runtime RPC configuration", result.success.value);
    try std.testing.expectEqual(@as(usize, 1), live.load(.acquire));
    result.deinit();
    try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
    try std.testing.expectEqual(@as(usize, 1), calls);
}

test "RPC-01 DECL-04 runtime registry admits retained handlers alongside original static bindings without changing protocol identity" {
    const Legacy = struct {
        fn handle(_: *const i64, _: *const Env) Body {
            return Body.succeed("static RPC reply");
        }
    };
    const old = comptime fx.dsl.rpcUnary(.{ .name = "static.rpc", .version = 1, .request = fx.JsonCodec.signed_integer, .success = fx.JsonCodec.borrowed_string, .failure = fx.JsonCodec.borrowed_string, .handler = Legacy.handle });
    const Use = struct {
        fn run(registry: anytype, inputs: anytype, _: *const Env) @TypeOf(registry.invoke(operation, inputs.request)) {
            std.debug.assert(registry.resolve("static.rpc", 1).ok == 1);
            return registry.invoke(operation, inputs.request);
        }
        fn useStatic(registry: anytype, inputs: anytype, _: *const Env) @TypeOf(registry.invoke(old.operation, inputs.request)) {
            return registry.invoke(old.operation, inputs.request);
        }
    };
    var live: std.atomic.Value(usize) = .init(0);
    var calls: usize = 0;
    const construction: Construction = .{ .allocator = std.testing.allocator };
    const env: Env = .{ .allocator = std.testing.allocator, .calls = &calls };
    var creation = make(std.testing.allocator, .{ .owned = try Owned.init(std.testing.allocator, &live), .threshold = @as(usize, 37) }, Callbacks.handle);
    var made = fx.run(&construction, creation);
    creation.deinit();
    var described = fx.dsl.withRpcRegistry(&env, .{
        .definition = fx.dsl.declareMetadata(.{ .use = Use.run }),
        .inputs = .{ .bindings = .{ made.success.retain(), fx.dsl.rpcStatic(old) }, .request = @as(i64, 11) },
    });
    made.deinit();
    var result = fx.run(&env, described);
    described.deinit();
    try std.testing.expectEqualStrings("runtime RPC configuration", result.success.value);
    result.deinit();
    try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
    var static_description = fx.dsl.withRpcRegistry(&env, .{
        .definition = fx.dsl.declareMetadata(.{ .use = Use.useStatic }),
        .inputs = .{ .bindings = .{ .{
            .definition = fx.dsl.declareMetadata(.{ .operation = operation, .handler = Callbacks.handle }),
            .inputs = .{ .owned = try Owned.init(std.testing.allocator, &live), .threshold = @as(usize, 37) },
        }, fx.dsl.rpcStatic(old) }, .request = @as(i64, 11) },
    });
    var static_result = fx.run(&env, static_description);
    static_description.deinit();
    try std.testing.expectEqualStrings("static RPC reply", static_result.success.value);
    static_result.deinit();
    try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
}

test "RPC-01 DECL-03 runtime registry materializes empty and comptime configuration views without runtime tuple pointers" {
    const Empty = struct {
        fn handle(_: anytype, _: *const i64, _: *const Env) Body {
            return Body.succeed("empty configuration reply");
        }
    };
    const Constant = struct {
        fn handle(configuration: anytype, _: *const i64, _: *const Env) Body {
            return Body.succeed(configuration.reply);
        }
    };
    const Use = struct {
        fn run(registry: anytype, inputs: anytype, _: *const Env) @TypeOf(registry.invoke(operation, inputs.request)) {
            return registry.invoke(operation, inputs.request);
        }
    };
    var calls: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .calls = &calls };
    inline for (.{ Empty, Constant }) |C| {
        var described = fx.dsl.withRpcRegistry(&env, .{
            .definition = fx.dsl.declareMetadata(.{ .use = Use.run }),
            .inputs = .{ .bindings = .{.{
                .definition = fx.dsl.declareMetadata(.{ .operation = operation, .handler = C.handle }),
                .inputs = if (C == Empty) .{} else .{ .reply = @as([]const u8, "constant configuration reply") },
            }}, .request = @as(i64, 11) },
        });
        var result = fx.run(&env, described);
        described.deinit();
        try std.testing.expectEqualStrings(if (C == Empty) "empty configuration reply" else "constant configuration reply", result.success.value);
        result.deinit();
    }
}

test "RPC-01 LIFE-11 LIFE-12 runtime registry keeps declaration configuration and invocation data until pending work and returned outcomes finish" {
    const Use = struct {
        fn run(registry: anytype, inputs: anytype, _: *const Env) @TypeOf(registry.invoke(operation, inputs.request)) {
            return registry.invoke(operation, inputs.request);
        }
    };
    inline for (.{ .success, .fail, .die, .interrupted, .finalization_failed, .cancel }) |mode| {
        var live: std.atomic.Value(usize) = .init(0);
        var calls: usize = 0;
        var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
        defer threads.deinit();
        var gate: Gate = .{ .result = switch (mode) {
            .success, .cancel => .{ .success = "" },
            .fail => .{ .fail = "" },
            .die => .{ .die = fx.Defect.init("registry handler defect") },
            .interrupted => .interrupted,
            .finalization_failed => .{ .finalization_failed = .{ .primary = .{ .failed = "" }, .first = .{ .release_failure = "" } } },
            else => unreachable,
        } };
        const env: Env = .{ .allocator = std.testing.allocator, .calls = &calls, .async_engine = threads.executor(), .gate = &gate };
        var described = fx.dsl.withRpcRegistry(&env, .{
            .definition = fx.dsl.declareMetadata(.{ .use = Use.run }),
            .inputs = .{ .bindings = .{.{
                .definition = fx.dsl.declareMetadata(.{ .operation = operation, .handler = Callbacks.handle }),
                .inputs = .{ .owned = try Owned.init(std.testing.allocator, &live), .threshold = @as(usize, 37) },
            }}, .request = @as(i64, 11) },
        });
        var pending = fx.runAsync(&env, described);
        described.deinit();
        gate.started.waitUncancelable(env.io);
        try std.testing.expectEqual(@as(usize, 1), live.load(.acquire));
        if (mode == .cancel) pending.request() else gate.proceed.set(env.io);
        var result = pending.join();
        pending.deinit();
        try std.testing.expect(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
        switch (mode) {
            .success => try std.testing.expectEqualStrings("runtime RPC configuration", result.success.value),
            .fail => try std.testing.expectEqualStrings("runtime RPC configuration", result.fail.value),
            .finalization_failed => {
                try std.testing.expectEqualStrings("runtime RPC configuration", result.finalization_failed.primary.failed.value);
                try std.testing.expectEqualStrings("runtime RPC configuration", result.finalization_failed.first.release_failure.value);
            },
            .die => try std.testing.expect(result == .die),
            .interrupted, .cancel => try std.testing.expect(result == .interrupted),
            else => unreachable,
        }
        result.deinit();
        try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
    }
}

test "RPC-01 LIFE-07 runtime registry allocation failures release only initialized storage and invocation owners" {
    const Use = struct {
        fn run(registry: anytype, inputs: anytype, _: *const Env) @TypeOf(registry.invoke(operation, inputs.request)) {
            return registry.invoke(operation, inputs.request);
        }
    };
    const Check = struct {
        fn run(allocator: std.mem.Allocator, input: i64) !void {
            var live: std.atomic.Value(usize) = .init(0);
            defer std.debug.assert(live.load(.acquire) == 0);
            var calls: usize = 0;
            const env: Env = .{ .allocator = allocator, .calls = &calls };
            var described = fx.dsl.withRpcRegistry(&env, .{
                .definition = fx.dsl.declareMetadata(.{ .use = Use.run }),
                .inputs = .{ .bindings = .{.{
                    .definition = fx.dsl.declareMetadata(.{ .operation = operation, .handler = Callbacks.handle }),
                    .inputs = .{ .owned = try Owned.init(allocator, &live), .threshold = @as(usize, 37) },
                }}, .request = input },
            });
            defer described.deinit();
            var result = fx.run(&env, described);
            defer result.deinit();
            if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
            if (input == 0) try std.testing.expectEqualStrings("runtime RPC configuration", result.fail.value) else try std.testing.expectEqualStrings("runtime RPC configuration", result.success.value);
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{@as(i64, 11)});
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{@as(i64, 0)});
}

test "RPC-01 LIFE-11 runtime registry value requests have stable owned addresses beyond callback scope and descriptor disposal" {
    const Issue = fx.SchemaIssue.Issue;
    const Bytes = struct {
        fn decode(input: *const std.json.Value) fx.Result([4]u8, Issue) {
            if (input.* != .string or input.string.len != 4) return .{ .err = Issue.pure(.decode, .invalid_value) };
            var result: [4]u8 = undefined;
            @memcpy(&result, input.string);
            return .{ .ok = result };
        }
        fn encode(input: *const [4]u8) fx.Result(std.json.Value, Issue) {
            return .{ .ok = .{ .string = input[0..] } };
        }
    };
    const request_codec = comptime fx.dsl.codec(.{ .kind = .pure, .decode = Bytes.decode, .encode = Bytes.encode });
    const bytes_operation = comptime fx.dsl.rpcUnaryDefinition(.{ .name = "request.bytes", .version = 1, .request = request_codec, .success = fx.JsonCodec.borrowed_string, .failure = fx.JsonCodec.borrowed_string });
    const Callback = struct {
        fn handle(_: anytype, request: *const [4]u8, _: *const Env) Body {
            return Body.succeed(request[0..]);
        }
        fn use(registry: anytype, inputs: anytype, _: *const Env) @TypeOf(registry.invoke(bytes_operation, inputs.request)) {
            return registry.invoke(bytes_operation, inputs.request);
        }
    };
    var calls: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .calls = &calls };
    var described = fx.dsl.withRpcRegistry(&env, .{
        .definition = fx.dsl.declareMetadata(.{ .use = Callback.use }),
        .inputs = .{ .bindings = .{.{
            .definition = fx.dsl.declareMetadata(.{ .operation = bytes_operation, .handler = Callback.handle }),
            .inputs = .{},
        }}, .request = @as([4]u8, .{ 'k', 'e', 'p', 't' }) },
    });
    var result = fx.run(&env, described);
    described.deinit();
    try std.testing.expectEqualStrings("kept", result.success.value);
    var copy = result.clone();
    try std.testing.expect(copy == .success);
    result.deinit();
    try std.testing.expectEqualStrings("kept", copy.success.value);
    copy.deinit();
}
