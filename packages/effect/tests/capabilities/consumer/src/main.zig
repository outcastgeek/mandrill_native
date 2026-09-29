//! Independent public-module consumer for native typed contracts.
const std = @import("std");
const fx = @import("effect");

const Env = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    value: u8,
    instant: fx.WallInstant,
    store: ?*const fx.KeyValueStore(@This(), void) = null,
    transaction: ?*fx.MemoryTransaction(u8) = null,
    atom_registry: ?*fx.AtomRegistry = null,
    atom_key: []const u8 = "",
    event_journal: ?*const fx.EventJournal(@This(), void) = null,
    event_id: []const u8 = "",
    event_payload: []const u8 = "",
    workflow_engine: ?*const fx.WorkflowEngine(@This(), void, .@"volatile") = null,
    workflow_activation: ?fx.WorkflowTypes.Activation = null,
    workflow_activity: ?fx.WorkflowTypes.ActivityId = null,
    activity_calls: ?*usize = null,
    execution_key: []const u8 = "",
    cluster_store: ?*const fx.ClusterMessageStore(@This(), void, .@"volatile") = null,
    cluster_entity_key: []const u8 = "",
    cluster_owner_id: []const u8 = "",
    cluster_generation: u64 = 0,
};
const Error = enum { unexpected };
const BuiltStreaming = fx.RpcProtocolBuilder(void, Env).streaming("consumer.built-stream", 1)
    .request(fx.JsonCodec.signed_integer)
    .chunk(fx.JsonCodec.signed_integer)
    .terminal(fx.JsonCodec.null_value)
    .failure(fx.JsonCodec.null_value);
const BinaryValue = struct { answer: u8, valid: bool };
const ConsumerRecord = struct { value: i64 };
const ConsumerRecordSchema = fx.SchemaRecordBuilder(ConsumerRecord, Env).begin()
    .fields(.{fx.SchemaBuilder.required("value", fx.SchemaRepresentation.signed_integer)})
    .policy(.{ .unknown_fields = .reject, .duplicate_fields = .reject, .issues = .first })
    .build();
const EventValueCodec = fx.PureCodec(u8, u8).init(struct {
    fn decode(value: *const u8) fx.Result(u8, fx.SchemaIssue.Issue) {
        return .{ .ok = value.* };
    }
}.decode, struct {
    fn encode(value: *const u8) fx.Result(u8, fx.SchemaIssue.Issue) {
        return .{ .ok = value.* };
    }
}.encode);
const EventErrorCodec = fx.PureCodec(Error, u8).init(struct {
    fn decode(value: *const u8) fx.Result(Error, fx.SchemaIssue.Issue) {
        if (value.* != @intFromEnum(Error.unexpected))
            return .{ .err = fx.SchemaIssue.Issue.pure(.decode, .invalid_value) };
        return .{ .ok = .unexpected };
    }
}.decode, struct {
    fn encode(value: *const Error) fx.Result(u8, fx.SchemaIssue.Issue) {
        return .{ .ok = @intFromEnum(value.*) };
    }
}.encode);
const EventFixture = struct {
    fn handle(value: *const u8, _: *const Env) fx.Effect(u8, Error, Env) {
        return fx.Effect(u8, Error, Env).succeed(value.*);
    }
};
fn eventValue(env: *const Env) *const u8 {
    return &env.value;
}
fn eventIdentity(env: *const Env) []const u8 {
    return env.event_id;
}
fn encodeEventPayload(codec: *const @TypeOf(EventValueCodec), value: *const u8, env: *const Env) fx.Effect(fx.OwnedSlice(u8), fx.SchemaIssue.Issue, Env) {
    const Out = fx.Effect(fx.OwnedSlice(u8), fx.SchemaIssue.Issue, Env);
    const encoded = switch (codec.encodeDirect(value)) {
        .ok => |byte| byte,
        .err => |problem| return Out.fromOwnedOutcome(env, .{ .fail = problem }),
    };
    const bytes = env.allocator.alloc(u8, @sizeOf(u8)) catch return Out.die(fx.Defect.out_of_memory);
    bytes[0] = encoded;
    return Out.fromOwnedSuccess(env, .{ .items = bytes, .allocator = env.allocator });
}
const CompletedEvent = fx.EventBuilder.event("consumer.created", 1)
    .payload(EventValueCodec)
    .success(EventValueCodec)
    .failure(EventErrorCodec)
    .handledBy(Env, EventFixture.handle);
const EventGroup = fx.EventBuilder.group(.{CompletedEvent});
const CompletedEntity = fx.ClusterEntityBuilder(Error, Env).begin("consumer.entity", 1)
    .request(EventValueCodec)
    .success(EventValueCodec)
    .failure(EventErrorCodec)
    .handledBy(EventFixture.handle);
const EntityRegistry = fx.ClusterEntityBuilder(Error, Env).registry(.{CompletedEntity});
const CompletedWorkflow = fx.WorkflowBuilder(Error, Env).begin(.{
    .name = "consumer.workflow",
    .version = 1,
    .payload_schema_version = 1,
    .success_schema_version = 1,
    .error_schema_version = 1,
}).payload(EventValueCodec).success(EventValueCodec).failure(EventErrorCodec).handledBy(EventFixture.handle);
const WorkflowRegistry = fx.WorkflowBuilder(Error, Env).registry(.{CompletedWorkflow});
const CompletedActivity = fx.WorkflowActivityBuilder(Error, Env).begin("consumer.activity", 1)
    .payload(EventValueCodec)
    .success(EventValueCodec)
    .failure(EventErrorCodec)
    .handledBy(EventFixture.handle);
