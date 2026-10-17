const std = @import("std");
const fx = @import("effect");
const E = enum { provider };
const W = fx.WorkflowBuilder(E, R);
const Clock = fx.WorkflowDurableClock(R, void, .@"volatile");
const Queue = fx.PersistedQueue(R, void, .@"volatile");
const Deferred = fx.WorkflowDurableDeferred(R, void, .@"volatile");
const Child = fx.WorkflowChildStore(R, void, .@"volatile");
const Engine = fx.WorkflowEngine(R, void, .@"volatile");
const Parallel = fx.WorkflowDurableParallel;
const Journal = Parallel.Journal(R, void, .@"volatile");
fn Arg(comptime F: type, comptime index: usize) type {
    return @typeInfo(@typeInfo(F).pointer.child).@"fn".param_types[index].?;
}
const Schedule = Arg(@FieldType(Clock.VTable, "schedule"), 2);
const ClaimPolicy = Arg(@FieldType(Clock.VTable, "claim_due"), 2);
const Token = Arg(@FieldType(Clock.VTable, "acknowledge"), 2);
const Submission = Arg(@FieldType(Queue.VTable, "offer"), 2);
const Identity = Arg(@FieldType(Deferred.VTable, "observe"), 2);
const R = struct {
    allocator: std.mem.Allocator,
    io: std.Io = std.testing.io,
    timer_store: *const Clock = undefined,
    clock: fx.Clock = fx.Clock.fromIo(&std.testing.io),
    queue: *const Queue = undefined,
    deferred: *const Deferred = undefined,
    children: *const Child = undefined,
    engine: *const Engine = undefined,
    journal: *const Journal = undefined,
    execution: fx.WorkflowTypes.ExecutionId = undefined,
    child: fx.WorkflowTypes.ExecutionId = undefined,
    now: i96 = 1,
    replay_arena: *std.heap.ArenaAllocator = undefined,
    token: ?Token = null,
};
const Roles = struct {
    fn now(env: *const R) fx.WallInstant {
        return .{ .unix_nanoseconds = env.now };
    }
    fn clock(env: *const R) *const Clock {
        return env.timer_store;
    }
    fn queue(env: *const R) *const Queue {
        return env.queue;
    }
    fn deferred(env: *const R) *const Deferred {
        return env.deferred;
    }
    fn children(env: *const R) *const Child {
        return env.children;
    }
    fn engine(env: *const R) *const Engine {
        return env.engine;
    }
    fn journal(env: *const R) *const Journal {
        return env.journal;
    }
    fn execution(env: *const R) fx.WorkflowTypes.ExecutionId {
        return env.execution;
    }
    fn schedule(env: *const R) Schedule {
        return .{ .identity = .{ .execution = env.execution, .occurrence = "retry" }, .due_at = .{ .unix_nanoseconds = 2 } };
    }
    fn policy(env: *const R) ClaimPolicy {
        return .{ .owner_id = "worker", .generation = 1, .lease_until = .{ .at = .{ .unix_nanoseconds = env.now + 5 } }, .max_items = 1 };
    }
    fn token(env: *const R) Token {
        return env.token.?;
    }
    fn submission(env: *const R) Submission {
        return .{ .queue_key = env.execution.encoded, .offer_key = env.child.encoded, .encoded_payload = "work", .visible_at = .{ .unix_nanoseconds = 1 }, .retention_until = .never };
    }
    fn identity(env: *const R) Identity {
        return .{ .execution = env.execution, .name = "reply", .version = 1 };
    }
    fn link(env: *const R) fx.WorkflowChildLink {
        return .{ .parent = env.execution, .child = env.child };
    }
    fn exit(_: *const R) []const u8 {
        return "completed";
    }
    fn clockError(problem: Clock.Error) E {
        var owned = problem;
        defer owned.deinit();
        return .provider;
    }
    fn queueError(problem: fx.WorkflowDurableQueue.Error(Queue.Failure, Deferred.Failure)) E {
        var owned = problem;
        defer owned.deinit();
        return .provider;
    }
    fn branch(env: *const R) Parallel.Branch {
        return .{ .execution = env.execution, .name = "calculation", .version = 1 };
    }
    fn attempt(_: *const R) u64 {
        return 1;
    }
    fn parallelError(problem: Parallel.RunError(Journal.Failure, fx.SchemaIssue.Issue)) E {
        var owned = problem;
        defer owned.deinit();
        return .provider;
    }
    fn childFailure(problem: Child.Failure) E {
        var owned = problem;
        defer owned.deinit();
        return .provider;
    }
};
const scheduled = W.clockSchedule(void, .@"volatile", Roles.clock, Roles.schedule, Roles.clockError);
const claimed = W.clockClaimDue(void, .@"volatile", Roles.clock, Roles.policy, Roles.clockError);
const acknowledged = W.clockAcknowledge(void, .@"volatile", Roles.clock, Roles.token, Roles.clockError);
const offered = W.queueOfferAndObserve(void, void, .@"volatile", .@"volatile", Roles.queue, Roles.deferred, Roles.submission, Roles.identity, Roles.queueError);
const completed = W.childCompleteAndWake(void, void, .@"volatile", .@"volatile", Roles.children, Roles.engine, Roles.link, Roles.exit, Roles.childFailure);
const reconciled = W.childReconcile(void, void, .@"volatile", .@"volatile", Roles.children, Roles.engine, Roles.execution, Roles.childFailure);
fn run(env: *const R, comptime step: anytype) fx.Exit(@TypeOf(step).Success, E) {
    var description = W.run(step);
    defer description.deinit();
    return fx.run(env, description);
}
test "WF-06 durable queue clock and child recovery keep native deduplication leases and pending completion authority" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var env: R = .{ .allocator = std.testing.allocator, .replay_arena = &arena };
    var parallel_memory = fx.MemoryWorkflowParallelJournal(R).init(&env);
    defer parallel_memory.deinit();
    const parallel_cap = parallel_memory.capability();
    env.journal = &parallel_cap;
    var clock = fx.MemoryWorkflowDurableClock(R, Roles.now).init(&env);
    defer clock.deinit();
    const clock_cap = clock.capability();
    env.timer_store = &clock_cap;
    var queue = fx.MemoryPersistedQueue(R, Roles.now).init(&env);
    defer queue.deinit();
    const queue_cap = queue.capability();
    env.queue = &queue_cap;
    var deferred = fx.MemoryWorkflowDurableDeferred(R).init(&env);
    defer deferred.deinit();
    const deferred_cap = deferred.capability();
    env.deferred = &deferred_cap;
    var children = fx.MemoryWorkflowChildStore(R).init(&env);
    defer children.deinit();
    const child_cap = children.capability();
    env.children = &child_cap;
    var engine = fx.MemoryWorkflowEngine(R, Roles.now).init(&env);
    defer engine.deinit();
    const engine_cap = engine.capability();
    env.engine = &engine_cap;
    var execution = fx.PrimaryKey.encodeDirect(&env, "recovery", &.{.{ .text = "parent" }});
    defer execution.deinit();
    try std.testing.expect(execution == .success);
    env.execution = .{ .encoded = execution.success.items };
    var child = fx.PrimaryKey.encodeDirect(&env, "recovery", &.{.{ .text = "child" }});
    defer child.deinit();
    try std.testing.expect(child == .success);
    env.child = .{ .encoded = child.success.items };
    var initial = run(&env, scheduled);
    defer initial.deinit();
    try std.testing.expect(initial == .success and initial.success == .scheduled);
    var resumed = run(&env, scheduled);
    defer resumed.deinit();
    try std.testing.expect(resumed == .success and resumed.success == .already_scheduled);
    var before_due = run(&env, claimed);
    defer before_due.deinit();
    try std.testing.expect(before_due == .success);
    env.now = 2;
    var first_claim = run(&env, claimed);
    defer first_claim.deinit();
    try std.testing.expect(first_claim == .success and first_claim.success == .claimed);
    try std.testing.expectEqual(@as(usize, 1), first_claim.success.claimed.items.len);
    env.now = 8;
    var reclaimed = run(&env, claimed);
    defer reclaimed.deinit();
    try std.testing.expect(reclaimed == .success and reclaimed.success == .claimed);
    try std.testing.expect(reclaimed.success.claimed.items[0].attempt > first_claim.success.claimed.items[0].attempt);
    const current = reclaimed.success.claimed.items[0];
    env.token = .{ .identity = .{ .execution = .{ .encoded = current.execution_key.items }, .occurrence = current.occurrence.items }, .owner_id = current.owner_id.items, .generation = current.generation, .attempt = current.attempt };
    var ack = run(&env, acknowledged);
    defer ack.deinit();
    try std.testing.expect(ack == .success);
    var offered_once = run(&env, offered);
    defer offered_once.deinit();
    try std.testing.expect(offered_once == .success and offered_once.success == .observed);
    try std.testing.expect(offered_once.success.observed.offer == .accepted);
    var offered_again = run(&env, offered);
    defer offered_again.deinit();
    try std.testing.expect(offered_again == .success and offered_again.success == .observed);
    try std.testing.expect(offered_again.success.observed.offer == .already_offered);
    var registering = child_cap.vtable.register(child_cap.userdata, &env, Roles.link(&env));
    defer registering.deinit();
    var registered = fx.run(&env, registering);
    defer registered.deinit();
    try std.testing.expect(registered == .success);
    var completion = run(&env, completed);
    defer completion.deinit();
    try std.testing.expect(completion == .success);
    var recovered = run(&env, reconciled);
    defer recovered.deinit();
    try std.testing.expect(recovered == .success);
    try std.testing.expectEqual(@as(usize, 1), recovered.success.pending_count);
    var acknowledging = child_cap.vtable.acknowledge(child_cap.userdata, &env, Roles.link(&env));
    defer acknowledging.deinit();
    var child_ack = fx.run(&env, acknowledging);
    defer child_ack.deinit();
    try std.testing.expect(child_ack == .success);
    var after_ack = run(&env, reconciled);
    defer after_ack.deinit();
    try std.testing.expect(after_ack == .success);
    try std.testing.expectEqual(@as(usize, 0), after_ack.success.pending_count);
    var branch = run(&env, recorded_branch);
    defer branch.deinit();
    try std.testing.expect(branch == .success and branch.success.source == .executed);
    var replay = run(&env, recorded_branch);
    defer replay.deinit();
    try std.testing.expect(replay == .success and replay.success.source == .replayed);
    try std.testing.expectEqual(@as(u32, 19), replay.success.exit.success);
    const retry = comptime W.retryBranch(u32, .{ .concurrency = 1, .max_attempts = 2 }, W.fail(u32, .provider));
    var exhausted = run(&env, retry);
    defer exhausted.deinit();
    try std.testing.expect(exhausted == .fail and exhausted.fail == .provider);
}

