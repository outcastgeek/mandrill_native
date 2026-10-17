//! Separate-package coverage: every dependency below comes from the public module.
const std = @import("std");
const fx = @import("effect");
const Counts = struct { live: usize = 0, opened: usize = 0, closed: usize = 0, environments: usize = 0, releases: usize = 0, services: usize = 0, answer: i64 = 0 };
const Root = struct { allocator: std.mem.Allocator, io: std.Io, counts: *Counts };
const Service = struct {
    pub const effect_owned_payload = true;
    counts: *Counts,
    pub fn deinit(self: *@This()) void {
        std.debug.assert(self.counts.environments == 1 and self.counts.closed == self.counts.opened);
        self.counts.services += 1;
    }
};
const Env = struct {
    pub const effect_owned_payload = true;
    allocator: std.mem.Allocator,
    io: std.Io,
    service: *const Service,
    pub fn deinit(self: *@This()) void {
        std.debug.assert(self.service.counts.closed == self.service.counts.opened);
        std.debug.assert(self.service.counts.services == 0);
        self.service.counts.environments += 1;
    }
};
const Owner = struct {
    pub const effect_owned_payload = true;
    live: *usize,
    pub fn deinit(self: *@This()) void {
        self.live.* -= 1;
    }
};
const operation = fx.dsl.rpcStreamDefinition(.{ .name = "consumer.dsl.stream", .version = 1, .request = fx.JsonCodec.signed_integer, .chunk = fx.JsonCodec.signed_integer, .terminal = fx.JsonCodec.null_value, .failure = fx.JsonCodec.signed_integer });
const Event = @TypeOf(operation).Event;
const Stream = fx.Stream(Event, i64, Env);
const Iterator = struct { events: [2]Event, emitted: bool = false };
const Native = struct {
    fn create(env: *const Env) fx.Exit(*Iterator, i64) {
        std.debug.assert(env.service.counts.live == 1);
        const iterator = env.allocator.create(Iterator) catch return .{ .die = fx.Defect.out_of_memory };
        iterator.* = .{ .events = .{ .{ .chunk = env.service.counts.answer }, .{ .terminal = {} } } };
        env.service.counts.opened += 1;
        return .{ .success = iterator };
    }
    fn pull(iterator: *Iterator) fx.Effect(fx.Chunk(Event), i64, Env) {
        return fx.Effect(fx.Chunk(Event), i64, Env).fromBorrowed(Iterator, iterator, struct {
            fn read(current: *const Iterator, _: *const Env) fx.Exit(fx.Chunk(Event), i64) {
                if (current.emitted) return .{ .success = .end };
                @constCast(current).emitted = true;
                return .{ .success = fx.Chunk(Event).borrowed(&current.events) };
            }
        }.read);
    }
    fn close(iterator: *Iterator) fx.Effect(void, i64, Env) {
        return fx.Effect(void, i64, Env).fromBorrowed(Iterator, iterator, struct {
            fn release(current: *const Iterator, env: *const Env) fx.Exit(void, i64) {
                std.debug.assert(env.service.counts.live == 1 and env.service.counts.environments == 0);
                env.service.counts.closed += 1;
                env.allocator.destroy(@constCast(current));
                return .{ .success = {} };
            }
        }.release);
    }
    fn stream(_: *const i64, _: *const Env) Stream {
        return Stream.fromPull(fx.Effect(*Iterator, i64, Env).fromOutcome(create), pull, close);
    }
    fn captured(configuration: anytype, context: *const fx.RpcContext, request: *const i64, env: *const Env) Stream {
        std.debug.assert(context.correlation == 43 and configuration.owner.live.* == 1);
        env.service.counts.answer = request.* + configuration.offset;
        return stream(request, env);
    }
};
fn Wire(comptime T: type) type {
    return struct {
        fn decode(input: *const fx.OwnedSlice(u8)) fx.Effect(T, fx.SchemaIssue.Issue, Env) {
            return fx.Effect(T, fx.SchemaIssue.Issue, Env).fromBorrowed(fx.OwnedSlice(u8), input, struct {
                fn parse(supplied: *const fx.OwnedSlice(u8), env: *const Env) fx.Exit(T, fx.SchemaIssue.Issue) {
                    if (T == void) return .{ .success = {} };
                    const parsed = std.fmt.parseInt(T, supplied.items, 10) catch return .{ .fail = fx.SchemaIssue.Issue.init(env, .decode, .invalid_value) };
                    return .{ .success = parsed };
                }
            }.parse);
        }
        fn encode(input: *const T) fx.Effect(fx.OwnedSlice(u8), fx.SchemaIssue.Issue, Env) {
            return fx.Effect(fx.OwnedSlice(u8), fx.SchemaIssue.Issue, Env).fromBorrowed(T, input, struct {
                fn write(supplied: *const T, env: *const Env) fx.Exit(fx.OwnedSlice(u8), fx.SchemaIssue.Issue) {
                    const output = (if (T == void) env.allocator.dupe(u8, "null") else std.fmt.allocPrint(env.allocator, "{d}", .{supplied.*})) catch return .{ .die = fx.Defect.out_of_memory };
                    return .{ .success = .{ .items = output, .allocator = env.allocator } };
                }
            }.write);
        }
        const codec = fx.RpcWireCodec(T, Env).init(decode, encode);
    };
}
const Server = fx.RpcStreamServer(operation.handledBy(Env, Native.stream), Wire(i64).codec, Wire(i64).codec, Wire(void).codec, Wire(i64).codec, u8, struct {
    fn session(_: *const Env) *const fx.RpcSession(Env, u8) {
        unreachable;
    }
}.session, struct {
    fn policy(_: *const Env) fx.RpcSessionPolicy {
        unreachable;
    }
}.policy);
const Body = fx.Effect(i64, Server.Failure, Env);
const Registry = struct {
    fn use(registry: anytype, inputs: anytype, env: *const Env) Body {
        var opening = registry.openWith(inputs.scope, operation, inputs.context, inputs.request);
        var mapped = opening.mapErrorOwned(env, struct {
            fn domain(value: i64) Server.Failure {
                return .{ .domain = value };
            }
        }.domain);
        return mapped.acquireReleaseCapturedOwned(env, {}, struct {
            fn read(_: *const void, reader: Server.Reader, current: *const Env) Body {
                var pull = reader.pull.borrow();
                var aligned = pull.mapErrorOwned(current, struct {
                    fn domain(value: i64) Server.Failure {
                        return .{ .domain = value };
                    }
                }.domain);
                return aligned.mapOwned(current, struct {
                    fn answer(chunk: fx.Chunk(Event)) i64 {
                        var owned = chunk;
                        defer owned.deinit();
                        return owned.items.values[0].chunk;
                    }
                }.answer);
            }
        }.read, struct {
            fn close(_: *const void, reader: Server.Reader, _: fx.PrimaryOutcome(Server.Failure), _: *const Env) Server.Unit {
                var owned = reader;
                owned.deinit();
                return Server.Unit.succeed({});
            }
        }.close);
    }
};
const Managed = struct {
    fn acquire(_: anytype, _: *const Root) fx.Effect(Service, u8, Root) {
        return fx.Effect(Service, u8, Root).fromOutcome(struct {
            fn create(env: *const Root) fx.Exit(Service, u8) {
                return .{ .success = .{ .counts = env.counts } };
            }
        }.create);
    }
    fn project(_: anytype, service: *const Service, env: *const Root) Env {
        return .{ .allocator = env.allocator, .io = env.io, .service = service };
    }
    fn use(configuration: anytype, scope: *fx.Scope(i64), env: *const Env) Body {
        env.service.counts.live += 1;
        var context = fx.RpcContext.empty;
        context.correlation = 43;
        return fx.dsl.withRpcRegistry(env, .{ .definition = fx.dsl.declareMetadata(.{ .use = Registry.use }), .inputs = .{ .bindings = .{.{ .definition = fx.dsl.declareMetadata(.{ .operation = operation, .handler = Native.captured }), .inputs = .{ .owner = Owner{ .live = &env.service.counts.live }, .offset = configuration.offset } }}, .request = @as(i64, 17), .context = context, .scope = scope } });
    }
    fn release(_: anytype, service: *const Service, _: anytype, _: *const Root) fx.Effect(void, u8, Root) {
        std.debug.assert(service.counts.closed == 1 and service.counts.environments == 1);
        service.counts.releases += 1;
        return fx.Effect(void, u8, Root).succeed({});
    }
};