const ActivityRegistry = fx.WorkflowActivityBuilder(Error, Env).registry(.{CompletedActivity});
const RecordedActivityFixture = struct {
    fn handle(value: *const u8, env: *const Env) fx.Effect(u8, void, Env) {
        env.activity_calls.?.* += 1;
        return fx.Effect(u8, void, Env).succeed(value.*);
    }
};
const RecordedActivity = fx.WorkflowActivityBuilder(void, Env).begin("consumer.recorded-activity", 1)
    .payload(EventValueCodec)
    .success(EventValueCodec)
    .failure(fx.JsonCodec.null_value)
    .handledBy(RecordedActivityFixture.handle);
const InterruptedActivity = RecordedActivity.definition.handledBy(Env, struct {
    fn handle(_: *const u8, _: *const Env) fx.Effect(u8, void, Env) {
        return fx.Effect(u8, void, Env).interrupted();
    }
}.handle);
const ProjectionFixture = struct {
    fn decode(_: anytype, env: *const Env, encoded: []const u8) fx.Exit(u8, fx.SchemaIssue.Issue) {
        _ = env;
        if (encoded.len != @sizeOf(u8))
            return .{ .fail = fx.SchemaIssue.Issue.pure(.decode, .invalid_value) };
        return switch (CompletedEvent.definition.decodePayloadDirect(&encoded[0])) {
            .ok => |value| .{ .success = value },
            .err => |problem| .{ .fail = problem },
        };
    }

    fn fold(_: anytype, state: *i64, event_value: *const u8) fx.Result(void, void) {
        state.* += event_value.*;
        return .{ .ok = {} };
    }
};
const EventProjection = EventGroup.projection(fx.JsonCodec.signed_integer, void, ProjectionFixture.decode, ProjectionFixture.fold);

const GroupedRequest = struct {
    pub const Input = struct { group: []const u8, value: u8 };
    pub const Success = u8;
    pub const DomainError = void;
};
const ResolverEnv = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    resolver: *const fx.RequestResolver(GroupedRequest, @This(), void),
    jobs: []const fx.RequestResolver(GroupedRequest, @This(), void).Job,
    calls: *usize,
};
const GroupedResolver = fx.RequestResolver(GroupedRequest, ResolverEnv, void);
const ResolverFixture = struct {
    fn selected(env: *const ResolverEnv) *const GroupedResolver {
        return env.resolver;
    }

    fn jobs(env: *const ResolverEnv) []const GroupedResolver.Job {
        return env.jobs;
    }

    fn group(job: *const GroupedResolver.Job) []const u8 {
        return job.input.group;
    }

    fn resolve(_: ?*anyopaque, env: *const ResolverEnv, jobs_to_resolve: []const GroupedResolver.Job) GroupedResolver.BatchEffect {
        return GroupedResolver.BatchEffect.fromOwned(env, jobs_to_resolve, struct {
            fn run(selected_jobs: *const []const GroupedResolver.Job, current: *const ResolverEnv) fx.Exit(GroupedResolver.Results, GroupedResolver.Failure) {
                current.calls.* += 1;
                const first_group = selected_jobs.*[0].input.group;
                for (selected_jobs.*) |job| std.debug.assert(std.mem.eql(u8, first_group, job.input.group));
                const results = current.allocator.alloc(GroupedResolver.ItemExit, selected_jobs.len) catch
                    return .{ .die = fx.Defect.out_of_memory };
                for (selected_jobs.*, results) |job, *result| result.* = .{ .success = job.input.value };
                return .{ .success = .{ .items = results, .allocator = current.allocator } };
            }
        }.run);
    }
};

const TxSubscription = fx.TransactionSubscriptionRef(u8);
const TxEnv = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    domain: *fx.TransactionDomain,
    capture: *const TxSubscription.AwaitCapture,
};
const TxFixture = struct {
    fn domain(env: *const TxEnv) *fx.TransactionDomain {
        return env.domain;
    }

    fn capture(env: *const TxEnv) *const TxSubscription.AwaitCapture {
        return env.capture;
    }

    fn increment(reference: *const *TxSubscription, journal: *fx.TransactionDomain.Journal) fx.TransactionAtomicDecision(u8, void) {
        const before = reference.*.read(journal) orelse return .retry;
        reference.*.write(journal, before.value + 1);
        return .{ .commit = before.value + 1 };
    }
};

fn exerciseCommittedStream(init: std.process.Init) void {
    var tx_env: TxEnv = .{ .allocator = init.gpa, .io = init.io, .domain = undefined, .capture = undefined };
    var domain = fx.TransactionDomain.init(&tx_env);
    defer domain.deinit();
    tx_env.domain = &domain;
    var reference = TxSubscription.init(&domain, 1);
    const capture: TxSubscription.AwaitCapture = .{ .reference = &reference, .version = 0 };
    tx_env.capture = &capture;
    const selected_reference: *TxSubscription = &reference;
    const update = fx.TransactionDomain.atomically(u8, void, TxEnv, *TxSubscription, &selected_reference, TxFixture.domain, TxFixture.increment);
    var updated = fx.run(&tx_env, update);
    defer updated.deinit();
    std.debug.assert(updated == .success and updated.success == 2);

    const Failure = fx.TransactionError(void);
    var scope = fx.Scope(Failure).init(&tx_env);
    const stream = comptime fx.TransactionChangesAfter(u8, TxEnv, TxFixture.capture, TxFixture.domain);
    var opened = fx.run(&tx_env, stream.open(&scope));
    std.debug.assert(opened == .success);
    var observed = fx.run(&tx_env, opened.success.pull.borrow());
    std.debug.assert(observed == .success and observed.success == .items);
    std.debug.assert(observed.success.items.values.len == 1);
    std.debug.assert(observed.success.items.values[0].value == 2 and observed.success.items.values[0].version == 1);
    observed.deinit();
    opened.deinit();
    var closed = fx.run(&tx_env, fx.Effect(void, Failure, TxEnv).closeScope(&scope));
    std.debug.assert(closed == .success);
    closed.deinit();
    scope.deinit();
}

