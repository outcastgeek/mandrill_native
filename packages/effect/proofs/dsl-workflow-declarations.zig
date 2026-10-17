const std = @import("std");
const fx = @import("effect");
const R = struct { allocator: std.mem.Allocator, input: i64, bias: i64, calls: *usize };
const W = fx.WorkflowBuilder(void, R);
const integer = fx.JsonCodec.signed_integer;
const nothing = fx.JsonCodec.null_value;
const Functions = struct {
    fn payload(env: *const R) *const i64 {
        return &env.input;
    }
    fn ordinary(input: anytype, env: anytype) fx.Effect(i64, void, R) {
        env.calls.* += 1;
        return fx.Effect(i64, void, R).succeed(input.* + env.bias);
    }
    fn durable(input: anytype, env: anytype) W.Step(i64) {
        env.calls.* += 1;
        return W.succeed(input.* + env.bias);
    }
};
const workflow_description: fx.WorkflowTypes.Definition = .{ .name = "dsl.workflow", .version = 2, .payload_schema_version = 3, .success_schema_version = 4, .error_schema_version = 5 };
test "DECL-01 INF-08 inferred activity declaration preserves the completed native handler and ordinary Effect boundary" {
    const described = comptime fx.dsl.activity(.{ .name = "dsl.activity", .version = 2, .payload = integer, .success = integer, .failure = nothing, .handler = Functions.ordinary });
    const old = fx.WorkflowActivityBuilder(void, R).begin("dsl.activity", 2).payload(integer).success(integer).failure(nothing).handledBy(struct {
        fn invoke(input: *const i64, env: *const R) fx.Effect(i64, void, R) {
            return Functions.ordinary(input, env);
        }
    }.invoke);
    comptime if (@TypeOf(described) != @TypeOf(old)) @compileError("activity declaration must preserve completed native handler identity");
    var calls: usize = 0;
    const env: R = .{ .allocator = std.testing.allocator, .input = 11, .bias = 7, .calls = &calls };
    const operation = comptime described.run(Functions.payload);
    comptime if (@TypeOf(operation) != fx.Effect(i64, void, R)) @compileError("activity execution must remain exact Effect");
    try std.testing.expectEqual(@as(usize, 0), calls);
    var result = fx.run(&env, operation);
    defer result.deinit();
    try std.testing.expectEqual(@as(i64, 18), result.success);
    try std.testing.expectEqual(@as(usize, 1), calls);
}
test "WF-01 DECL-01 INF-08 named workflow requires the durable Step handler and retains every explicit definition and schema version" {
    const described = comptime fx.dsl.workflow(.{ .description = workflow_description, .payload = integer, .success = integer, .failure = nothing, .handler = Functions.durable });
    const old = fx.WorkflowBuilder(void, R).begin(workflow_description).payload(integer).success(integer).failure(nothing).handledBy(Functions.durable);
    comptime if (@TypeOf(described) != @TypeOf(old)) @compileError("workflow declaration must preserve completed native handler identity");
    try std.testing.expectEqualDeep(workflow_description, described.definition.description);
    var calls: usize = 0;
    const env: R = .{ .allocator = std.testing.allocator, .input = 17, .bias = 5, .calls = &calls };
    const operation = comptime described.run(Functions.payload);
    comptime if (@TypeOf(operation) != fx.Effect(i64, void, R)) @compileError("controlled workflow execution must remain exact Effect");
    try std.testing.expectEqual(@as(usize, 0), calls);
    var result = fx.run(&env, operation);
    defer result.deinit();
    try std.testing.expectEqual(@as(i64, 22), result.success);
    try std.testing.expectEqual(@as(usize, 1), calls);
}

fn EnvOf(comptime T: type) type {
    return @typeInfo(T).pointer.child;
}
const Generic = struct {
    fn durable(input: anytype, env: anytype) fx.WorkflowBuilder(void, EnvOf(@TypeOf(env))).Step(i64) {
        env.calls.* += 1;
        return fx.WorkflowBuilder(void, EnvOf(@TypeOf(env))).succeed(input.* + env.bias);
    }
};
test "INF-13 ROLE-02 WF-01 workflow namespace supplies only an otherwise absent environment anchor for a generic durable handler" {
    const Declaration = struct {
        pub const description = workflow_description;
        pub const payload = integer;
        pub const success = integer;
        pub const failure = nothing;
        pub const handler = Generic.durable;
        pub const environment = R;
    };
    const described = comptime fx.dsl.workflow(Declaration);
    var calls: usize = 0;
    const env: R = .{ .allocator = std.testing.allocator, .input = 31, .bias = 5, .calls = &calls };
    var result = fx.run(&env, comptime described.run(Functions.payload));
    defer result.deinit();
    try std.testing.expectEqual(@as(i64, 36), result.success);
    try std.testing.expectEqual(@as(usize, 1), calls);
}

test "DECL-03 WF-01 anonymous workflow description normalizes into the native definition without a caller type annotation" {
    const described = comptime fx.dsl.workflow(.{
        .description = .{ .name = "dsl.workflow", .version = 2, .payload_schema_version = 3, .success_schema_version = 4, .error_schema_version = 5 },
        .payload = integer,
        .success = integer,
        .failure = nothing,
        .handler = Functions.durable,
    });
    try std.testing.expectEqualDeep(workflow_description, described.definition.description);
}
