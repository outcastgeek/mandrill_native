const std = @import("std");
const fx = @import("effect");
const Mode = enum { success, failure, defect, interrupted };
const Env = struct {
    allocator: std.mem.Allocator,
    io: std.Io = std.testing.io,
    engine: ?*const fx.WorkflowEngine(@This(), void, .@"volatile") = null,
    activation: ?fx.WorkflowTypes.Activation = null,
    activity: fx.WorkflowTypes.ActivityId = .{ .encoded = "", .attempt = 1 },
    input: i64 = 17,
    mode: Mode = .success,
    calls: *usize,
    decoded: *?fx.DecodedStoredOutcome(i64, void),
};
const Engine = fx.WorkflowEngine(Env, void, .@"volatile");
const CodecError = fx.SchemaIssue.Issue;
const ActivityExit = fx.Exit(i64, void);
const Failure = fx.WorkflowActivityRunError(Engine.Failure, CodecError);
const Output = fx.WorkflowRecordedActivity(i64, void);
const W = fx.WorkflowBuilder(Failure, Env);
const limits: fx.StoredOutcome.Limits = .{ .max_payload_bytes = 256, .max_cleanup_records = 8, .max_total_bytes = 512 };
fn failure(comptime A: type, value: anytype) fx.Exit(A, CodecError) {
    return switch (value) {
        .success => unreachable,
        .fail => |problem| .{ .fail = problem },
        .die => |problem| .{ .die = problem },
        .interrupted => .interrupted,
        .finalization_failed => |problem| .{ .finalization_failed = problem },
    };
}
const Calls = struct {
    fn now(_: *const Env) fx.WallInstant {
        return .{ .unix_nanoseconds = 1 };
    }
    fn engine(env: *const Env) *const Engine {
        return env.engine.?;
    }
    fn activation(env: *const Env) fx.WorkflowTypes.Activation {
        return env.activation.?;
    }
    fn activity(env: *const Env) fx.WorkflowTypes.ActivityId {
        return env.activity;
    }
    fn payload(env: *const Env) *const i64 {
        return &env.input;
    }
    fn body(input: anytype, env: anytype) fx.Effect(i64, void, Env) {
        env.calls.* += 1;
        return switch (env.mode) {
            .success => fx.Effect(i64, void, Env).succeed(input.*),
            .failure => fx.Effect(i64, void, Env).fail({}),
            .defect => fx.Effect(i64, void, Env).die(fx.Defect.init("recorded defect")),
            .interrupted => fx.Effect(i64, void, Env).interrupted(),
        };
    }
    fn encodeInteger(value: *const i64, env: *const Env) fx.Exit(fx.OwnedSlice(u8), CodecError) {
        const bytes = env.allocator.alloc(u8, 8) catch return .{ .die = fx.Defect.out_of_memory };
        std.mem.writeInt(i64, @ptrCast(bytes.ptr), value.*, .little);
        return .{ .success = .{ .items = bytes, .allocator = env.allocator } };
    }
    fn decodeInteger(bytes: []const u8, _: *const Env) fx.Exit(i64, CodecError) {
        if (bytes.len != 8) return .{ .fail = CodecError.pure(.decode, .invalid_value) };
        return .{ .success = std.mem.readInt(i64, @ptrCast(bytes.ptr), .little) };
    }
    fn encodeVoid(_: *const void, env: *const Env) fx.Exit(fx.OwnedSlice(u8), CodecError) {
        const bytes = env.allocator.alloc(u8, 0) catch return .{ .die = fx.Defect.out_of_memory };
        return .{ .success = .{ .items = bytes, .allocator = env.allocator } };
    }
    fn decodeVoid(bytes: []const u8, _: *const Env) fx.Exit(void, CodecError) {
        if (bytes.len != 0) return .{ .fail = CodecError.pure(.decode, .invalid_value) };
        return .{ .success = {} };
    }
    fn encode(value: *const ActivityExit, env: *const Env) fx.Exit(fx.OwnedSlice(u8), CodecError) {
        const codec = comptime fx.TypedOutcomeCodec(i64, void, Env, Env).init(encodeInteger, encodeVoid, decodeInteger, decodeVoid);
        const prepared = codec.encodeDirect(env, value);
        if (prepared != .success) return failure(fx.OwnedSlice(u8), prepared);
        var stored = prepared.success;
        defer stored.deinit();
        return fx.StoredOutcome.encode(env, &stored, limits);
    }
    fn decode(bytes: []const u8, env: *const Env) fx.Exit(ActivityExit, CodecError) {
        const parsed = fx.StoredOutcome.decode(env, bytes, limits);
        if (parsed != .success) return failure(ActivityExit, parsed);
        const codec = comptime fx.TypedOutcomeCodec(i64, void, Env, Env).init(encodeInteger, encodeVoid, decodeInteger, decodeVoid);
        const decoded = codec.decodeDirect(env, parsed.success);
        if (decoded != .success) return failure(ActivityExit, decoded);
        // Defect diagnostics borrow the native decoded owner. Retain it
        // through inspection of every replay Exit derived from that view.
        std.debug.assert(env.decoded.* == null);
        env.decoded.* = decoded.success;
        return .{ .success = env.decoded.*.?.value.clone() };
    }
};
const declared = fx.dsl.activity(.{ .name = "dsl.recorded", .version = 2, .payload = fx.JsonCodec.signed_integer, .success = fx.JsonCodec.signed_integer, .failure = fx.JsonCodec.null_value, .handler = Calls.body });
const step = W.activity(declared, void, CodecError, .@"volatile", Calls.engine, Calls.activation, Calls.activity, Calls.payload, Calls.encode, Calls.decode);
const program = W.run(step);