fn selected(env: *const Env) u8 {
    return env.value;
}
fn now(env: *const Env) fx.WallInstant {
    return env.instant;
}
fn store(env: *const Env) *const fx.KeyValueStore(Env, void) {
    return env.store.?;
}
fn key(_: *const Env) []const u8 {
    return "consumer-key";
}
fn payload(_: *const Env) []const u8 {
    return "consumer-value";
}
fn expiry(_: *const Env) fx.Deadline {
    return .never;
}
fn encodeByte(value: *const u8, env: *const Env) fx.Exit(fx.OwnedSlice(u8), fx.SchemaIssue.Issue) {
    const bytes = env.allocator.alloc(u8, @sizeOf(u8)) catch return .{ .die = fx.Defect.out_of_memory };
    bytes[0] = value.*;
    return .{ .success = .{ .items = bytes, .allocator = env.allocator } };
}
fn decodeByte(bytes: []const u8, _: *const Env) fx.Exit(u8, fx.SchemaIssue.Issue) {
    if (bytes.len != @sizeOf(u8)) return .{ .fail = fx.SchemaIssue.Issue.pure(.decode, .invalid_value) };
    return .{ .success = bytes[0] };
}
fn encodeVoid(_: *const void, env: *const Env) fx.Exit(fx.OwnedSlice(u8), fx.SchemaIssue.Issue) {
    const bytes = env.allocator.alloc(u8, 0) catch return .{ .die = fx.Defect.out_of_memory };
    return .{ .success = .{ .items = bytes, .allocator = env.allocator } };
}
fn decodeVoid(bytes: []const u8, _: *const Env) fx.Exit(void, fx.SchemaIssue.Issue) {
    if (bytes.len != 0) return .{ .fail = fx.SchemaIssue.Issue.pure(.decode, .invalid_value) };
    return .{ .success = {} };
}
const stored_outcome_limits: fx.StoredOutcome.Limits = .{
    .max_payload_bytes = 256,
    .max_cleanup_records = 8,
    .max_total_bytes = 512,
};
fn encodeClusterExit(value: *const fx.Exit(u8, void), env: *const Env) fx.Exit(fx.OwnedSlice(u8), fx.SchemaIssue.Issue) {
    const codec = comptime fx.TypedOutcomeCodec(u8, void, Env, Env).init(encodeByte, encodeVoid, decodeByte, decodeVoid);
    const prepared = codec.encodeDirect(env, value);
    return switch (prepared) {
        .success => |encoded| blk: {
            var owned = encoded;
            defer owned.deinit();
            break :blk fx.StoredOutcome.encode(env, &owned, stored_outcome_limits);
        },
        .fail => |problem| .{ .fail = problem },
        .die => |defect| .{ .die = defect },
        .interrupted => .interrupted,
        .finalization_failed => |details| .{ .finalization_failed = details },
    };
}
fn issueFailure(comptime A: type, value: anytype) fx.Exit(A, fx.SchemaIssue.Issue) {
    return switch (value) {
        .success => unreachable,
        .fail => |problem| .{ .fail = problem },
        .die => |defect| .{ .die = defect },
        .interrupted => .interrupted,
        .finalization_failed => |details| .{ .finalization_failed = details },
    };
}
fn decodeClusterExit(bytes: []const u8, env: *const Env) fx.Exit(fx.Exit(u8, void), fx.SchemaIssue.Issue) {
    const parsed = fx.StoredOutcome.decode(env, bytes, stored_outcome_limits);
    const stored = switch (parsed) {
        .success => |value| value,
        else => return issueFailure(fx.Exit(u8, void), parsed),
    };
    const codec = comptime fx.TypedOutcomeCodec(u8, void, Env, Env).init(encodeByte, encodeVoid, decodeByte, decodeVoid);
    const decoded = codec.decodeDirect(env, stored);
    return switch (decoded) {
        .success => |value| blk: {
            var owned = value;
            defer owned.deinit();
            const copy = owned.value.clone();
            if (copy == .die and copy.die.kind == .out_of_memory and
                !(owned.value == .die and owned.value.die.kind == .out_of_memory))
                break :blk .{ .die = fx.Defect.out_of_memory };
            break :blk .{ .success = copy };
        },
        else => issueFailure(fx.Exit(u8, void), decoded),
    };
}
fn clusterStore(env: *const Env) *const fx.ClusterMessageStore(Env, void, .@"volatile") {
    return env.cluster_store.?;
}
fn clusterAddress(env: *const Env) fx.ClusterTypes.EntityAddress {
    return .{ .group = "consumer.entity", .entity_key = env.cluster_entity_key };
}
fn clusterOwner(env: *const Env) fx.ClusterTypes.Ownership {
    return .{ .owner_id = env.cluster_owner_id, .generation = env.cluster_generation };
}
fn clusterClaimPolicy(_: *const Env) fx.ClusterClaimPolicy {
    return .{ .max_messages = 1, .lease_until = .{ .at = .{ .unix_nanoseconds = 500 } } };
}
fn clusterHandler(claim: *const fx.ClusterClaimedMessage, _: *const Env) fx.Effect(u8, void, Env) {
    return fx.Effect(u8, void, Env).succeed(claim.encoded_request.items[0]);
}
fn transaction(env: *const Env) *fx.MemoryTransaction(u8) {
    return env.transaction.?;
}
fn registry(env: *const Env) *fx.AtomRegistry {
    return env.atom_registry.?;
}
fn atomKey(env: *const Env) []const u8 {
    return env.atom_key;
}
fn readAtom(_: *const Env) fx.Effect(u8, void, Env) {
    return fx.Effect(u8, void, Env).service(selected);
}
fn eventJournal(env: *const Env) *const fx.EventJournal(Env, void) {
    return env.event_journal.?;
}
fn proposal(env: *const Env) fx.EventProposal {
    return .{ .tag = "consumer.created", .event_id = env.event_id, .encoded = env.event_payload, .schema_version = 1 };
}
fn beforeFirst(_: *const Env) fx.EventCursor {
    return .before_first;
}
fn pagePolicy(_: *const Env) fx.EventReadPolicy {
    return .{ .max_entries = 8, .max_encoded_bytes = 512 };
}
fn echo(request: *const i64, env: *const Env) fx.Effect(i64, void, Env) {
    return fx.Effect(i64, void, Env).fromOwned(env, request.*, struct {
        fn run(value: *const i64, _: *const Env) fx.Exit(i64, void) {
            return .{ .success = value.* };
        }
    }.run);
}
fn emptyBuiltStream(_: *const i64, _: *const Env) fx.Stream(BuiltStreaming.OperationType.Event, void, Env) {
    const Event = BuiltStreaming.OperationType.Event;
    const Pull = fx.Effect(fx.Chunk(Event), void, Env);
    const Unit = fx.Effect(void, void, Env);
    return comptime fx.Stream(Event, void, Env).fromPull(Unit.succeed({}), struct {
        fn next(_: void) Pull {
            return Pull.fromOutcome(struct {
                fn end(_: *const Env) fx.Exit(fx.Chunk(Event), void) {
                    return .{ .success = .end };
                }
            }.end);
        }
    }.next, struct {
        fn release(_: void) Unit {
            return Unit.succeed({});
        }
    }.release);
}
fn workflowEngine(env: *const Env) *const fx.WorkflowEngine(Env, void, .@"volatile") {
    return env.workflow_engine.?;
}
fn execution(env: *const Env) fx.WorkflowTypes.ExecutionId {
    return .{ .encoded = env.execution_key };
}
fn submission(env: *const Env) fx.WorkflowTypes.Submission {
    return .{ .definition = .{ .name = "consumer.workflow", .version = 1, .payload_schema_version = 1, .success_schema_version = 1, .error_schema_version = 1 }, .execution = execution(env), .encoded_payload = env.event_payload };
}
fn workflowOwner(_: *const Env) []const u8 {
    return "consumer-workflow-owner";
}
fn workflowActivationPolicy(_: *const Env) fx.WorkflowTypes.ActivationPolicy {
    return .{ .lease_until = .{ .at = .{ .unix_nanoseconds = 1000 } } };
}
fn workflowActivation(env: *const Env) fx.WorkflowTypes.Activation {
    return env.workflow_activation.?;
}
fn workflowActivity(env: *const Env) fx.WorkflowTypes.ActivityId {
    return env.workflow_activity.?;
}
const Cell = fx.MemoryTransaction(u8).Cell;
fn increment(cell: *const *Cell, journal: *fx.MemoryTransaction(u8).Journal) fx.TransactionDecision(u8, void) {
    const before = journal.read(cell.*) orelse return .retry;
    journal.write(cell.*, before + 1);
    return .{ .commit = before + 1 };
}