pub fn exercise(allocator: std.mem.Allocator, io: std.Io) void {
    exerciseCodecs(allocator);
    exerciseWriter(allocator);
    exercisePrograms(allocator, io);
    var counts: Counts = .{};
    const env: Root = .{ .allocator = allocator, .io = io, .counts = &counts };
    var program = fx.dsl.withRpcSession(&env, .{ .definition = fx.dsl.declareMetadata(.{ .server = Server, .acquire = Managed.acquire, .project = Managed.project, .use = Managed.use, .release = Managed.release }), .inputs = .{ .offset = @as(i64, @intCast(counts.opened + 25)) } });
    comptime if (@TypeOf(program) != fx.Effect(i64, fx.RpcMiddlewareFailure(Server.Failure, u8), Root)) @compileError("public managed session must remain the exact existing Effect");
    std.debug.assert(counts.opened == 0);
    var result = fx.run(&env, program);
    program.deinit();
    defer result.deinit();
    std.debug.assert(result == .success and result.success == 42);
    std.debug.assert(counts.live == 0 and counts.closed == 1 and counts.environments == 1 and counts.releases == 1 and counts.services == 1);
}

fn exerciseCodecs(allocator: std.mem.Allocator) void {
    const CodecEnv = struct { allocator: std.mem.Allocator };
    const E = enum { invalid };
    const Functions = struct {
        fn decode(configuration: anytype, input: anytype) fx.Result(u16, E) {
            return .{ .ok = @as(u16, input.*) + configuration.offset };
        }
        fn encode(configuration: anytype, input: anytype, _: *const CodecEnv) fx.Effect(u8, E, CodecEnv) {
            return fx.Effect(u8, E, CodecEnv).succeed(@intCast(input.* - configuration.offset));
        }
    };
    var live: usize = 1;
    const env: CodecEnv = .{ .allocator = allocator };
    var description = fx.dsl.makeCodec(&env, .{ .definition = fx.dsl.declareMetadata(.{ .kind = fx.dsl.CodecKind.pure_decode, .decode = Functions.decode, .encode = Functions.encode }), .inputs = .{ .owner = Owner{ .live = &live }, .offset = @as(u16, @intCast(live + 2)) } });
    var made = fx.run(&env, description);
    std.debug.assert(made == .success);
    const Codec = @TypeOf(made).Success;
    comptime if (!Codec.pure_decoding or Codec.pure_encoding or Codec.DecodingEnvironment != void or Codec.EncodingEnvironment != CodecEnv) @compileError("public mixed codec must retain explicit directional purity and services");
    var input = made.success.representation(39);
    var decoded = made.success.decodeDirect(&input);
    std.debug.assert(decoded == .ok and decoded.ok.value == 42);
    var pure_program = made.success.decode(&input);
    const unit: void = {};
    var lifted = fx.run(&unit, pure_program);
    pure_program.deinit();
    std.debug.assert(lifted == .success and lifted.success.value == 42);
    var program = made.success.encode(&decoded.ok);
    comptime if (@TypeOf(program) != fx.Effect(Codec.Representation, Codec.Error, CodecEnv)) @compileError("public captured codec must return the exact existing Effect");
    var encoded = fx.run(&env, program);
    program.deinit();
    decoded.deinit();
    lifted.deinit();
    input.deinit();
    made.deinit();
    description.deinit();
    std.debug.assert(encoded == .success and encoded.success.value == 39 and live == 1);
    encoded.deinit();
    std.debug.assert(live == 0);
}