fn checkRecorded(mode: Mode) !void {
    comptime if (@TypeOf(program) != fx.Effect(Output, Failure, Env)) @compileError("recorded declaration must compose as the existing exact Effect");
    var calls: usize = 0;
    var decoded: ?fx.DecodedStoredOutcome(i64, void) = null;
    defer if (decoded) |*owner| owner.deinit();
    var env: Env = .{ .allocator = std.testing.allocator, .calls = &calls, .mode = mode, .decoded = &decoded };
    var memory = fx.MemoryWorkflowEngine(Env, Calls.now).init(&env);
    defer memory.deinit();
    const engine = memory.capability();
    env.engine = &engine;
    var execution_key = fx.PrimaryKey.encodeDirect(&env, "dsl-workflow", &.{.{ .text = "run" }});
    defer execution_key.deinit();
    try std.testing.expect(execution_key == .success);
    const execution: fx.WorkflowTypes.ExecutionId = .{ .encoded = execution_key.success.items };
    const definition: fx.WorkflowTypes.Definition = .{ .name = "dsl.workflow", .version = 1, .payload_schema_version = 1, .success_schema_version = 1, .error_schema_version = 1 };
    var submitting = engine.vtable.submit(engine.userdata, &env, .{ .definition = definition, .execution = execution, .encoded_payload = "17" });
    defer submitting.deinit();
    var submitted = fx.run(&env, submitting);
    defer submitted.deinit();
    try std.testing.expect(submitted == .success and submitted.success == .accepted);
    var activating = engine.vtable.activate(engine.userdata, &env, execution, "owner", .{ .lease_until = .{ .at = .{ .unix_nanoseconds = 1000 } } });
    defer activating.deinit();
    var activated = fx.run(&env, activating);
    defer activated.deinit();
    try std.testing.expect(activated == .success and activated.success == .activated);
    env.activation = activated.success.activated.activation();
    var activity_key = fx.PrimaryKey.encodeDirect(&env, "dsl-activity", &.{.{ .text = "step" }});
    defer activity_key.deinit();
    try std.testing.expect(activity_key == .success);
    env.activity = .{ .encoded = activity_key.success.items, .attempt = 1 };
    try std.testing.expectEqual(@as(usize, 0), calls);
    var first = fx.run(&env, program);
    defer first.deinit();
    if (mode == .interrupted) {
        // The existing engine leaves interrupted attempts unrecorded, so a
        // resumed invocation executes fresh work before it becomes replayable.
        try std.testing.expect(first == .interrupted);
        try std.testing.expectEqual(@as(usize, 1), calls);
        env.mode = .success;
        env.input = 99;
        var resumed = fx.run(&env, program);
        defer resumed.deinit();
        try std.testing.expect(resumed == .success and resumed.success.source == .executed);
        try std.testing.expectEqual(@as(i64, 99), resumed.success.exit.success);
        try std.testing.expectEqual(@as(usize, 2), calls);
        var replay = fx.run(&env, program);
        defer replay.deinit();
        try std.testing.expect(replay == .success and replay.success.source == .replayed);
        try std.testing.expectEqual(@as(i64, 99), replay.success.exit.success);
        try std.testing.expectEqual(@as(usize, 2), calls);
        return;
    }
    try std.testing.expect(first == .success and first.success.source == .executed);
    try checkOutcome(mode, &first.success.exit);
    try std.testing.expectEqual(@as(usize, 1), calls);
    env.input = 99;
    env.mode = if (mode == .success) .failure else .success;
    var replay = fx.run(&env, program);
    defer replay.deinit();
    try std.testing.expect(replay == .success and replay.success.source == .replayed);
    try checkOutcome(mode, &replay.success.exit);
    try std.testing.expectEqual(@as(usize, 1), calls);
}

fn checkOutcome(mode: Mode, value: *const ActivityExit) !void {
    switch (mode) {
        .success => try std.testing.expectEqual(@as(i64, 17), value.success),
        .failure => try std.testing.expect(value.* == .fail),
        .defect => {
            try std.testing.expect(value.* == .die);
            try std.testing.expectEqualStrings("recorded defect", value.die.message);
        },
        .interrupted => try std.testing.expect(value.* == .interrupted),
    }
}
test "WF-04 DECL-03 named activity executes once and replays the recorded outcome through the existing workflow engine and exact Effect boundary" {
    inline for (.{ Mode.success, Mode.failure, Mode.defect, Mode.interrupted }) |mode| try checkRecorded(mode);
}
