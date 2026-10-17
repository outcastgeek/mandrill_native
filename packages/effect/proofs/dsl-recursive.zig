const std = @import("std");
const fx = @import("effect");
const Issue = fx.SchemaIssue.Issue;
const Env = struct { allocator: std.mem.Allocator, defaults: *usize };
const Node = struct { value: i64, next: ?fx.dsl.RecursiveBox(@This()) };
fn defaultValue(env: anytype) i64 {
    env.defaults.* += 1;
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    if (comptime @hasField(R, "cancel_on_default")) {
        if (env.cancel_on_default) env.cancellation.request();
    }
    return 99;
}
fn schema(comptime Context: type) type {
    return fx.dsl.schemaRecord(Context.Environment, .{ .value = Context.Value, .fields = .{
        .value = .{ .codec = fx.JsonCodec.signed_integer, .presence = .defaulted, .default = defaultValue },
        .next = .{ .codec = Context.Schema, .presence = .optional },
    }, .policy = .{ .unknown_fields = .reject, .duplicate_fields = .reject, .issues = .first } });
}
const Recursive = fx.dsl.recursiveSchema(Env, .{ .value = Node, .max_depth = 3, .reference = "node-v1", .schema = schema });
const limits: fx.SchemaBuilder.ParseLimits = .{ .max_document_bytes = 256, .max_depth = 16, .max_value_bytes = 64 };
const source = "{\"value\":1,\"next\":{\"value\":2}}";

test "CODEC-04 COMP-03 inferred recursive schema is lazy and derives composite node ownership without application cleanup methods" {
    comptime if (@hasDecl(Node, "deinit") or @hasDecl(Node, "clone")) @compileError("the application node must not supply mechanical ownership methods");
    comptime if (Recursive.Decoded != fx.dsl.RecursiveBox(Node)) @compileError("recursive schema must retain its exact generated node owner");
    var defaults: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .defaults = &defaults };
    const input: std.json.Value = .{ .object = .{} };
    var unused = Recursive.decode(&input);
    comptime if (@TypeOf(unused) != fx.Effect(Recursive.Decoded, Issue, Env)) @compileError("recursive decoding must expose exact Effect");
    unused.deinit();
    try std.testing.expectEqual(@as(usize, 0), defaults);
    var result = fx.run(&env, Recursive.decode(&input));
    defer result.deinit();
    try std.testing.expect(result == .success and result.success.value.value == 99 and result.success.value.next == null);
    try std.testing.expectEqual(@as(usize, 1), defaults);
    try std.testing.expectEqualStrings("node-v1", Recursive.encodedNode().reference);
}

test "CODEC-04 CAP-08 LIFE-11 recursive inferred boxes clone nested owners and encoded bytes survive node teardown" {
    var defaults: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .defaults = &defaults };
    var parsed = Recursive.parseBytesDirect(&env, source, limits);
    try std.testing.expect(parsed == .success);
    var retained = try parsed.success.clone();
    parsed.deinit();
    try std.testing.expectEqual(@as(i64, 1), retained.value.value.value);
    try std.testing.expectEqual(@as(i64, 2), retained.value.value.next.?.value.value);
    try std.testing.expect(retained.value.value.next.?.value.next == null);
    const encoding = Recursive.encodeBytes(&retained.value);
    comptime if (@TypeOf(encoding) != fx.Effect(fx.OwnedSlice(u8), Issue, Env)) @compileError("recursive encoding must expose exact Effect");
    var encoded = fx.run(&env, encoding);
    retained.deinit();
    defer encoded.deinit();
    try std.testing.expect(encoded == .success);
    try std.testing.expectEqualStrings(source, encoded.success.items);
}

test "CODEC-04 CODEC-07 recursive inferred schema preserves invocation local depth policy and owned issue paths" {
    const Bounded = fx.dsl.recursiveSchema(Env, .{ .value = Node, .max_depth = 2, .reference = "bounded-node-v1", .schema = schema });
    var defaults: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .defaults = &defaults };
    var too_deep = Bounded.parseBytesDirect(&env, "{\"value\":1,\"next\":{\"value\":2,\"next\":{\"value\":3}}}", limits);
    defer too_deep.deinit();
    try std.testing.expect(too_deep == .fail and too_deep.fail.kind == .depth_limit);
    try std.testing.expectEqual(@as(usize, 2), too_deep.fail.path.?.len);
    for (too_deep.fail.path.?) |element| try std.testing.expectEqualStrings("next", element.field.items);
    var valid = Bounded.parseBytesDirect(&env, source, limits);
    defer valid.deinit();
    try std.testing.expect(valid == .success and valid.success.value.value.next.?.value.value == 2);
}

test "CODEC-04 CODEC-06 LIFE-07 recursive generated node owners roll back parsing cloning and encoding at every allocation failure" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            var defaults: usize = 0;
            const env: Env = .{ .allocator = allocator, .defaults = &defaults };
            var parsed = Recursive.parseBytesDirect(&env, source, limits);
            defer parsed.deinit();
            if (parsed == .die and parsed.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expect(parsed == .success);
            var retained = try parsed.success.clone();
            defer retained.deinit();
            var encoded = fx.run(&env, Recursive.encodeBytes(&retained.value));
            defer encoded.deinit();
            if (encoded == .die and encoded.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expect(encoded == .success);
            try std.testing.expectEqualStrings(source, encoded.success.items);
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

test "CODEC-06 LIFE-07 late cancellation discards a completed inferred recursive node and every owned child" {
    const Runtime = struct { allocator: std.mem.Allocator, defaults: *usize, cancellation: *fx.Cancellation, cancel_on_default: bool };
    const Described = fx.dsl.recursiveSchema(Runtime, .{ .value = Node, .max_depth = 3, .reference = "cancelled-node-v1", .schema = schema });
    var defaults: usize = 0;
    var cancellation: fx.Cancellation = .{};
    var allocator = std.testing.FailingAllocator.init(std.testing.allocator, .{});
    const env: Runtime = .{ .allocator = allocator.allocator(), .defaults = &defaults, .cancellation = &cancellation, .cancel_on_default = true };
    var object: std.json.ObjectMap = .{};
    defer object.deinit(std.testing.allocator);
    try object.put(std.testing.allocator, "value", .{ .integer = 1 });
    try object.put(std.testing.allocator, "next", .{ .object = .{} });
    const input: std.json.Value = .{ .object = object };
    var result = fx.run(&env, Described.decode(&input));
    defer result.deinit();
    try std.testing.expect(result == .interrupted);
    try std.testing.expectEqual(@as(usize, 1), defaults);
    try std.testing.expect(allocator.allocated_bytes >= 2 * @sizeOf(Node));
    try std.testing.expectEqual(allocator.allocated_bytes, allocator.freed_bytes);
}
