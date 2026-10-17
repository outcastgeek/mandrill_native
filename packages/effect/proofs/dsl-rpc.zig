const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, calls: *usize, bias: i64 };
const integer = fx.JsonCodec.signed_integer;
const nothing = fx.JsonCodec.null_value;
const Reply = fx.Effect(i64, void, Env);
const Callbacks = struct {
    fn echo(input: anytype, env: anytype) Reply {
        env.calls.* += 1;
        return Reply.succeed(input.* + env.bias);
    }
    fn typedEcho(input: *const i64, env: *const Env) Reply {
        return echo(input, env);
    }
    fn contextual(context: fx.RpcContext, input: *const i64, env: *const Env) Reply {
        env.calls.* += 1;
        return Reply.succeed(input.* + @as(i64, @intCast(context.correlation.?)) + @as(i64, @intCast(context.metadata[0].value.len)));
    }
};

test "RPC-01 INF-08 ROLE-02 DECL-01 inferred unary records preserve staged definitions role ordering and exact Effect dispatch" {
    const first = comptime fx.dsl.rpcUnary(.{ .name = "dsl.echo", .version = 3, .request = integer, .success = integer, .failure = nothing, .handler = Callbacks.echo });
    const second = comptime fx.dsl.rpcUnary(.{ .handler = Callbacks.echo, .success = integer, .version = 3, .failure = nothing, .name = "dsl.echo", .request = integer });
    const staged = fx.RpcProtocolBuilder(void, Env).unary("dsl.echo", 3).request(integer).success(integer).failure(nothing);
    comptime {
        if (@TypeOf(first) != @TypeOf(second) or @TypeOf(first.operation) != @TypeOf(staged.definition)) @compileError("named declarations must preserve exact canonical staged types");
    }
    const Registry = fx.dsl.rpcRegistry(.{first});
    var calls: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .calls = &calls, .bias = 7 };
    const request: i64 = 11;
    const described = Registry.invoke(second.operation, &request);
    comptime {
        if (@TypeOf(described) != Reply) @compileError("RPC evaluation must be exact Effect");
    }
    try std.testing.expectEqual(@as(usize, 0), calls);
    var result = fx.run(&env, described);
    defer result.deinit();
    try std.testing.expectEqual(@as(i64, 18), result.success);
    try std.testing.expectEqual(@as(usize, 1), calls);
    try std.testing.expect(Registry.resolve("dsl.echo", 2) == .err);
    try std.testing.expect(Registry.resolve("unknown", 3) == .err);
}

test "RPC-01 DECL-04 INF-08 named definition pairing preserves context and mixed registration with original handlers" {
    const operation = comptime fx.dsl.rpcUnaryDefinition(.{ .name = "dsl.context", .version = 1, .request = integer, .success = integer, .failure = nothing });
    const handler = comptime fx.dsl.rpcHandled(.{ .handler = Callbacks.contextual, .definition = operation });
    const old = comptime fx.RpcProtocolBuilder(void, Env).unary("legacy.echo", 1).request(integer).success(integer).failure(nothing).handledBy(Callbacks.typedEcho);
    const Registry = fx.dsl.rpcRegistry(.{ old, handler });
    var calls: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .calls = &calls, .bias = 0 };
    const request: i64 = 11;
    const context: fx.RpcContext = .{ .correlation = 5, .metadata = &.{.{ .key = "trace", .value = "abc" }} };
    var described = Registry.invokeWith(operation, &context, &request, &env);
    defer described.deinit();
    try std.testing.expectEqual(@as(usize, 0), calls);
    var result = fx.run(&env, described);
    defer result.deinit();
    try std.testing.expectEqual(@as(i64, 19), result.success);
    try std.testing.expectEqual(@as(usize, 1), calls);
}

const stream_operation = fx.dsl.rpcStreamDefinition(.{ .name = "dsl.stream", .version = 2, .request = integer, .chunk = integer, .terminal = nothing, .failure = nothing });
const Event = @TypeOf(stream_operation).Event;
const Stream = fx.Stream(Event, void, Env);
const Iterator = struct { allocator: std.mem.Allocator, index: usize = 0 };
const Streaming = struct {
    fn open(env: *const Env) fx.Exit(*Iterator, void) {
        const iterator = env.allocator.create(Iterator) catch return .{ .die = fx.Defect.out_of_memory };
        iterator.* = .{ .allocator = env.allocator };
        env.calls.* += 1;
        return .{ .success = iterator };
    }
    fn next(iterator: *Iterator, env: *const Env) fx.Exit(?Event, void) {
        defer iterator.index += 1;
        return .{ .success = switch (iterator.index) {
            0 => .{ .chunk = env.bias },
            1 => .{ .terminal = {} },
            else => null,
        } };
    }
    fn close(iterator: *Iterator) fx.Effect(void, void, Env) {
        return fx.Effect(void, void, Env).fromBorrowed(Iterator, iterator, struct {
            fn run(value: *const Iterator, _: *const Env) fx.Exit(void, void) {
                value.allocator.destroy(@constCast(value));
                return .{ .success = {} };
            }
        }.run);
    }
    fn handle(_: *const i64, _: *const Env) Stream {
        return Stream.fromIterable(Iterator, open, next, close);
    }
};

