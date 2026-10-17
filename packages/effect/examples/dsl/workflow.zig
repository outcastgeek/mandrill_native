//! EX-05: business-key traversal, recorded activity replay, and compensation.
const std = @import("std");
const fx = @import("effect");
pub const E = enum { identity, recording, compensation, business };
const W = fx.WorkflowBuilder(E, R);
const Engine = fx.WorkflowEngine(R, void, .@"volatile");
const C = fx.WorkflowCompensation;
const Journal = C.Journal(R, void, .@"volatile");
const Issue = fx.SchemaIssue.Issue;
const Raw = fx.WorkflowActivityRunError(Engine.Failure, Issue);
const Recorded = fx.WorkflowRecordedActivity(i64, void);
const Row = struct { id: []const u8, amount: i64 };
pub const R = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    replay_arena: *std.heap.ArenaAllocator,
    engine: *const Engine = undefined,
    journal: *const Journal = undefined,
    execution: fx.WorkflowTypes.ExecutionId = undefined,
    activation: fx.WorkflowTypes.Activation = undefined,
    calls: *usize,
    undos: *usize,
};
const Inputs = struct { rows: []const Row, attempt: u64 = 1 };
const ActivityInputs = struct { amount: *const i64, key: []const u8, attempt: u64 };
fn castFailure(comptime A: type, value: anytype) fx.Exit(A, @TypeOf(value).Error) {
    return switch (value) {
        .success => unreachable,
        .fail => |problem| .{ .fail = problem },
        .die => |problem| .{ .die = problem },
        .interrupted => .interrupted,
        .finalization_failed => |problem| .{ .finalization_failed = problem },
    };
}
fn Payload(comptime T: type) type {
    return struct {
        fn encode(value: *const T, env: *const R) fx.Exit(fx.OwnedSlice(u8), Issue) {
            const size: usize = if (T == void) 0 else if (T == i64) 8 else 1;
            const bytes = env.allocator.alloc(u8, size) catch return .{ .die = fx.Defect.out_of_memory };
            if (comptime T == i64) std.mem.writeInt(i64, @ptrCast(bytes.ptr), value.*, .little) else if (comptime T != void) {
                bytes[0] = @backingInt(value.*);
            }
            return .{ .success = .{ .items = bytes, .allocator = env.allocator } };
        }
        fn decode(bytes: []const u8, _: *const R) fx.Exit(T, Issue) {
            if (comptime T == void) return if (bytes.len == 0) .{ .success = {} } else .{ .fail = Issue.pure(.decode, .invalid_value) };
            if (comptime T == i64) return if (bytes.len == 8) .{ .success = std.mem.readInt(i64, @ptrCast(bytes.ptr), .little) } else .{ .fail = Issue.pure(.decode, .invalid_value) };
            if (bytes.len != 1 or bytes[0] >= @typeInfo(T).@"enum".field_names.len) return .{ .fail = Issue.pure(.decode, .invalid_value) };
            return .{ .success = @fromBackingInt(@intCast(bytes[0])) };
        }
    };
}
fn OutcomeCodec(comptime A: type, comptime D: type) type {
    return struct {
        const Exit = fx.Exit(A, D);
        const codec = fx.TypedOutcomeCodec(A, D, R, R).init(Payload(A).encode, Payload(D).encode, Payload(A).decode, Payload(D).decode);
        const limits: fx.StoredOutcome.Limits = .{ .max_payload_bytes = 256, .max_cleanup_records = 8, .max_total_bytes = 1024 };
        fn encode(value: *const Exit, env: *const R) fx.Exit(fx.OwnedSlice(u8), Issue) {
            const prepared = codec.encodeDirect(env, value);
            if (prepared != .success) return castFailure(fx.OwnedSlice(u8), prepared);
            var stored = prepared.success;
            defer stored.deinit();
            return fx.StoredOutcome.encode(env, &stored, limits);
        }
        fn decode(bytes: []const u8, env: *const R) fx.Exit(Exit, Issue) {
            // The run owns an arena through inspection of all replayed exits,
            // including defect diagnostics borrowed from the decoded wire owner.
            var decoding = env.*;
            decoding.allocator = env.replay_arena.allocator();
            const parsed = fx.StoredOutcome.decode(&decoding, bytes, limits);
            if (parsed != .success) return castFailure(Exit, parsed);
            const decoded = codec.decodeDirect(&decoding, parsed.success);
            if (decoded != .success) return castFailure(Exit, decoded);
            return .{ .success = decoded.success.value };
        }
    };
}
pub const Calls = struct {
    pub fn now(_: *const R) fx.WallInstant {
        return .{ .unix_nanoseconds = 1 };
    }
    fn handler(amount: *const i64, env: *const R) fx.Effect(i64, void, R) {
        env.calls.* += 1;
        return fx.Effect(i64, void, R).succeed(amount.* * 2);
    }
    fn engine(_: *const ActivityInputs, env: *const R) *const Engine {
        return env.engine;
    }
    fn activation(_: *const ActivityInputs, env: *const R) fx.WorkflowTypes.Activation {
        return env.activation;
    }
    fn activity(input: *const ActivityInputs, _: *const R) fx.WorkflowTypes.ActivityId {
        return .{ .encoded = input.key, .attempt = input.attempt };
    }
    fn payload(input: *const ActivityInputs, _: *const R) *const i64 {
        return input.amount;
    }
    fn activityError(problem: Raw) E {
        var owned = problem;
        defer owned.deinit();
        return .recording;
    }
    fn identityError(_: W.TraversalError) E {
        return .identity;
    }
    fn execution(_: *const Inputs, env: *const R) fx.WorkflowTypes.ExecutionId {
        return env.execution;
    }
    fn items(input: *const Inputs, _: *const R) []const Row {
        return input.rows;
    }
    fn row(input: *const Inputs, value: *const Row, iteration: *const W.Iteration, env: *const R) W.OwnedStep(Recorded) {
        return W.activityCaptured(env, ActivityInputs{ .amount = &value.amount, .key = iteration.key, .attempt = input.attempt }, .{ .handler = activity_definition, .engine = engine, .activation = activation, .activity = activity, .payload = payload, .encode_exit = OutcomeCodec(i64, void).encode, .decode_exit = OutcomeCodec(i64, void).decode, .map_error = activityError });
    }
    fn undo(_: *const i64, env: *const R) fx.Effect(void, void, R) {
        env.undos.* += 1;
        return fx.Effect(void, void, R).succeed({});
    }
    fn undoEngine(env: *const R) *const Engine {
        return env.engine;
    }
    fn undoActivation(env: *const R) fx.WorkflowTypes.Activation {
        return env.activation;
    }
    fn undoActivity(env: *const R) fx.WorkflowTypes.ActivityId {
        return .{ .encoded = env.execution.encoded, .attempt = 1 };
    }
    fn undoPayload(_: *const R) *const i64 {
        return &@as(i64, 0);
    }
    fn unit(result: fx.WorkflowRecordedActivity(void, void)) void {
        var owned = result;
        defer owned.deinit();
    }
    fn journal(env: *const R) *const Journal {
        return env.journal;
    }
    fn compensationIdentity(env: *const R) C.Identity {
        return .{ .execution = env.execution, .name = "release-reservation", .version = 1 };
    }
    fn compensationAttempt(env: *const R) C.Attempt {
        return .{ .identity = compensationIdentity(env), .ordinal = 1, .attempt = 1 };
    }
    fn journalError(problem: Journal.Failure) E {
        var owned = problem;
        defer owned.deinit();
        return .compensation;
    }
    fn compensationError(problem: C.RunError(Journal.Failure, Issue)) E {
        var owned = problem;
        defer owned.deinit();
        return .compensation;
    }
};
const activity_definition = fx.dsl.activity(.{ .name = "invoice-line", .version = 1, .payload = fx.JsonCodec.signed_integer, .success = fx.JsonCodec.signed_integer, .failure = fx.JsonCodec.null_value, .handler = Calls.handler });
pub fn invoice(env: *const R, input: Inputs) fx.Effect(fx.OwnedSlice(Recorded), E, R) {
    var traversal = W.forEach(env, input, .{ .name = "invoice-lines", .version = 1, .key_field = "id", .execution = Calls.execution, .items = Calls.items, .body = Calls.row, .map_error = Calls.identityError });
    return traversal.intoEffect();
}
// The release is itself a recorded activity; the compensation journal records
// its completion and retains the original exit independently of replay.
const undo_definition = fx.dsl.activity(.{ .name = "release-reservation", .version = 1, .payload = fx.JsonCodec.signed_integer, .success = fx.JsonCodec.null_value, .failure = fx.JsonCodec.null_value, .handler = Calls.undo });
const undo = W.map(W.activityMapped(undo_definition, void, Issue, .@"volatile", Calls.undoEngine, Calls.undoActivation, Calls.undoActivity, Calls.undoPayload, OutcomeCodec(void, void).encode, OutcomeCodec(void, void).decode, Calls.activityError), Calls.unit);
const registration = W.registerCompensation(void, .@"volatile", Calls.journal, Calls.compensationIdentity, Calls.journalError);
const compensation = W.recordedCompensation(undo, void, Issue, .@"volatile", Calls.journal, Calls.compensationAttempt, OutcomeCodec(void, E).encode, OutcomeCodec(void, E).decode, Calls.compensationError);
fn check(allocator: std.mem.Allocator, io: std.Io) !void {
    var calls: usize = 0;
    var undos: usize = 0;
    var arena = std.heap.ArenaAllocator.init(allocator);
    defer arena.deinit();
    var env: R = .{ .allocator = allocator, .io = io, .replay_arena = &arena, .calls = &calls, .undos = &undos };
    var memory = fx.MemoryWorkflowEngine(R, Calls.now).init(&env);
    defer memory.deinit();
    const engine = memory.capability();
    env.engine = &engine;
    var compensation_memory = fx.MemoryWorkflowCompensation(R).init(&env);
    defer compensation_memory.deinit();
    const journal = compensation_memory.capability();
    env.journal = &journal;
    var key = fx.PrimaryKey.encodeDirect(&env, "invoice", &.{.{ .text = "invoice-42" }});
    defer key.deinit();
    if (key != .success) return error.Defect;
    env.execution = .{ .encoded = key.success.items };
    const definition: fx.WorkflowTypes.Definition = .{ .name = "invoice", .version = 1, .payload_schema_version = 1, .success_schema_version = 1, .error_schema_version = 1 };
    var submitting = engine.vtable.submit(engine.userdata, &env, .{ .definition = definition, .execution = env.execution, .encoded_payload = "invoice-42" });
    defer submitting.deinit();
    var submitted = fx.run(&env, submitting);
    defer submitted.deinit();
    try std.testing.expect(submitted == .success);
    var activating = engine.vtable.activate(engine.userdata, &env, env.execution, "invoice-worker", .{ .lease_until = .{ .at = .{ .unix_nanoseconds = 1000 } } });
    defer activating.deinit();
    var activated = fx.run(&env, activating);
    defer activated.deinit();
    try std.testing.expect(activated == .success and activated.success == .activated);
    env.activation = activated.success.activated.activation();
    const rows = [_]Row{ .{ .id = "book", .amount = 12 }, .{ .id = "pen", .amount = 3 } };
    var description = invoice(&env, .{ .rows = &rows });
    var first = fx.run(&env, description);
    description.deinit();
    defer first.deinit();
    try std.testing.expect(first == .success);
    try std.testing.expectEqual(@as(usize, 2), calls);
    try std.testing.expectEqual(@as(i64, 24), first.success.items[0].exit.success);
    const reordered = [_]Row{ rows[1], rows[0] };
    var again = invoice(&env, .{ .rows = &reordered });
    var replay = fx.run(&env, again);
    again.deinit();
    defer replay.deinit();
    try std.testing.expect(replay == .success);
    try std.testing.expectEqual(@as(usize, 2), calls);
    try std.testing.expect(replay.success.items[0].source == .replayed);
    try std.testing.expectEqual(@as(i64, 6), replay.success.items[0].exit.success);
    const duplicates = [_]Row{ rows[0], rows[0] };
    var invalid = invoice(&env, .{ .rows = &duplicates });
    var rejected = fx.run(&env, invalid);
    invalid.deinit();
    defer rejected.deinit();
    try std.testing.expect(rejected == .fail and rejected.fail == .identity);
    try std.testing.expectEqual(@as(usize, 2), calls);
    var registering = W.run(registration);
    defer registering.deinit();
    var registered = fx.run(&env, registering);
    defer registered.deinit();
    try std.testing.expect(registered == .success);
    var compensating = W.run(compensation);
    defer compensating.deinit();
    var undone = fx.run(&env, compensating);
    defer undone.deinit();
    try std.testing.expect(undone == .success and undone.success.source == .executed);
    var repeated = fx.run(&env, compensating);
    defer repeated.deinit();
    try std.testing.expect(repeated == .success and repeated.success.source == .replayed);
    try std.testing.expectEqual(@as(usize, 1), undos);
}
pub fn main(init: std.process.Init) !void {
    try check(init.gpa, init.io);
}
test "EX-05 WF-03 WF-04 WF-05 durable traversal derives business identities replays reordered rows rejects duplicates and records compensation" {
    try check(std.testing.allocator, std.testing.io);
}
