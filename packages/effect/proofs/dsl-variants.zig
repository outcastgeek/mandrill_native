const std = @import("std");
const fx = @import("effect");
const Issue = fx.SchemaIssue.Issue;
const Env = struct { allocator: std.mem.Allocator };
const limits: fx.SchemaBuilder.ParseLimits = .{ .max_document_bytes = 256, .max_depth = 12, .max_value_bytes = 128 };

test "CODEC-01 INF-07 inferred variant derives payloads and keeps explicit wire tags constructors projections and writer evidence" {
    const Variant = fx.dsl.variant(Env, .{ .discriminator = "kind", .payload = "data", .cases = .{
        .count = .{ .tag = "amount-v1", .codec = fx.JsonCodec.signed_integer },
        .name = .{ .tag = "label-v1", .codec = fx.JsonCodec.borrowed_string },
    } });
    comptime if (@FieldType(Variant.Decoded, "count") != i64 or @FieldType(Variant.Decoded, "name") != []const u8) @compileError("variant payloads must derive from selected codecs");
    comptime if (!Variant.supports_direct_writer) @compileError("built in writer evidence must remain available");
    const env: Env = .{ .allocator = std.testing.allocator };
    var made = Variant.make("count", 42);
    try std.testing.expectEqual(@as(i64, 42), Variant.project("count", &made).?.*);
    try std.testing.expect(Variant.project("name", &made) == null);
    var parsed = Variant.parseBytesDirect(&env, "{\"kind\":\"amount-v1\",\"data\":41}", limits);
    defer parsed.deinit();
    try std.testing.expect(parsed == .success);
    const json = parsed.success.json();
    const decoding = Variant.decode(&json);
    comptime if (@TypeOf(decoding) != fx.Effect(Variant.Decoded, Issue, Env)) @compileError("variant decoding must expose exact Effect");
    var decoded = fx.run(&env, decoding);
    defer decoded.deinit();
    try std.testing.expect(decoded == .success and decoded.success.count == 41);
    const encoding = Variant.encodeBytes(&parsed.success.value);
    comptime if (@TypeOf(encoding) != fx.Effect(fx.OwnedSlice(u8), Issue, Env)) @compileError("variant encoding must expose exact Effect");
    var encoded = fx.run(&env, encoding);
    defer encoded.deinit();
    try std.testing.expect(encoded == .success);
    try std.testing.expectEqualStrings("{\"kind\":\"amount-v1\",\"data\":41}", encoded.success.items);
    var direct = Variant.encodeBytesWith(&env, &made, .{ .max_output_bytes = 128, .max_json_depth = 8, .max_value_bytes = 64, .max_integer_digits = 16 });
    defer direct.deinit();
    try std.testing.expect(direct == .success);
    try std.testing.expectEqualStrings("{\"kind\":\"amount-v1\",\"data\":42}", direct.success.items);
    const shape = Variant.encodedNode();
    try std.testing.expect(shape.* == .tagged_union);
    try std.testing.expectEqualStrings("amount-v1", shape.tagged_union.cases[0].tag);
    var invalid = Variant.parseBytesDirect(&env, "{\"kind\":\"count\",\"data\":41}", limits);
    defer invalid.deinit();
    try std.testing.expect(invalid == .fail and invalid.fail.kind == .expected);
    try std.testing.expectEqualStrings("kind", invalid.fail.path.?[0].field.items);
}

const Child = fx.dsl.schemaRecord(Env, .{ .fields = .{
    .name = .{ .codec = fx.JsonCodec.borrowed_string, .presence = .required },
    .extras = .{ .extensions = true },
}, .policy = .{ .unknown_fields = .preserve, .duplicate_fields = .reject, .issues = .first } });
const OwnedVariant = fx.dsl.variant(Env, .{ .discriminator = "type", .payload = "payload", .cases = .{
    .record = .{ .tag = "record-v1", .codec = Child },
    .count = .{ .tag = "count-v1", .codec = fx.JsonCodec.signed_integer },
} });
const source = "{\"type\":\"record-v1\",\"payload\":{\"name\":\"retained\",\"extra\":\"also retained\"}}";