test "RPC-02 INF-08 DECL-04 streaming definition supplies its own event type and registry retains chunk terminal distinction" {
    const handler = comptime fx.dsl.rpcHandled(.{ .definition = stream_operation, .handler = Streaming.handle });
    const combined = comptime fx.dsl.rpcStream(.{ .name = "dsl.combined-stream", .version = 2, .request = integer, .chunk = integer, .terminal = nothing, .failure = nothing, .handler = Streaming.handle });
    const Registry = fx.dsl.rpcRegistry(.{ handler, combined });
    try std.testing.expect(Registry.resolve("dsl.stream", 2) == .ok);
    var calls: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .calls = &calls, .bias = 7 };
    const request: i64 = 11;
    // This callback constructs a stream; opening/consumption remains lazy.
    _ = handler.open(&request, &env);
    try std.testing.expectEqual(@as(usize, 0), calls);
    const source = comptime handler.open(&@as(i64, 0), &@as(Env, undefined));
    const collect = comptime source.runCollect(4);
    comptime {
        if (@TypeOf(collect) != fx.Effect(fx.OwnedSlice(Event), void, Env)) @compileError("RPC stream consumption must compose as exact Effect");
    }
    var result = fx.run(&env, collect);
    defer result.deinit();
    try std.testing.expectEqual(@as(usize, 2), result.success.items.len);
    try std.testing.expectEqual(@as(i64, 7), result.success.items[0].chunk);
    try std.testing.expect(result.success.items[1] == .terminal);
    try std.testing.expectEqual(@as(usize, 1), calls);
}

test "RPC-01 INF-14 DECL-01 schema type roles and generic idempotency selectors retain the existing primary key contract" {
    const RequestSchema = fx.dsl.schemaRecord(Env, .{ .fields = .{
        .key = .{ .codec = fx.JsonCodec.borrowed_string, .presence = .required },
        .value = .{ .codec = integer, .presence = .required },
    }, .policy = .{ .unknown_fields = .reject, .duplicate_fields = .reject, .issues = .first } });
    const Roles = struct {
        fn key(input: anytype) []const u8 {
            return input.key;
        }
        fn handle(input: anytype, env: *const Env) Reply {
            return Reply.succeed(input.value + env.bias);
        }
    };
    const handler = comptime fx.dsl.rpcUnary(.{ .name = "dsl.key", .version = 1, .request = RequestSchema, .success = integer, .failure = nothing, .idempotency = Roles.key, .handler = Roles.handle });
    var calls: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .calls = &calls, .bias = 7 };
    var encoded_key = fx.PrimaryKey.encodeDirect(&env, "rpc-request", &.{.{ .text = "request-11" }});
    defer encoded_key.deinit();
    try std.testing.expect(encoded_key == .success);
    const Request = @TypeOf(handler.operation).Request;
    var request: Request = .{ .key = encoded_key.success.items, .value = 11 };
    const selected = handler.operation.key(&request);
    try std.testing.expect(selected == .ok);
    try std.testing.expectEqualStrings(encoded_key.success.items, selected.ok.?);
    var reply = fx.run(&env, handler.invoke(&request));
    defer reply.deinit();
    try std.testing.expectEqual(@as(i64, 18), reply.success);
    request.key = "invalid key";
    try std.testing.expect(handler.operation.key(&request) == .err);
}

test "INF-03 INF-08 INF-12 RPC-01 codec environment and request fields specialize dependent generic combined and paired handlers lazily" {
    const R = struct { allocator: std.mem.Allocator, offset: i64, calls: *usize };
    const Request = fx.dsl.schemaRecord(R, .{ .fields = .{ .value = .{ .codec = integer, .presence = .required } }, .policy = .{ .unknown_fields = .reject, .duplicate_fields = .reject, .issues = .first } });
    const Generic = struct {
        fn handle(input: anytype, env: anytype) fx.Effect(@TypeOf(input.value), void, R) {
            env.calls.* += 1;
            return fx.Effect(@TypeOf(input.value), void, R).succeed(input.value + env.offset);
        }
    };
    const combined = comptime fx.dsl.rpcUnary(.{ .name = "generic.combined", .version = 1, .request = Request, .success = integer, .failure = nothing, .handler = Generic.handle });
    const definition = comptime fx.dsl.rpcUnaryDefinition(.{ .name = "generic.paired", .version = 1, .request = Request, .success = integer, .failure = nothing });
    const paired = comptime fx.dsl.rpcHandled(.{ .definition = definition, .handler = Generic.handle });
    const Registry = fx.dsl.rpcRegistry(.{ combined, paired });
    var calls: usize = 0;
    const env: R = .{ .allocator = std.testing.allocator, .offset = 7, .calls = &calls };
    const request: Request.Decoded = .{ .value = 11 };
    const first = Registry.invoke(combined.operation, &request);
    const second = Registry.invoke(paired.operation, &request);
    comptime {
        if (@TypeOf(first) != fx.Effect(i64, void, R) or @TypeOf(second) != fx.Effect(i64, void, R)) @compileError("all available codec environment evidence must specialize the exact handler Effect");
    }
    try std.testing.expectEqual(@as(usize, 0), calls);
    var first_result = fx.run(&env, first);
    defer first_result.deinit();
    var second_result = fx.run(&env, second);
    defer second_result.deinit();
    try std.testing.expectEqual(@as(i64, 18), first_result.success);
    try std.testing.expectEqual(@as(i64, 18), second_result.success);
    try std.testing.expectEqual(@as(usize, 2), calls);
}

