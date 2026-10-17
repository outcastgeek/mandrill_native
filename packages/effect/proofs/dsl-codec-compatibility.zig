const std = @import("std");
const fx = @import("effect");
const Issue = fx.SchemaIssue.Issue;
const Env = struct { allocator: std.mem.Allocator };
const Bits = union { word: i64, bits: u64 };
const Roles = struct {
    fn decode(input: *const std.json.Value) fx.Result(Bits, Issue) {
        return switch (fx.JsonCodec.signed_integer.decodeDirect(input)) {
            .ok => |value| .{ .ok = .{ .word = value } },
            .err => |issue| .{ .err = issue },
        };
    }
    fn encode(value: *const Bits) fx.Result(std.json.Value, Issue) {
        return .{ .ok = .{ .integer = value.word } };
    }
};
const codec = fx.dsl.codec(.{ .kind = .pure, .decode = Roles.decode, .encode = Roles.encode, .shape = fx.JsonCodec.signed_integer.encoded_shape });
const Record = fx.dsl.schemaRecord(Env, .{ .fields = .{ .raw = .{ .codec = codec, .presence = .required } }, .policy = .{ .unknown_fields = .reject, .duplicate_fields = .reject, .issues = .first } });
const Variant = fx.dsl.variant(Env, .{ .discriminator = "kind", .payload = "data", .cases = .{ .raw = .{ .tag = "bits-v1", .codec = codec } } });
const Object = struct {
    pub const Representation = std.json.Value;
    pub const Builder = struct { allocator: std.mem.Allocator, values: std.json.ObjectMap = .{} };
    pub fn count(input: *const Representation) usize {
        return if (input.* == .object) input.object.count() else 0;
    }
    pub fn nameAt(input: *const Representation, index: usize) []const u8 {
        return input.object.keys()[index];
    }
    pub fn find(input: *const Representation, name: []const u8) ?*const Representation {
        return if (input.* == .object) input.object.getPtr(name) else null;
    }
    pub fn init(env: *const Env, _: usize) fx.Exit(Builder, Issue) {
        return .{ .success = .{ .allocator = env.allocator } };
    }
    pub fn put(_: *const Env, builder: *Builder, name: []const u8, value: *const Representation) fx.Exit(void, Issue) {
        builder.values.put(builder.allocator, name, value.*) catch return .{ .die = fx.Defect.out_of_memory };
        return .{ .success = {} };
    }
    pub fn finish(_: *const Env, builder: *Builder) fx.Exit(Representation, Issue) {
        const values = builder.values;
        builder.values = .{};
        return .{ .success = .{ .object = values } };
    }
    pub fn discard(builder: *Builder) void {
        builder.values.deinit(builder.allocator);
    }
};
const Native = fx.dsl.nativeRecord(Object, .{ .decode_env = Env, .fields = .{ .raw = .{ .codec = codec, .presence = .required } }, .reject_unknown = true });
const Preserving = fx.dsl.schemaRecord(Env, .{ .fields = .{
    .raw = .{ .codec = codec, .presence = .required },
    .extras = .{ .extensions = true },
}, .policy = .{ .unknown_fields = .preserve, .duplicate_fields = .reject, .issues = .first } });
const preserved_source = "{\"raw\":41,\"extra\":\"retained\"}";

test "CODEC-03 inferred pure codec retains ordinary non owning untagged application values" {
    comptime if (@TypeOf(codec).Decoded != Bits) @compileError("non owning codec channels must retain their original type");
    var decoded = codec.decodeDirect(&.{ .integer = 41 });
    defer decoded.deinit();
    try std.testing.expect(decoded == .ok and decoded.ok.word == 41);
    var encoded = codec.encodeDirect(&decoded.ok);
    defer encoded.deinit();
    try std.testing.expect(encoded == .ok and encoded.ok.integer == 41);
    const lifted = comptime codec.asCodec();
    const described = lifted.decode(&.{ .integer = 42 });
    comptime if (@TypeOf(described) != fx.Effect(Bits, Issue, void)) @compileError("native untagged value must remain an ordinary Effect channel");
    var result = fx.run(&@as(void, {}), described);
    defer result.deinit();
    try std.testing.expect(result == .success and result.success.word == 42);
}

