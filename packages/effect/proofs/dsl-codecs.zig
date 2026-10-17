const std = @import("std");
const fx = @import("effect");
const Issue = fx.SchemaIssue.Issue;
const E = enum { invalid };
const TransformEnv = struct { allocator: std.mem.Allocator };
const OwningTransform = struct {
    var live: usize = 0;
    const Token = struct {
        pub const effect_owned_payload = true;
        pub fn deinit(_: *@This()) void {
            live -= 1;
        }
    };
    const Value = struct { token: Token, number: i64 };
    fn forward(number: i64) fx.Result(Value, Issue) {
        if (number < 0) return .{ .err = Issue.pure(.decode, .invalid_value) };
        live += 1;
        return .{ .ok = .{ .token = .{}, .number = number } };
    }
    fn backward(value: *const Value) fx.Result(i64, Issue) {
        std.debug.assert(live == 1);
        return .{ .ok = value.number };
    }
    fn limits(_: *const TransformEnv) fx.JsonWriter.Limits {
        return .{ .max_output_bytes = 64, .max_json_depth = 4, .max_value_bytes = 32, .max_integer_digits = 16 };
    }
};

test "CODEC-04 CODEC-07 inferred pure transformation derives owning composite output and preserves native validation writer and shape evidence" {
    const codec = comptime fx.dsl.transformCodec(fx.JsonCodec.signed_integer, .{ .forward = OwningTransform.forward, .backward = OwningTransform.backward });
    comptime if (@TypeOf(codec).Decoded != fx.dsl.ownership.Capture(OwningTransform.Value)) @compileError("owning transformed result must receive generated composite cleanup");
    try std.testing.expectEqual(@as(usize, 0), OwningTransform.live);
    var decoded = codec.decodeDirect(&.{ .integer = 17 });
    errdefer decoded.deinit();
    try std.testing.expect(decoded == .ok);
    try std.testing.expectEqual(@as(usize, 1), OwningTransform.live);
    var encoded = codec.encodeDirect(&decoded.ok);
    try std.testing.expectEqual(@as(i64, 17), encoded.ok.integer);
    encoded.deinit();
    var checked = codec.validateDirect(&decoded.ok);
    try std.testing.expect(checked == .ok);
    checked.deinit();
    var rejected = codec.decodeDirect(&.{ .integer = -1 });
    try std.testing.expect(rejected == .err);
    rejected.deinit();
    try std.testing.expectEqual(@as(usize, 1), OwningTransform.live);
    const env: TransformEnv = .{ .allocator = std.testing.allocator };
    var program = fx.JsonCodec.requireWriter(codec).writeWith(TransformEnv, &decoded.ok, OwningTransform.limits);
    var written = fx.run(&env, program);
    errdefer written.deinit();
    program.deinit();
    try std.testing.expectEqualStrings("17", written.success.items);
    try std.testing.expect(codec.encoded_shape.?.* == .constrained);
    try std.testing.expectEqual(fx.JsonCodec.signed_integer.encoded_shape.?, codec.encoded_shape.?.constrained.base);
    try std.testing.expectEqualStrings("transform", codec.encoded_shape.?.constrained.rules[0].check);
    decoded.deinit();
    decoded = .{ .err = Issue.pure(.decode, .invalid_value) };
    try std.testing.expectEqual(@as(usize, 0), OwningTransform.live);
    // The native writer copied the representation before its owning value died.
    try std.testing.expectEqualStrings("17", written.success.items);
    written.deinit();
    written = .interrupted;
}