const Issue = fx.SchemaIssue.Issue;
const IntegerExit = fx.Exit(u32, E);
const codec = fx.TypedOutcomeCodec(u32, E, R, R).init(Codec.integer, Codec.failure, Codec.readInteger, Codec.readFailure);
const Codec = struct {
    const limits: fx.StoredOutcome.Limits = .{ .max_payload_bytes = 256, .max_cleanup_records = 8, .max_total_bytes = 1024 };
    fn integer(value: *const u32, env: *const R) fx.Exit(fx.OwnedSlice(u8), Issue) {
        const bytes = env.allocator.alloc(u8, 4) catch return .{ .die = fx.Defect.out_of_memory };
        std.mem.writeInt(u32, @ptrCast(bytes.ptr), value.*, .little);
        return .{ .success = .{ .items = bytes, .allocator = env.allocator } };
    }
    fn failure(_: *const E, env: *const R) fx.Exit(fx.OwnedSlice(u8), Issue) {
        return .{ .success = .{ .items = env.allocator.alloc(u8, 0) catch return .{ .die = fx.Defect.out_of_memory }, .allocator = env.allocator } };
    }
    fn readInteger(bytes: []const u8, _: *const R) fx.Exit(u32, Issue) {
        return if (bytes.len == 4) .{ .success = std.mem.readInt(u32, @ptrCast(bytes.ptr), .little) } else .{ .fail = Issue.pure(.decode, .invalid_value) };
    }
    fn readFailure(bytes: []const u8, _: *const R) fx.Exit(E, Issue) {
        return if (bytes.len == 0) .{ .success = .provider } else .{ .fail = Issue.pure(.decode, .invalid_value) };
    }
    fn cast(comptime A: type, value: anytype) fx.Exit(A, Issue) {
        return switch (value) {
            .success => unreachable,
            .fail => |err| .{ .fail = err },
            .die => |err| .{ .die = err },
            .interrupted => .interrupted,
            .finalization_failed => |err| .{ .finalization_failed = err },
        };
    }
    fn encode(value: *const IntegerExit, env: *const R) fx.Exit(fx.OwnedSlice(u8), Issue) {
        const encoded = codec.encodeDirect(env, value);
        if (encoded != .success) return cast(fx.OwnedSlice(u8), encoded);
        var stored = encoded.success;
        defer stored.deinit();
        return fx.StoredOutcome.encode(env, &stored, limits);
    }
    fn decode(bytes: []const u8, env: *const R) fx.Exit(IntegerExit, Issue) {
        var decoding = env.*;
        decoding.allocator = env.replay_arena.allocator();
        const parsed = fx.StoredOutcome.decode(&decoding, bytes, limits);
        if (parsed != .success) return cast(IntegerExit, parsed);
        const decoded = codec.decodeDirect(&decoding, parsed.success);
        if (decoded != .success) return cast(IntegerExit, decoded);
        return .{ .success = decoded.success.value };
    }
};
const recorded_branch = W.recordedBranch(u32, W.succeed(@as(u32, 19)), void, Issue, .@"volatile", Roles.journal, Roles.branch, Roles.attempt, Codec.encode, Codec.decode, Roles.parallelError);