test "CODEC-01 CAP-08 LIFE-11 nested inferred record variant generates active case ownership and retained parsed views" {
    const env: Env = .{ .allocator = std.testing.allocator };
    var parsed = OwnedVariant.parseBytesDirect(&env, source, limits);
    try std.testing.expect(parsed == .success);
    var retained = try parsed.success.clone();
    parsed.deinit();
    defer retained.deinit();
    const child = OwnedVariant.project("record", &retained.value).?;
    try std.testing.expectEqualStrings("retained", Child.get("name", child).*);
    try std.testing.expectEqualStrings("also retained", Child.get("extras", child).values.get("extra").?.string);
    try std.testing.expect(OwnedVariant.project("count", &retained.value) == null);
    var encoded = fx.run(&env, OwnedVariant.encodeBytes(&retained.value));
    defer encoded.deinit();
    try std.testing.expect(encoded == .success);
    try std.testing.expectEqualStrings(source, encoded.success.items);
    var count = OwnedVariant.make("count", 7);
    defer count.deinit();
    try std.testing.expectEqual(@as(i64, 7), OwnedVariant.project("count", &count).?.*);
}

test "CODEC-01 CODEC-06 LIFE-07 inferred owned variant releases nested fields and parsed clones at every allocation failure" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            const env: Env = .{ .allocator = allocator };
            var parsed = OwnedVariant.parseBytesDirect(&env, source, limits);
            defer parsed.deinit();
            if (parsed == .die and parsed.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expect(parsed == .success);
            var retained = try parsed.success.clone();
            defer retained.deinit();
            var encoded = fx.run(&env, OwnedVariant.encodeBytes(&retained.value));
            defer encoded.deinit();
            if (encoded == .die and encoded.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expect(encoded == .success);
            try std.testing.expectEqualStrings(source, encoded.success.items);
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

test "CODEC-01 existing custom owned tagged unions preserve their direct payload layout through legacy and inferred declarations" {
    const Existing = union(enum) {
        pub const effect_owned_payload = true;
        record: Child.Decoded,
        count: i64,
        pub fn deinit(self: *@This()) void {
            if (self.* == .record) self.record.deinit();
        }
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            return switch (self.*) {
                .record => |value| .{ .record = try value.clone() },
                .count => |value| .{ .count = value },
            };
        }
    };
    const Legacy = fx.VariantSchema.Variant(Existing, Env, "type", "payload", .{
        fx.VariantSchema.nestedCase("record", "record-v1", Child),
        fx.VariantSchema.taggedCase("count", "count-v1", fx.JsonCodec.signed_integer),
    });
    const Inferred = fx.dsl.variant(Env, .{ .value = Existing, .discriminator = "type", .payload = "payload", .cases = .{
        .record = .{ .tag = "record-v1", .codec = Child },
        .count = .{ .tag = "count-v1", .codec = fx.JsonCodec.signed_integer },
    } });
    comptime if (Inferred.Decoded != Existing) @compileError("an authoritative existing owner must retain its exact type");
    inline for (.{ Legacy, Inferred }) |Variant| {
        const env: Env = .{ .allocator = std.testing.allocator };
        var parsed = Variant.parseBytesDirect(&env, source, limits);
        defer parsed.deinit();
        try std.testing.expect(parsed == .success and parsed.success.value == .record);
        try std.testing.expectEqualStrings("retained", Child.get("name", Variant.project("record", &parsed.success.value).?).*);
        var encoded = fx.run(&env, Variant.encodeBytes(&parsed.success.value));
        defer encoded.deinit();
        try std.testing.expect(encoded == .success);
        try std.testing.expectEqualStrings(source, encoded.success.items);
        var count = Variant.make("count", 7);
        defer count.deinit();
        try std.testing.expect(count == .count and count.count == 7);
    }
}