fn exerciseWriter(allocator: std.mem.Allocator) void {
    const WriterEnv = struct { allocator: std.mem.Allocator };
    const Functions = struct {
        fn decode(_: anytype, input: *const std.json.Value) fx.Result(i64, fx.SchemaIssue.Issue) {
            return fx.JsonCodec.signed_integer.decodeDirect(input);
        }
        fn encode(_: anytype, input: *const i64) fx.Result(std.json.Value, fx.SchemaIssue.Issue) {
            return fx.JsonCodec.signed_integer.encodeDirect(input);
        }
        fn write(configuration: anytype, input: *const i64, output: *fx.JsonWriter.Writer) fx.Exit(void, fx.SchemaIssue.Issue) {
            const adjusted = input.* + configuration.offset;
            return fx.JsonCodec.signed_integer.writeJson(&adjusted, output);
        }
        fn limits(_: *const WriterEnv) fx.JsonWriter.Limits {
            return .{ .max_output_bytes = 64, .max_json_depth = 4, .max_value_bytes = 32, .max_integer_digits = 16 };
        }
    };
    var live: usize = 1;
    const env: WriterEnv = .{ .allocator = allocator };
    var description = fx.dsl.makeCodec(&env, .{ .definition = fx.dsl.declareMetadata(.{ .kind = fx.dsl.CodecKind.pure, .decode = Functions.decode, .encode = Functions.encode, .writer = Functions.write }), .inputs = .{ .owner = Owner{ .live = &live }, .offset = @as(i64, @intCast(live + 2)) } });
    var made = fx.run(&env, description);
    std.debug.assert(made == .success);
    const Codec = @TypeOf(made).Success;
    comptime if (!Codec.has_explicit_writer) @compileError("public captured writer requires explicit validated evidence");
    var input = made.success.value(39);
    var validated = made.success.validateDirect(&input);
    std.debug.assert(validated == .ok);
    validated.deinit();
    var program = made.success.writeWith(&input, Functions.limits);
    comptime if (@TypeOf(program) != fx.Effect(fx.OwnedSlice(u8), Codec.Error, WriterEnv)) @compileError("public captured writer must remain the exact Effect with inferred caller environment");
    var output = fx.run(&env, program);
    program.deinit();
    input.deinit();
    made.deinit();
    description.deinit();
    std.debug.assert(live == 0);
    std.debug.assert(output == .success and std.mem.eql(u8, output.success.items, "42"));
    output.deinit();
}