pub fn main(init: std.process.Init) void {
    _ = ConsumerRecordSchema.encodedNode();
    const event_resolution = EventGroup.resolve("consumer.created", 1);
    std.debug.assert(event_resolution == .ok and event_resolution.ok == 0);
    const entity_resolution = EntityRegistry.resolve("consumer.entity", 1);
    std.debug.assert(entity_resolution == .ok and entity_resolution.ok == 0);
    std.debug.assert(WorkflowRegistry.resolve("consumer.workflow", 1).? == 0);
    std.debug.assert(ActivityRegistry.resolve("consumer.activity", 1).? == 0);
    exerciseCommittedStream(init);
    var env: Env = .{ .allocator = init.gpa, .io = init.io, .value = 7, .instant = .{ .unix_nanoseconds = 0 } };
    const integer_codec = comptime fx.IntegerCodec(i16);
    const option_codec = comptime fx.OptionCodec.option(integer_codec);
    const optional_value: ?i16 = -12;
    const optional_encoded = option_codec.encodeDirect(&optional_value);
    std.debug.assert(optional_encoded == .ok and optional_encoded.ok == .some);
    const optional_decoded = option_codec.decodeDirect(&optional_encoded.ok);
    std.debug.assert(optional_decoded == .ok and optional_decoded.ok.? == optional_value.?);
    const tuple_codec = comptime fx.TupleCodec.tuple(.{ integer_codec, fx.JsonCodec.boolean });
    const tuple_value: @TypeOf(tuple_codec).Decoded = .{ .items = .{ @as(i16, -12), true } };
    const tuple_encoded = tuple_codec.encodeDirect(&tuple_value);
    std.debug.assert(tuple_encoded == .ok);
    const tuple_decoded = tuple_codec.decodeDirect(&tuple_encoded.ok);
    std.debug.assert(tuple_decoded == .ok and tuple_decoded.ok.items[0] == -12 and tuple_decoded.ok.items[1]);
    const decimal_codec = comptime fx.FixedDecimalCodec(i64, 2);
    const decimal_value: fx.FixedDecimal(i64, 2) = .{ .coefficient = 1234 };
    const decimal_encoded = decimal_codec.encodeDirect(&decimal_value);
    std.debug.assert(decimal_encoded == .ok and std.mem.eql(u8, decimal_encoded.ok.slice(), "12.34"));
    const decimal_decoded = decimal_codec.decodeDirect(&decimal_encoded.ok);
    std.debug.assert(decimal_decoded == .ok and decimal_decoded.ok.coefficient == decimal_value.coefficient);
    const instant_encoded = fx.WallInstantCodec.encodeDirect(&env.instant);
    std.debug.assert(instant_encoded == .ok);
    const instant_decoded = fx.WallInstantCodec.decodeDirect(&instant_encoded.ok);
    std.debug.assert(instant_decoded == .ok and instant_decoded.ok.unix_nanoseconds == env.instant.unix_nanoseconds);
    var activity_body = fx.run(&env, comptime fx.WorkflowActivityBuilder(Error, Env).succeed(@as(u8, 1)));
    defer activity_body.deinit();
    std.debug.assert(activity_body == .success and activity_body.success == 1);
    const PureWorkflow = fx.WorkflowBuilder(void, Env);
    const pure_step = comptime PureWorkflow.bind(PureWorkflow.succeed(@as(u8, 1)), struct {
        fn next(value: u8) @TypeOf(PureWorkflow.succeed(@as(u8, 0))) {
            return PureWorkflow.succeed(value + 1);
        }
    }.next);
    var pure_workflow = fx.run(&env, comptime PureWorkflow.run(pure_step));
    defer pure_workflow.deinit();
    std.debug.assert(pure_workflow == .success and pure_workflow.success == 2);
    const event_encoded = CompletedEvent.definition.encodePayloadDirect(&env.value);
    std.debug.assert(event_encoded == .ok and event_encoded.ok == env.value);
    const success_encoded = CompletedEvent.definition.schema(.success).encodeDirect(&env.value);
    std.debug.assert(success_encoded == .ok and success_encoded.ok == env.value);
    const event_error: Error = .unexpected;
    const error_encoded = CompletedEvent.definition.schema(.failure).encodeDirect(&event_error);
    std.debug.assert(error_encoded == .ok);
    const error_decoded = CompletedEvent.definition.schema(.failure).decodeDirect(&error_encoded.ok);
    std.debug.assert(error_decoded == .ok and error_decoded.ok == event_error);
    const program = comptime fx.Effect(u8, Error, Env).service(selected);
    var result = fx.run(&env, program);
    defer result.deinit();
    std.debug.assert(result == .success and result.success == env.value);

    var group_a = fx.PrimaryKey.encodeDirect(&env, "consumer-group", &.{.{ .text = "a" }});
    defer group_a.deinit();
    var group_b = fx.PrimaryKey.encodeDirect(&env, "consumer-group", &.{.{ .text = "b" }});
    defer group_b.deinit();
    var request_a = fx.PrimaryKey.encodeDirect(&env, "consumer-request", &.{.{ .text = "a" }});
    defer request_a.deinit();
    var request_b = fx.PrimaryKey.encodeDirect(&env, "consumer-request", &.{.{ .text = "b" }});
    defer request_b.deinit();
    var request_c = fx.PrimaryKey.encodeDirect(&env, "consumer-request", &.{.{ .text = "c" }});
    defer request_c.deinit();
    std.debug.assert(group_a == .success and group_b == .success and request_a == .success and request_b == .success and request_c == .success);
    const resolver_inputs = [_]GroupedRequest.Input{
        .{ .group = group_a.success.items, .value = 1 },
        .{ .group = group_b.success.items, .value = 2 },
        .{ .group = group_a.success.items, .value = 3 },
    };
    const resolver_jobs = [_]GroupedResolver.Job{
        .{ .key = request_a.success.items, .input = &resolver_inputs[0] },
        .{ .key = request_b.success.items, .input = &resolver_inputs[1] },
        .{ .key = request_c.success.items, .input = &resolver_inputs[2] },
    };
    const grouped_provider = GroupedResolver.fromProvider(null, &.{ .resolve_batch = ResolverFixture.resolve });
    var grouped_calls: usize = 0;
    const resolver_env: ResolverEnv = .{ .allocator = init.gpa, .io = init.io, .resolver = &grouped_provider, .jobs = &resolver_jobs, .calls = &grouped_calls };
    const grouped_program = GroupedResolver.resolveGrouped(ResolverFixture.selected, ResolverFixture.jobs, ResolverFixture.group);
    var grouped_result = fx.run(&resolver_env, grouped_program);
    defer grouped_result.deinit();
    std.debug.assert(grouped_result == .success);
    std.debug.assert(grouped_calls == 2);
    std.debug.assert(grouped_result.success.items.len == resolver_inputs.len);
    for (grouped_result.success.items, resolver_inputs) |resolved, input| {
        std.debug.assert(resolved == .success and resolved.success == input.value);
    }

    const Binary = fx.TypedBinary.Codec(BinaryValue, "consumer.schema.v1");
    const limits: fx.TypedBinary.Limits = .{
        .frame = .{ .max_schema_name_bytes = 64, .max_payload_bytes = 128, .max_frame_bytes = 256 },
        .max_collection_items = 32,
        .max_depth = 8,
    };
    const value: BinaryValue = .{ .answer = env.value, .valid = true };
    var encoded = Binary.encode(&env, &value, limits);
    defer encoded.deinit();
    std.debug.assert(encoded == .success);
    var decoded = Binary.decode(&env, encoded.success.items, limits);
    defer decoded.deinit();
    std.debug.assert(decoded == .success and decoded.success.value.answer == env.value);

    var memory = fx.MemoryPersistence(Env, now).init(&env);
    defer memory.deinit();
    var capability = memory.keyValue();
    env.store = &capability;
    const Store = fx.KeyValueStore(Env, void);
    var written = fx.run(&env, Store.write(store, key, payload, expiry));
    defer written.deinit();
    std.debug.assert(written == .success and written.success == .committed);
    var read = fx.run(&env, comptime Store.read(store, key));
    defer read.deinit();
    std.debug.assert(read == .success and read.success == .found);
    std.debug.assert(std.mem.eql(u8, read.success.found.items, payload(&env)));

    var domain = fx.MemoryTransaction(u8).init(&env);
    defer domain.deinit();
    env.transaction = &domain;
    var cell = Cell.init(&domain, env.value);
    const selected_cell: *Cell = &cell;
    const atomic = fx.MemoryTransaction(u8).atomically(u8, void, Env, *Cell, &selected_cell, transaction, increment);
    var committed = fx.run(&env, atomic);
    defer committed.deinit();
    std.debug.assert(committed == .success and committed.success == env.value + 1);
    std.debug.assert(cell.value == committed.success);

    var identity = fx.PrimaryKey.encodeDirect(&env, "consumer-atom", &.{.{ .text = "value" }});
    defer identity.deinit();
    std.debug.assert(identity == .success);
    env.atom_key = identity.success.items;
    const persisted = comptime fx.PersistableBuilder(u8, void, Env, void, Env, Env).start()
        .withCodec(fx.TypedOutcomeCodec(u8, void, Env, Env).init(encodeByte, encodeVoid, decodeByte, decodeVoid))
        .withSchemaVersion(1)
        .withLimits(.{ .max_payload_bytes = 256, .max_cleanup_records = 8, .max_total_bytes = 512 })
        .withStore(store)
        .withKey(atomKey)
        .withExpiry(expiry)
        .withSharedEnvironment()
        .build();
    const stored_value: fx.Exit(u8, void) = .{ .success = env.value };
    var persist_write = persisted.writeValue(&env, &stored_value);
    defer persist_write.deinit();
    var persist_result = fx.run(&env, persist_write);
    defer persist_result.deinit();
    std.debug.assert(persist_result == .success and persist_result.success == .committed);
    var persist_read = persisted.read(&env);
    defer persist_read.deinit();
    var read_result = fx.run(&env, persist_read);
    defer read_result.deinit();
    std.debug.assert(read_result == .success and read_result.success == .found);
    std.debug.assert(read_result.success.found.value == .success and read_result.success.found.value.success == env.value);
    const opened = fx.AtomRegistry.create(&env, .{ .max_nodes = 8, .max_dependencies_per_node = 4 }, 8);
    const atom_registry = switch (opened) {
        .success => |created_registry| created_registry,
        else => unreachable,
    };
    defer atom_registry.deinit();
    env.atom_registry = atom_registry;
    const atom = comptime fx.AtomBuilder(u8, void, Env).begin("consumer.atom", 1)
        .key(atomKey)
        .dependsOn(&.{})
        .fromEffect(readAtom)
        .readOnly();
    var registered = fx.run(&env, comptime atom.register(registry));
    defer registered.deinit();
    std.debug.assert(registered == .success);
    var observed = fx.run(&env, comptime atom.read());
    defer observed.deinit();
    std.debug.assert(observed == .success and observed.success == env.value);

    var event_identity = fx.PrimaryKey.encodeDirect(&env, "consumer-event", &.{.{ .text = "created" }});
    defer event_identity.deinit();
    std.debug.assert(event_identity == .success);
    env.event_id = event_identity.success.items;
    const event_bytes = [_]u8{event_encoded.ok};
    env.event_payload = &event_bytes;
    var event_memory = fx.MemoryEventJournal(Env).init(&env);
    defer event_memory.deinit();
    const event_capability = event_memory.capability();
    env.event_journal = &event_capability;
    const Journal = fx.EventJournal(Env, void);
    var appended = fx.run(&env, comptime Journal.append(eventJournal, proposal));
    defer appended.deinit();
    std.debug.assert(appended == .success and appended.success == .committed);
    var page = fx.run(&env, comptime Journal.readAfter(eventJournal, beforeFirst, pagePolicy));
    defer page.deinit();
    std.debug.assert(page == .success and page.success.entries.items.len == 1);
    std.debug.assert(std.mem.eql(u8, page.success.entries.items[0].encoded.items, env.event_payload));
    const initial_projection: i64 = 0;
    var replayed = EventProjection.replayPage(&env, &initial_projection, .before_first, &page.success);
    defer replayed.deinit();
    std.debug.assert(replayed == .success and replayed.success.state == env.value);
    const catch_up = comptime fx.EventJournalCatchUp(Env, void, eventJournal, beforeFirst, pagePolicy);
    var caught_up = fx.run(&env, catch_up.runCollect(4));
    defer caught_up.deinit();
    std.debug.assert(caught_up == .success and caught_up.success.items.len == 1);
    std.debug.assert(caught_up.success.items[0].entries.items.len == 1);
    var typed_event_identity = fx.PrimaryKey.encodeDirect(&env, "consumer-event", &.{.{ .text = "typed" }});
    defer typed_event_identity.deinit();
    std.debug.assert(typed_event_identity == .success);
    env.event_id = typed_event_identity.success.items;
    const typed_append = CompletedEvent.appendEncoded(void, eventValue, eventJournal, eventIdentity, encodeEventPayload);
    var typed_appended = fx.run(&env, typed_append);
    defer typed_appended.deinit();
    std.debug.assert(typed_appended == .success and typed_appended.success == .committed);
    std.debug.assert(typed_appended.success.committed.result == env.value);
    var legacy_event_identity = fx.PrimaryKey.encodeDirect(&env, "consumer-event", &.{.{ .text = "legacy" }});
    defer legacy_event_identity.deinit();
    std.debug.assert(legacy_event_identity == .success);
    env.event_id = legacy_event_identity.success.items;
    const legacy_append = fx.EventLogHandleAndAppend(CompletedEvent, Env, void, eventValue, eventJournal, proposal);
    var legacy_appended = fx.run(&env, legacy_append);
    defer legacy_appended.deinit();
    std.debug.assert(legacy_appended == .success and legacy_appended.success == .committed);

    const integer = fx.JsonCodec.signed_integer;
    const nothing = fx.JsonCodec.null_value;
    const operation = comptime fx.RpcUnary(@TypeOf(integer), @TypeOf(integer), @TypeOf(nothing))
        .init("consumer.echo", 1, integer, integer, nothing);
    const wrapped_operation = comptime fx.RpcMiddlewareOperation(operation, nothing, Env);
    std.debug.assert(std.mem.eql(u8, wrapped_operation.name, operation.name));
    const handler = comptime operation.handledBy(Env, echo);
    const BuiltUnary = fx.RpcProtocolBuilder(void, Env).unary("consumer.built-echo", 1)
        .request(integer)
        .success(integer)
        .failure(nothing);
    const built_handler = comptime BuiltUnary.handledBy(echo);
    const built_stream_handler = comptime BuiltStreaming.handledBy(emptyBuiltStream);
    const BuiltProtocol = fx.RpcProtocolBuilder(void, Env).registry(.{ built_handler, built_stream_handler });
    const built_resolution = BuiltProtocol.resolve("consumer.built-echo", 1);
    std.debug.assert(built_resolution == .ok and built_resolution.ok == 0);
    const stream_resolution = BuiltProtocol.resolve("consumer.built-stream", 1);
    std.debug.assert(stream_resolution == .ok and stream_resolution.ok == 1);
    const request: i64 = env.value;
    var reply = fx.run(&env, handler.invoke(&request));
    defer reply.deinit();
    std.debug.assert(reply == .success and reply.success == request);

    var workflow_identity = fx.PrimaryKey.encodeDirect(&env, "consumer-workflow", &.{.{ .text = "run" }});
    defer workflow_identity.deinit();
    std.debug.assert(workflow_identity == .success);
    env.execution_key = workflow_identity.success.items;
    var workflow_memory = fx.MemoryWorkflowEngine(Env, now).init(&env);
    defer workflow_memory.deinit();
    const workflow_capability = workflow_memory.capability();
    env.workflow_engine = &workflow_capability;
    const Workflow = fx.WorkflowEngine(Env, void, .@"volatile");
    var submitted = fx.run(&env, comptime Workflow.submit(workflowEngine, submission));
    defer submitted.deinit();
    std.debug.assert(submitted == .success and submitted.success == .accepted);
    var workflow_state = fx.run(&env, comptime Workflow.poll(workflowEngine, execution));
    defer workflow_state.deinit();
    std.debug.assert(workflow_state == .success and workflow_state.success == .queued);
    var activated = fx.run(&env, comptime Workflow.activate(workflowEngine, execution, workflowOwner, workflowActivationPolicy));
    defer activated.deinit();
    std.debug.assert(activated == .success and activated.success == .activated);
    env.workflow_activation = activated.success.activated.activation();
    var activity_identity = fx.PrimaryKey.encodeDirect(&env, "consumer-activity", &.{.{ .text = "run" }});
    defer activity_identity.deinit();
    std.debug.assert(activity_identity == .success);
    env.workflow_activity = .{ .encoded = activity_identity.success.items, .attempt = 1 };
    var activity_calls: usize = 0;
    env.activity_calls = &activity_calls;
    const DurableWorkflow = fx.WorkflowBuilder(
        fx.WorkflowActivityRunError(Workflow.Failure, fx.SchemaIssue.Issue),
        Env,
    );
    const activity_step = comptime DurableWorkflow.activity(
        RecordedActivity,
        void,
        fx.SchemaIssue.Issue,
        .@"volatile",
        workflowEngine,
        workflowActivation,
        workflowActivity,
        eventValue,
        encodeClusterExit,
        decodeClusterExit,
    );
    const run_activity = DurableWorkflow.run(activity_step);
    var first_activity = fx.run(&env, run_activity);
    defer first_activity.deinit();
    std.debug.assert(first_activity == .success and first_activity.success.source == .executed);
    std.debug.assert(first_activity.success.exit == .success and first_activity.success.exit.success == env.value);
    var replayed_activity = fx.run(&env, run_activity);
    defer replayed_activity.deinit();
    std.debug.assert(replayed_activity == .success and replayed_activity.success.source == .replayed);
    std.debug.assert(replayed_activity.success.exit == .success and replayed_activity.success.exit.success == env.value);
    std.debug.assert(activity_calls == 1);
    var interrupted_identity = fx.PrimaryKey.encodeDirect(&env, "consumer-activity", &.{.{ .text = "interrupted" }});
    defer interrupted_identity.deinit();
    std.debug.assert(interrupted_identity == .success);
    env.workflow_activity = .{ .encoded = interrupted_identity.success.items, .attempt = 1 };
    const interrupted_step = comptime DurableWorkflow.activity(
        InterruptedActivity,
        void,
        fx.SchemaIssue.Issue,
        .@"volatile",
        workflowEngine,
        workflowActivation,
        workflowActivity,
        eventValue,
        encodeClusterExit,
        decodeClusterExit,
    );
    var interrupted_activity = fx.run(&env, DurableWorkflow.run(interrupted_step));
    defer interrupted_activity.deinit();
    std.debug.assert(interrupted_activity == .interrupted);
    var resumed_activity = fx.run(&env, run_activity);
    defer resumed_activity.deinit();
    std.debug.assert(resumed_activity == .success and resumed_activity.success.source == .executed);
    std.debug.assert(activity_calls == 2);

    var ownership_memory = fx.MemoryClusterOwnership(Env, now).init(&env);
    defer ownership_memory.deinit();
    const ownership = ownership_memory.capability();
    const address: fx.ClusterTypes.EntityAddress = .{ .group = "consumer.entity", .entity_key = env.atom_key };
    var acquire = ownership.vtable.acquire(ownership.userdata, &env, address, "consumer-owner", .{ .until = .{ .at = .{ .unix_nanoseconds = 1000 } } });
    defer acquire.deinit();
    var acquired = fx.run(&env, acquire);
    defer acquired.deinit();
    std.debug.assert(acquired == .success and acquired.success == .acquired);
    var message_memory = fx.MemoryClusterMessageStore(Env, now).init(&env, ownership_memory.fence());
    defer message_memory.deinit();
    const message_capability = message_memory.capability();
    env.cluster_store = &message_capability;
    env.cluster_entity_key = env.atom_key;
    const lease = acquired.success.acquired.token();
    env.cluster_owner_id = lease.owner_id;
    env.cluster_generation = lease.generation;
    const message_address: fx.ClusterTypes.Address = .{
        .group = address.group,
        .entity_key = address.entity_key,
        .request_key = env.event_id,
    };
    const request_bytes = [_]u8{env.value};
    var accept_message = message_capability.vtable.accept(message_capability.userdata, &env, .{
        .address = message_address,
        .definition_version = 1,
        .encoded_request = &request_bytes,
        .retention_until = .{ .at = .{ .unix_nanoseconds = 1000 } },
    });
    defer accept_message.deinit();
    var message_accepted = fx.run(&env, accept_message);
    defer message_accepted.deinit();
    std.debug.assert(message_accepted == .success and message_accepted.success == .accepted);
    const cluster_program = fx.ClusterClaimAndServe(
        u8,
        void,
        Env,
        void,
        fx.SchemaIssue.Issue,
        .@"volatile",
        clusterStore,
        clusterAddress,
        clusterOwner,
        clusterClaimPolicy,
        clusterHandler,
        encodeClusterExit,
    );
    var served = fx.run(&env, cluster_program);
    defer served.deinit();
    std.debug.assert(served == .success and served.success.completed == 1 and served.success.stopped == .drained);
    var read_reply_effect = message_capability.vtable.read_reply(message_capability.userdata, &env, message_address);
    defer read_reply_effect.deinit();
    var saved_reply = fx.run(&env, read_reply_effect);
    defer saved_reply.deinit();
    std.debug.assert(saved_reply == .success and saved_reply.success == .completed);
    const decoded_reply = fx.StoredOutcome.decode(&env, saved_reply.success.completed.items, stored_outcome_limits);
    std.debug.assert(decoded_reply == .success);
    const stored_codec = comptime fx.TypedOutcomeCodec(u8, void, Env, Env).init(encodeByte, encodeVoid, decodeByte, decodeVoid);
    var typed_reply = stored_codec.decodeDirect(&env, decoded_reply.success);
    defer typed_reply.deinit();
    std.debug.assert(typed_reply == .success and typed_reply.success.value == .success and typed_reply.success.value.success == env.value);
    var release = ownership.vtable.release(ownership.userdata, &env, acquired.success.acquired.token());
    defer release.deinit();
    var released = fx.run(&env, release);
    defer released.deinit();
    std.debug.assert(released == .success and released.success == .released);
}