test "CODEC-04 CODEC-07 LIFE-07 pure transformed owning output survives lifted decode and every native writer allocation failure without business Clone" {
    const codec = comptime fx.dsl.transformCodec(fx.JsonCodec.signed_integer, .{ .forward = OwningTransform.forward, .backward = OwningTransform.backward });
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            try std.testing.expectEqual(@as(usize, 0), OwningTransform.live);
            defer std.debug.assert(OwningTransform.live == 0);
            const unit: void = {};
            const lifted = comptime codec.asCodec();
            var decoded = fx.run(&unit, lifted.decode(&.{ .integer = 17 }));
            defer decoded.deinit();
            try std.testing.expect(decoded == .success);
            const env: TransformEnv = .{ .allocator = allocator };
            var program = fx.JsonCodec.requireWriter(codec).writeWith(TransformEnv, &decoded.success, OwningTransform.limits);
            defer program.deinit();
            var written = fx.run(&env, program);
            defer written.deinit();
            if (written == .die and written.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expectEqualStrings("17", written.success.items);
            try std.testing.expectEqual(@as(usize, 1), OwningTransform.live);
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

test "CODEC-03 INF-07 paired pure codec roles infer both value directions and preserve explicit writer evidence" {
    const Roles = struct {
        fn decode(input: *const std.json.Value) fx.Result(i64, Issue) {
            return fx.JsonCodec.signed_integer.decodeDirect(input);
        }
        fn encode(value: *const i64) fx.Result(std.json.Value, Issue) {
            return fx.JsonCodec.signed_integer.encodeDirect(value);
        }
        fn write(value: *const i64, output: *fx.JsonWriter.Writer) fx.Exit(void, Issue) {
            return fx.JsonCodec.signed_integer.writeJson(value, output);
        }
    };
    const codec = comptime fx.dsl.codec(.{ .kind = .pure, .encode = Roles.encode, .writer = Roles.write, .decode = Roles.decode, .shape = fx.JsonCodec.signed_integer.encoded_shape });
    comptime if (@TypeOf(codec) != fx.PureCodec(i64, std.json.Value)) @compileError("pure declaration must reuse the existing codec");
    var decoded = codec.decodeDirect(&.{ .integer = 41 });
    defer decoded.deinit();
    try std.testing.expect(decoded == .ok and decoded.ok == 41);
    var encoded = codec.encodeDirect(&decoded.ok);
    defer encoded.deinit();
    try std.testing.expect(encoded == .ok and encoded.ok.integer == 41);
    const Env = struct { allocator: std.mem.Allocator };
    const env: Env = .{ .allocator = std.testing.allocator };
    var writer = fx.JsonWriter.Writer.init(&env, .{ .max_output_bytes = 64, .max_json_depth = 4, .max_value_bytes = 32, .max_integer_digits = 16 });
    defer writer.deinit();
    var written = codec.writeJson(&decoded.ok, &writer);
    defer written.deinit();
    try std.testing.expect(written == .success);
    var bytes = try writer.finish();
    defer bytes.deinit();
    try std.testing.expectEqualStrings("41", bytes.items);
}

test "CODEC-03 INF-07 contextual codec keeps independent decode encode environments and exact Effect channels" {
    const DecodeEnv = struct { added: u64 };
    const EncodeEnv = struct { subtracted: u64 };
    const Roles = struct {
        fn decode(input: *const i64) fx.Effect(u64, E, DecodeEnv) {
            return fx.Effect(u64, E, DecodeEnv).fromBorrowed(i64, input, struct {
                fn run(value: *const i64, env: *const DecodeEnv) fx.Exit(u64, E) {
                    if (value.* < 0) return .{ .fail = .invalid };
                    return .{ .success = @as(u64, @intCast(value.*)) + env.added };
                }
            }.run);
        }
        fn encode(value: *const u64) fx.Effect(i64, E, EncodeEnv) {
            return fx.Effect(i64, E, EncodeEnv).fromBorrowed(u64, value, struct {
                fn run(input: *const u64, env: *const EncodeEnv) fx.Exit(i64, E) {
                    if (input.* < env.subtracted) return .{ .fail = .invalid };
                    return .{ .success = @intCast(input.* - env.subtracted) };
                }
            }.run);
        }
    };
    const codec = comptime fx.dsl.codec(.{ .kind = .contextual, .decode = Roles.decode, .encode = Roles.encode });
    comptime if (@TypeOf(codec) != fx.DomainCodec.DomainCodec(u64, i64, E, DecodeEnv, EncodeEnv)) @compileError("codec must preserve both original environments");
    const de: DecodeEnv = .{ .added = 2 };
    const en: EncodeEnv = .{ .subtracted = 3 };
    const input: i64 = 40;
    const program = codec.decode(&input);
    comptime if (@TypeOf(program) != fx.Effect(u64, E, DecodeEnv)) @compileError("decode must be exact Effect");
    var decoded = fx.run(&de, program);
    defer decoded.deinit();
    try std.testing.expect(decoded == .success and decoded.success == 42);
    var encoded = fx.run(&en, codec.encode(&decoded.success));
    defer encoded.deinit();
    try std.testing.expect(encoded == .success and encoded.success == 39);
    var invalid = fx.run(&de, codec.decode(&@as(i64, -1)));
    defer invalid.deinit();
    try std.testing.expect(invalid == .fail and invalid.fail == .invalid);
}

test "CODEC-03 CAP-08 pure codec derives composite success error ownership without outer destructors or Clone" {
    const Owner = struct {
        pub const effect_owned_payload = true;
        live: *usize,
        pub fn deinit(self: *@This()) void {
            self.live.* -= 1;
        }
    };
    const Value = struct { token: Owner };
    const Rep = struct { live: *usize, fail: bool };
    const Roles = struct {
        fn decode(input: *const Rep) fx.Result(Value, Value) {
            input.live.* += 1;
            const value: Value = .{ .token = .{ .live = input.live } };
            return if (input.fail) .{ .err = value } else .{ .ok = value };
        }
        fn encode(value: *const Value) fx.Result(Rep, Value) {
            return .{ .ok = .{ .live = value.token.live, .fail = false } };
        }
    };
    const codec = comptime fx.dsl.codec(.{ .kind = .pure, .decode = Roles.decode, .encode = Roles.encode });
    comptime if (@TypeOf(codec).Decoded != fx.dsl.ownership.Capture(Value)) @compileError("pure native result must receive generated ownership before lifting into Effect");
    inline for (.{ false, true }) |fail| {
        var live: usize = 0;
        const input: Rep = .{ .live = &live, .fail = fail };
        var result = codec.decodeDirect(&input);
        try std.testing.expect(if (fail) result == .err else result == .ok);
        try std.testing.expectEqual(@as(usize, 1), live);
        result.deinit();
        try std.testing.expectEqual(@as(usize, 0), live);
        var executed = fx.run(&@as(void, {}), codec.decode(&input));
        try std.testing.expect(if (fail) executed == .fail else executed == .success);
        try std.testing.expectEqual(@as(usize, 1), live);
        executed.deinit();
        try std.testing.expectEqual(@as(usize, 0), live);
    }
}

test "INF-12 CODEC-03 paired generic codec callbacks specialize from opposite directional evidence" {
    const Roles = struct {
        fn decode(input: anytype) fx.Result(u16, E) {
            return .{ .ok = input.* + 1 };
        }
        fn encode(value: anytype) fx.Result(u8, E) {
            return .{ .ok = @intCast(value.* - 1) };
        }
    };
    const codec = comptime fx.dsl.codec(.{ .kind = .pure, .decode = Roles.decode, .encode = Roles.encode, .representation = u8 });
    var result = codec.decodeDirect(&@as(u8, 8));
    defer result.deinit();
    try std.testing.expect(result == .ok and result.ok == 9);
}

test "CODEC-03 CODEC-04 mixed codec directions preserve explicit one-sided purity and service requirements" {
    const Env = struct { added: u8 };
    const Roles = struct {
        fn decodePure(input: *const u8) fx.Result(u16, E) {
            return .{ .ok = @as(u16, input.*) + 1 };
        }
        fn encodePure(value: *const u16) fx.Result(u8, E) {
            return .{ .ok = @intCast(value.* * 2) };
        }
        fn decodeContext(input: *const u8) fx.Effect(u16, E, Env) {
            return fx.Effect(u16, E, Env).fromBorrowed(u8, input, struct {
                fn run(value: *const u8, env: *const Env) fx.Exit(u16, E) {
                    return .{ .success = @as(u16, value.*) + env.added };
                }
            }.run);
        }
        fn encodeContext(input: *const u16) fx.Effect(u8, E, Env) {
            return fx.Effect(u8, E, Env).fromBorrowed(u16, input, struct {
                fn run(value: *const u16, env: *const Env) fx.Exit(u8, E) {
                    return .{ .success = @intCast(value.* + env.added) };
                }
            }.run);
        }
    };
    const decode_pure = comptime fx.dsl.codec(.{ .kind = .pure_decode, .decode = Roles.decodePure, .encode = Roles.encodeContext });
    const encode_pure = comptime fx.dsl.codec(.{ .kind = .pure_encode, .decode = Roles.decodeContext, .encode = Roles.encodePure });
    comptime if (!@TypeOf(decode_pure).pure_decoding or @TypeOf(decode_pure).pure_encoding or @TypeOf(decode_pure).DecodingEnvironment != void or @TypeOf(decode_pure).EncodingEnvironment != Env) @compileError("decode purity must not erase encoding services");
    comptime if (@TypeOf(encode_pure).pure_decoding or !@TypeOf(encode_pure).pure_encoding or @TypeOf(encode_pure).DecodingEnvironment != Env or @TypeOf(encode_pure).EncodingEnvironment != void) @compileError("encode purity must not erase decoding services");
    const env: Env = .{ .added = 1 };
    var direct = decode_pure.decodeDirect(&@as(u8, 4));
    defer direct.deinit();
    var encoded = fx.run(&env, decode_pure.encode(&direct.ok));
    defer encoded.deinit();
    try std.testing.expect(encoded == .success and encoded.success == 6);
    var decoded = fx.run(&env, encode_pure.decode(&@as(u8, 4)));
    defer decoded.deinit();
    var written = encode_pure.encodeDirect(&decoded.success);
    defer written.deinit();
    try std.testing.expect(written == .ok and written.ok == 10);
}

test "CODEC-04 CODEC-07 inferred pure transform retains writer evidence and schema field failure context" {
    const Value = struct { amount: u16, backwards: ?*usize = null };
    const Roles = struct {
        fn forward(value: i64) fx.Result(Value, Issue) {
            if (value < 0 or value > 254) return .{ .err = Issue.pure(.decode, .{ .custom = "amount-range" }) };
            return .{ .ok = .{ .amount = @intCast(value + 1) } };
        }
        fn backward(value: *const Value) fx.Result(i64, Issue) {
            if (value.backwards) |count| count.* += 1;
            if (value.amount == 0) return .{ .err = Issue.pure(.encode, .{ .custom = "amount-range" }) };
            return .{ .ok = value.amount - 1 };
        }
    };
    const transformed = comptime fx.dsl.transformCodec(fx.JsonCodec.signed_integer, .{ .forward = Roles.forward, .backward = Roles.backward });
    comptime if (@TypeOf(transformed).Decoded != Value or transformed.writer == null) @compileError("transform must infer its application value and retain actual writer evidence");
    const Env = struct { allocator: std.mem.Allocator };
    const env: Env = .{ .allocator = std.testing.allocator };
    var decoded = transformed.decodeDirect(&.{ .integer = 41 });
    defer decoded.deinit();
    try std.testing.expect(decoded == .ok and decoded.ok.amount == 42);
    var calls: usize = 0;
    const input: Value = .{ .amount = 42, .backwards = &calls };
    var encoded = transformed.encodeDirect(&input);
    defer encoded.deinit();
    try std.testing.expect(encoded == .ok and encoded.ok.integer == 41);
    try std.testing.expectEqual(@as(usize, 1), calls);
    var writer = fx.JsonWriter.Writer.init(&env, .{ .max_output_bytes = 64, .max_json_depth = 4, .max_value_bytes = 32, .max_integer_digits = 16 });
    defer writer.deinit();
    var written = transformed.writeJson(&input, &writer);
    defer written.deinit();
    try std.testing.expect(written == .success);
    try std.testing.expectEqual(@as(usize, 2), calls);
    try std.testing.expectEqualStrings("41", writer.output.items);
    const Record = fx.dsl.schemaRecord(Env, .{ .fields = .{ .amount = .{ .codec = transformed, .presence = .required } }, .policy = .{ .unknown_fields = .reject, .duplicate_fields = .reject, .issues = .first } });
    var invalid = Record.parseBytesDirect(&env, "{\"amount\":-1}", .{ .max_document_bytes = 64, .max_depth = 4, .max_value_bytes = 32 });
    defer invalid.deinit();
    try std.testing.expect(invalid == .fail and invalid.fail.direction == .decode);
    try std.testing.expectEqualStrings("amount", invalid.fail.path.?[0].field.items);
    try std.testing.expectEqualStrings("amount-range", invalid.fail.kind.custom);
}

test "CODEC-04 INF-12 generic transformation callbacks specialize from the selected child codec" {
    const Roles = struct {
        fn forward(input: anytype) fx.Result(u16, Issue) {
            return .{ .ok = @intCast(input + 1) };
        }
        fn backward(value: anytype) fx.Result(i64, Issue) {
            return .{ .ok = value.* - 1 };
        }
    };
    const transformed = comptime fx.dsl.transformCodec(fx.JsonCodec.signed_integer, .{ .forward = Roles.forward, .backward = Roles.backward });
    var decoded = transformed.decodeDirect(&.{ .integer = 8 });
    defer decoded.deinit();
    try std.testing.expect(decoded == .ok and decoded.ok == 9);
    var encoded = transformed.encodeDirect(&decoded.ok);
    defer encoded.deinit();
    try std.testing.expect(encoded == .ok and encoded.ok.integer == 8);
}