test "INF-03 INF-13 RPC-01 one environment anchor resolves genuinely independent codec services without rewriting their evidence" {
    const R = struct { allocator: std.mem.Allocator, offset: i64, calls: *usize };
    const W = struct { allocator: std.mem.Allocator, distinct: bool };
    const policy = .{ .unknown_fields = .reject, .duplicate_fields = .reject, .issues = .first };
    const Request = fx.dsl.schemaRecord(R, .{ .fields = .{ .value = .{ .codec = integer, .presence = .required } }, .policy = policy });
    const Success = fx.dsl.schemaRecord(W, .{ .fields = .{ .value = .{ .codec = integer, .presence = .required } }, .policy = policy });
    const Generic = struct {
        fn handle(input: anytype, env: anytype) fx.Effect(Success.Decoded, void, @typeInfo(@TypeOf(env)).pointer.child) {
            env.calls.* += 1;
            return fx.Effect(Success.Decoded, void, @typeInfo(@TypeOf(env)).pointer.child).succeed(.{ .value = input.value + env.offset });
        }
    };
    const handler = comptime fx.dsl.rpcUnary(.{ .name = "independent.codecs", .version = 1, .request = Request, .success = Success, .failure = nothing, .handler = Generic.handle, .environment = R });
    comptime {
        if (@TypeOf(handler).Environment != R or @TypeOf(handler.operation.success).Environment != W or @TypeOf(handler.operation.request).Environment != R) @compileError("handler and codec service requirements must remain independently typed");
    }
    var calls: usize = 0;
    const env: R = .{ .allocator = std.testing.allocator, .offset = 7, .calls = &calls };
    const request: Request.Decoded = .{ .value = 11 };
    const operation = handler.invoke(&request);
    try std.testing.expectEqual(@as(usize, 0), calls);
    var result = fx.run(&env, operation);
    defer result.deinit();
    try std.testing.expectEqual(@as(i64, 18), result.success.value);
    try std.testing.expectEqual(@as(usize, 1), calls);
}

test "INF-03 CODEC-03 RPC-01 generic handler inference uses request decoding evidence and preserves its independent encoding environment" {
    const R = struct { allocator: std.mem.Allocator, offset: i64 };
    const W = struct { allocator: std.mem.Allocator, distinct: bool };
    const Value = struct { value: i64 };
    const Issue = fx.SchemaIssue.Issue;
    const Selected = struct {
        fn decode(input: *const std.json.Value) fx.Effect(Value, Issue, R) {
            return fx.Effect(Value, Issue, R).succeed(.{ .value = input.integer });
        }
        fn encode(input: *const Value) fx.Effect(std.json.Value, Issue, W) {
            return fx.Effect(std.json.Value, Issue, W).succeed(.{ .integer = input.value });
        }
        fn handle(input: anytype, env: anytype) fx.Effect(@TypeOf(input.value), void, R) {
            return fx.Effect(@TypeOf(input.value), void, R).succeed(input.value + env.offset);
        }
    };
    const request_codec = comptime fx.dsl.codec(.{ .kind = .contextual, .decode = Selected.decode, .encode = Selected.encode });
    const handler = comptime fx.dsl.rpcUnary(.{ .name = "directional.codec", .version = 1, .request = request_codec, .success = integer, .failure = nothing, .handler = Selected.handle });
    comptime {
        if (@TypeOf(handler).Environment != R or @TypeOf(handler.operation.request).EncodingEnvironment != W) @compileError("request encoding cannot be conflated with decoding/handler service inference");
    }
    const env: R = .{ .allocator = std.testing.allocator, .offset = 7 };
    const request: Value = .{ .value = 11 };
    var result = fx.run(&env, handler.invoke(&request));
    defer result.deinit();
    try std.testing.expectEqual(@as(i64, 18), result.success);
}