fn exercisePrograms(allocator: std.mem.Allocator, io: std.Io) void {
    const R = struct { allocator: std.mem.Allocator, io: std.Io, live: *usize, cancellation: *fx.Cancellation };
    const E = enum { invalid };
    const Token = struct {
        pub const effect_owned_payload = true;
        live: *usize,
        pub fn deinit(self: *@This()) void {
            self.live.* -= 1;
        }
    };
    const State = struct { token: Token, number: u32 };
    const Functions = struct {
        fn source(_: *const R) fx.Effect(State, E, R) {
            return fx.Effect(State, E, R).fromOutcome(struct {
                fn create(env: *const R) fx.Exit(State, E) {
                    env.live.* += 1;
                    return .{ .success = .{ .token = .{ .live = env.live }, .number = 41 } };
                }
            }.create);
        }
        fn number(value: *const State, _: *const R) fx.Effect(u32, E, R) {
            return fx.Effect(u32, E, R).succeed(value.number);
        }
        fn early(_: *const R) bool {
            return true;
        }
        fn immediate(env: *const R) fx.Effect(State, E, R) {
            env.live.* += 1;
            env.cancellation.request();
            return fx.Effect(State, E, R).succeed(.{ .token = .{ .live = env.live }, .number = 1 });
        }
        const transferred = fx.dsl.program.describe(.{ fx.dsl.program.binding("owned", .{}, source), fx.dsl.program.returnWhen("early", .{}, early, "owned"), fx.dsl.program.binding("fallback", .{}, source) });
        const projected = fx.dsl.program.describe(.{ fx.dsl.program.binding("owned", .{}, source), fx.dsl.program.binding("number", .{"owned"}, number) });
        const interrupted_branch = fx.dsl.program.describe(.{fx.dsl.program.branch("answer", .{}, early, immediate, immediate)});
    };
    comptime if (@TypeOf(Functions.transferred) != fx.Effect(fx.dsl.ownership.Capture(State), E, R)) @compileError("public program must generate native ownership while retaining exact Effect");
    var live: usize = 0;
    var normal_cancellation: fx.Cancellation = .{};
    const env: R = .{ .allocator = allocator, .io = io, .live = &live, .cancellation = &normal_cancellation };
    var transferred = fx.run(&env, Functions.transferred);
    std.debug.assert(transferred == .success and transferred.success.value.number == 41 and live == 1);
    transferred.deinit();
    std.debug.assert(live == 0);
    var projected = fx.run(&env, Functions.projected);
    std.debug.assert(projected == .success and projected.success == 41 and live == 0);
    projected.deinit();
    var cancellation: fx.Cancellation = .{};
    const cancelled_env: R = .{ .allocator = allocator, .io = io, .live = &live, .cancellation = &cancellation };
    var interrupted = fx.run(&cancelled_env, Functions.interrupted_branch);
    std.debug.assert(interrupted == .interrupted and live == 0);
    interrupted.deinit();
    std.debug.assert(live == 0);
}