test "CODEC-01 inferred record retains a pure codec untagged field without generating an owner" {
    const env: Env = .{ .allocator = std.testing.allocator };
    var object: std.json.ObjectMap = .{};
    defer object.deinit(env.allocator);
    try object.put(env.allocator, "raw", .{ .integer = 41 });
    const input: std.json.Value = .{ .object = object };
    var result = fx.run(&env, Record.decode(&input));
    defer result.deinit();
    try std.testing.expect(result == .success and Record.get("raw", &result.success).word == 41);
    var encoded = fx.run(&env, Record.encodeBytes(&result.success));
    defer encoded.deinit();
    try std.testing.expect(encoded == .success);
    try std.testing.expectEqualStrings("{\"raw\":41}", encoded.success.items);
}

test "CODEC-01 inferred tagged variant can contain an ordinary non owning untagged payload" {
    const env: Env = .{ .allocator = std.testing.allocator };
    var value = Variant.make("raw", .{ .word = 41 });
    try std.testing.expectEqual(@as(i64, 41), Variant.project("raw", &value).?.word);
    var encoded = fx.run(&env, Variant.encodeBytes(&value));
    defer encoded.deinit();
    try std.testing.expect(encoded == .success);
    try std.testing.expectEqualStrings("{\"kind\":\"bits-v1\",\"data\":41}", encoded.success.items);
    var parsed = Variant.parseBytesDirect(&env, encoded.success.items, .{ .max_document_bytes = 128, .max_depth = 4, .max_value_bytes = 64 });
    defer parsed.deinit();
    try std.testing.expect(parsed == .success and parsed.success.value.raw.word == 41);
}

test "CODEC-02 inferred native record retains ordinary non owning untagged field types" {
    const env: Env = .{ .allocator = std.testing.allocator };
    var object: std.json.ObjectMap = .{};
    defer object.deinit(env.allocator);
    try object.put(env.allocator, "raw", .{ .integer = 41 });
    const input: std.json.Value = .{ .object = object };
    var decoded = fx.run(&env, Native.decode(&input));
    defer decoded.deinit();
    try std.testing.expect(decoded == .success and decoded.success.raw.word == 41);
    var encoded = fx.run(&env, Native.encode(&decoded.success));
    defer encoded.deinit();
    try std.testing.expect(encoded == .success);
    defer encoded.success.object.deinit(env.allocator);
    try std.testing.expectEqual(@as(i64, 41), encoded.success.object.get("raw").?.integer);
}

test "CODEC-01 CAP-08 LIFE-11 inferred owner mixes an untagged value with owned preserved fields across clone and final cleanup" {
    const env: Env = .{ .allocator = std.testing.allocator };
    var parsed = Preserving.parseBytesDirect(&env, preserved_source, .{ .max_document_bytes = 128, .max_depth = 4, .max_value_bytes = 64 });
    try std.testing.expect(parsed == .success);
    var retained = try parsed.success.clone();
    parsed.deinit();
    try std.testing.expectEqual(@as(i64, 41), Preserving.get("raw", &retained.value).word);
    try std.testing.expectEqualStrings("retained", Preserving.get("extras", &retained.value).values.get("extra").?.string);
    var encoded = fx.run(&env, Preserving.encodeBytes(&retained.value));
    retained.deinit();
    defer encoded.deinit();
    try std.testing.expect(encoded == .success);
    try std.testing.expectEqualStrings(preserved_source, encoded.success.items);
}

test "CODEC-01 CODEC-06 LIFE-07 mixed untagged and owned inferred fields clean partial clones at every allocation failure" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            const env: Env = .{ .allocator = allocator };
            var parsed = Preserving.parseBytesDirect(&env, preserved_source, .{ .max_document_bytes = 128, .max_depth = 4, .max_value_bytes = 64 });
            defer parsed.deinit();
            if (parsed == .die and parsed.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expect(parsed == .success);
            var retained = try parsed.success.clone();
            defer retained.deinit();
            try std.testing.expectEqual(@as(i64, 41), Preserving.get("raw", &retained.value).word);
            var encoded = fx.run(&env, Preserving.encodeBytes(&retained.value));
            defer encoded.deinit();
            if (encoded == .die and encoded.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expect(encoded == .success);
            try std.testing.expectEqualStrings(preserved_source, encoded.success.items);
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}
