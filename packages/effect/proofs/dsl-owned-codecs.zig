const std = @import("std");
const fx = @import("effect");
const gates = @import("support/dsl_gate.zig");
const Threads = @import("support/thread_executor.zig").ThreadExecutor;
const Raw = struct { text: []const u8, number: i64 };
const Error = struct { message: []const u8 };
const Gate = gates.Gate(Raw, Error);
const EncodeGate = gates.Gate(usize, Error);
const Construction = struct { allocator: std.mem.Allocator };
const rejecting_executor: fx.Executor = .{
    .userdata = null,
    .capabilities = .{ .independent = false, .cpu = false, .blocking = false },
    .submit_fn = struct {
        fn submit(_: ?*anyopaque, _: fx.ExecutionKind, _: fx.ExecutionJob) fx.Executor.Submission {
            return .{ .rejected = fx.Defect.init("unexpected codec provider admission") };
        }
    }.submit,
};
const DecodeEnv = struct {
    allocator: std.mem.Allocator,
    io: std.Io = std.testing.io,
    async_engine: fx.Executor = rejecting_executor,
    gate: ?*Gate = null,
    offset: i64,
    calls: *usize,
    cancel_after_success: bool = false,
    cancellation: *fx.Cancellation,
};
const EncodeEnv = struct {
    allocator: std.mem.Allocator,
    io: std.Io = std.testing.io,
    async_engine: fx.Executor = rejecting_executor,
    gate: ?*EncodeGate = null,
    offset: usize,
    calls: *usize,
    cancellation: *fx.Cancellation,
};
const Configuration = struct {
    pub const effect_owned_payload = true;
    allocator: std.mem.Allocator,
    bytes: []u8,
    live: *std.atomic.Value(usize),
    pub fn deinit(self: *@This()) void {
        self.allocator.free(self.bytes);
        _ = self.live.fetchSub(1, .acq_rel);
    }
    fn init(allocator: std.mem.Allocator, live: *std.atomic.Value(usize)) !@This() {
        const bytes = try allocator.dupe(u8, "captured codec configuration");
        _ = live.fetchAdd(1, .acq_rel);
        return .{ .allocator = allocator, .bytes = bytes, .live = live };
    }
};
const PureCallbacks = struct {
    fn decode(configuration: anytype, input: *const usize) fx.Result(Raw, Error) {
        configuration.calls.* += 1;
        if (input.* == 0) return .{ .err = .{ .message = configuration.owner.bytes } };
        return .{ .ok = .{ .text = configuration.owner.bytes, .number = @as(i64, @intCast(input.*)) + configuration.offset } };
    }
    fn encode(configuration: anytype, input: *const Raw) fx.Result(usize, Error) {
        configuration.calls.* += 1;
        if (input.number < configuration.offset) return .{ .err = .{ .message = configuration.owner.bytes } };
        return .{ .ok = @intCast(input.number - configuration.offset) };
    }
    fn contextualDecode(configuration: anytype, input: *const usize, env: *const DecodeEnv) fx.Effect(Raw, Error, DecodeEnv) {
        return Callbacks.decode(&configuration.owner, input, env);
    }
    fn contextualEncode(configuration: anytype, input: *const Raw, env: *const EncodeEnv) fx.Effect(usize, Error, EncodeEnv) {
        return Callbacks.encode(&configuration.owner, input, env);
    }
};
const WriterMode = enum { normal, failure, cleanup, limited, invalid, late_cancel };
const WriterEnv = struct { allocator: std.mem.Allocator, max_output: usize, cancellation: *fx.Cancellation };
const WriterCallbacks = struct {
    fn decode(_: anytype, input: *const std.json.Value) fx.Result(i64, fx.SchemaIssue.Issue) {
        return fx.JsonCodec.signed_integer.decodeDirect(input);
    }
    fn encode(_: anytype, input: *const i64) fx.Result(std.json.Value, fx.SchemaIssue.Issue) {
        return fx.JsonCodec.signed_integer.encodeDirect(input);
    }
    fn write(configuration: anytype, input: *const i64, output: *fx.JsonWriter.Writer) fx.Exit(void, fx.SchemaIssue.Issue) {
        configuration.calls.* += 1;
        std.debug.assert(configuration.owner.live.load(.acquire) == 1);
        const env = .{ .allocator = output.output.allocator };
        if (configuration.mode == .failure) return .{ .fail = fx.SchemaIssue.Issue.init(&env, .encode, .{ .custom = configuration.owner.bytes }) };
        if (configuration.mode == .cleanup) return .{ .finalization_failed = .{
            .primary = .{ .failed = fx.SchemaIssue.Issue.init(&env, .encode, .{ .custom = configuration.owner.bytes }) },
            .first = .{ .release_failure = fx.SchemaIssue.Issue.init(&env, .encode, .{ .custom = configuration.owner.bytes }) },
        } };
        if (configuration.mode == .late_cancel) configuration.cancellation.request();
        const adjusted = input.* + configuration.offset;
        return fx.JsonCodec.signed_integer.writeJson(&adjusted, output);
    }
    fn limits(env: *const WriterEnv) fx.JsonWriter.Limits {
        return .{ .max_output_bytes = env.max_output, .max_json_depth = 4, .max_value_bytes = 32, .max_integer_digits = 16 };
    }
};
const Callbacks = struct {
    fn decode(configuration: *const Configuration, input: *const usize, env: *const DecodeEnv) fx.Effect(Raw, Error, DecodeEnv) {
        env.calls.* += 1;
        const P = fx.Effect(Raw, Error, DecodeEnv);
        if (env.gate) |gate| {
            if (gate.result == .success) gate.result.success = .{ .text = configuration.bytes, .number = @as(i64, @intCast(input.*)) + env.offset };
            if (gate.result == .fail) gate.result.fail.message = configuration.bytes;
            if (gate.result == .finalization_failed) {
                gate.result.finalization_failed.primary.failed.message = configuration.bytes;
                gate.result.finalization_failed.first.release_failure.message = configuration.bytes;
            }
            return P.fromAsync(struct {
                fn start(current: *const DecodeEnv) fx.Async(Raw, Error).Start {
                    return current.gate.?.start();
                }
            }.start);
        }
        const Capture = struct { configuration: *const Configuration, input: *const usize };
        return P.fromOwned(env, Capture{ .configuration = configuration, .input = input }, struct {
            fn run(capture: *const Capture, current: *const DecodeEnv) fx.Exit(Raw, Error) {
                if (capture.input.* == 0) return .{ .fail = .{ .message = capture.configuration.bytes } };
                if (current.cancel_after_success) current.cancellation.request();
                return .{ .success = .{ .text = capture.configuration.bytes, .number = @as(i64, @intCast(capture.input.*)) + current.offset } };
            }
        }.run);
    }
    fn encode(configuration: *const Configuration, input: *const Raw, env: *const EncodeEnv) fx.Effect(usize, Error, EncodeEnv) {
        env.calls.* += 1;
        std.debug.assert(configuration.live.load(.acquire) == 1);
        if (env.gate) |gate| {
            if (gate.result == .success) gate.result.success = @as(usize, @intCast(input.number)) + env.offset;
            if (gate.result == .fail) gate.result.fail.message = configuration.bytes;
            if (gate.result == .finalization_failed) {
                gate.result.finalization_failed.primary.failed.message = configuration.bytes;
                gate.result.finalization_failed.first.release_failure.message = configuration.bytes;
            }
            return fx.Effect(usize, Error, EncodeEnv).fromAsync(struct {
                fn start(current: *const EncodeEnv) fx.Async(usize, Error).Start {
                    return current.gate.?.start();
                }
            }.start);
        }
        return fx.Effect(usize, Error, EncodeEnv).fromBorrowed(Raw, input, struct {
            fn run(value: *const Raw, current: *const EncodeEnv) fx.Exit(usize, Error) {
                return .{ .success = @as(usize, @intCast(value.number)) + current.offset };
            }
        }.run);
    }
};
const metadata = fx.dsl.declareMetadata(.{ .kind = fx.dsl.CodecKind.contextual, .decode = Callbacks.decode, .encode = Callbacks.encode });
fn describe(allocator: std.mem.Allocator, configuration: Configuration) @TypeOf(fx.dsl.makeCodec(&@as(Construction, undefined), .{ .definition = metadata, .inputs = configuration })) {
    const env: Construction = .{ .allocator = allocator };
    return fx.dsl.makeCodec(&env, .{ .definition = metadata, .inputs = configuration });
}

test "CODEC-03 CODEC-04 DECL-03 CAP-02 captured codec infers independent environments and owns configuration through results and errors" {
    var live: std.atomic.Value(usize) = .init(0);
    var calls: usize = 0;
    var cancellation: fx.Cancellation = .{};
    const construction: Construction = .{ .allocator = std.testing.allocator };
    const decoding: DecodeEnv = .{ .allocator = std.testing.allocator, .offset = 5, .calls = &calls, .cancellation = &cancellation };
    const encoding: EncodeEnv = .{ .allocator = std.testing.allocator, .offset = 7, .calls = &calls, .cancellation = &cancellation };
    var description = describe(std.testing.allocator, try Configuration.init(std.testing.allocator, &live));
    try std.testing.expectEqual(@as(usize, 0), calls);
    var made = fx.run(&construction, description);
    const C = @TypeOf(made).Success;
    comptime {
        if (@TypeOf(description) != fx.Effect(C, C.Error, Construction)) @compileError("codec creation must compose as exact Effect");
        if (C.DecodingEnvironment != DecodeEnv or C.EncodingEnvironment != EncodeEnv) @compileError("codec environments must remain independent");
    }
    description.deinit();
    var wire = made.success.representation(11);
    var decoded_description = made.success.decode(&wire);
    comptime {
        if (@TypeOf(decoded_description) != fx.Effect(C.Decoded, C.Error, DecodeEnv)) @compileError("codec decode must remain exact Effect");
    }
    var decoded = fx.run(&decoding, decoded_description);
    decoded_description.deinit();
    try std.testing.expectEqual(@as(i64, 16), decoded.success.value.number);
    var encoded_description = made.success.encode(&decoded.success);
    comptime {
        if (@TypeOf(encoded_description) != fx.Effect(C.Representation, C.Error, EncodeEnv)) @compileError("codec encode must remain exact Effect");
    }
    var encoded = fx.run(&encoding, encoded_description);
    encoded_description.deinit();
    try std.testing.expectEqual(@as(usize, 23), encoded.success.value);
    var bad_wire = made.success.representation(0);
    var bad_description = made.success.decode(&bad_wire);
    var failed = fx.run(&decoding, bad_description);
    bad_description.deinit();
    bad_wire.deinit();
    wire.deinit();
    made.deinit();
    encoded.deinit();
    try std.testing.expectEqual(@as(usize, 1), live.load(.acquire));
    try std.testing.expectEqualStrings("captured codec configuration", decoded.success.value.text);
    decoded.deinit();
    try std.testing.expectEqualStrings("captured codec configuration", failed.fail.value.message);
    failed.deinit();
    try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
    try std.testing.expectEqual(@as(usize, 3), calls);
}

test "CAP-04 CAP-10 CODEC-04 codec definition discard and reuse retain one noncloneable configuration" {
    var live: std.atomic.Value(usize) = .init(0);
    const env: Construction = .{ .allocator = std.testing.allocator };
    var unused = describe(std.testing.allocator, try Configuration.init(std.testing.allocator, &live));
    unused.deinit();
    try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
    var description = describe(std.testing.allocator, try Configuration.init(std.testing.allocator, &live));
    var first = fx.run(&env, description);
    var second = fx.run(&env, description);
    description.deinit();
    first.deinit();
    try std.testing.expectEqual(@as(usize, 1), live.load(.acquire));
    var copy = try second.success.clone();
    second.deinit();
    copy.deinit();
    try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
}

test "LIFE-07 CODEC-04 codec construction and every decode encode allocation failure release configuration" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            var live: std.atomic.Value(usize) = .init(0);
            defer std.debug.assert(live.load(.acquire) == 0);
            var calls: usize = 0;
            var cancellation: fx.Cancellation = .{};
            const construction: Construction = .{ .allocator = allocator };
            const decoding: DecodeEnv = .{ .allocator = allocator, .offset = 5, .calls = &calls, .cancellation = &cancellation };
            const encoding: EncodeEnv = .{ .allocator = allocator, .offset = 7, .calls = &calls, .cancellation = &cancellation };
            var description = describe(allocator, try Configuration.init(allocator, &live));
            defer description.deinit();
            var made = fx.run(&construction, description);
            defer made.deinit();
            if (made == .die and made.die.kind == .out_of_memory) return error.OutOfMemory;
            var wire = made.success.representation(11);
            defer wire.deinit();
            var decode_description = made.success.decode(&wire);
            defer decode_description.deinit();
            var decoded = fx.run(&decoding, decode_description);
            defer decoded.deinit();
            if (decoded == .die and decoded.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expectEqual(@as(i64, 16), decoded.success.value.number);
            var encode_description = made.success.encode(&decoded.success);
            defer encode_description.deinit();
            var encoded = fx.run(&encoding, encode_description);
            defer encoded.deinit();
            if (encoded == .die and encoded.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expectEqual(@as(usize, 23), encoded.success.value);
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

test "CODEC-04 CODEC-06 LIFE-11 LIFE-12 both captured directions retain configuration through pending outcomes and cancellation" {
    inline for (.{ true, false }) |decoding| {
        inline for (.{ .success, .fail, .die, .interrupted, .finalization_failed, .cancel }) |mode| {
            var live: std.atomic.Value(usize) = .init(0);
            var input_live: std.atomic.Value(usize) = .init(0);
            var calls: usize = 0;
            var cancellation: fx.Cancellation = .{};
            var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
            defer threads.deinit();
            const PendingGate = if (decoding) Gate else EncodeGate;
            var gate: PendingGate = .{ .result = switch (mode) {
                .success, .cancel => .{ .success = if (decoding) .{ .text = "", .number = 0 } else 0 },
                .fail => .{ .fail = .{ .message = "" } },
                .die => .{ .die = fx.Defect.init("pending codec defect") },
                .interrupted => .interrupted,
                .finalization_failed => .{ .finalization_failed = .{ .primary = .{ .failed = .{ .message = "" } }, .first = .{ .release_failure = .{ .message = "" } } } },
                else => unreachable,
            } };
            const R = if (decoding) DecodeEnv else EncodeEnv;
            const env: R = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .gate = &gate, .offset = 7, .calls = &calls, .cancellation = &cancellation };
            const construction: Construction = .{ .allocator = std.testing.allocator };
            var description = describe(std.testing.allocator, try Configuration.init(std.testing.allocator, &live));
            var made = fx.run(&construction, description);
            description.deinit();
            var input_description = describe(std.testing.allocator, try Configuration.init(std.testing.allocator, &input_live));
            var input_codec = fx.run(&construction, input_description);
            input_description.deinit();
            // The borrowed input belongs to a different configuration, so it
            // cannot accidentally keep the operation's configuration alive.
            var input = if (decoding) input_codec.success.representation(11) else input_codec.success.value(.{ .text = "input", .number = 11 });
            input_codec.deinit();
            var operation = if (decoding) made.success.decode(&input) else made.success.encode(&input);
            var pending = fx.runAsync(&env, operation);
            operation.deinit();
            made.deinit();
            gate.started.waitUncancelable(env.io);
            try std.testing.expectEqual(@as(usize, 1), calls);
            try std.testing.expectEqual(@as(usize, 1), live.load(.acquire));
            if (mode == .cancel) pending.request() else gate.proceed.set(env.io);
            var result = pending.join();
            pending.deinit();
            input.deinit();
            try std.testing.expect(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
            try std.testing.expectEqual(@as(usize, 0), input_live.load(.acquire));
            switch (mode) {
                .success => {
                    try std.testing.expect(result == .success);
                    if (decoding) try std.testing.expectEqualStrings("captured codec configuration", result.success.value.text);
                    try std.testing.expectEqual(@as(usize, 1), live.load(.acquire));
                },
                .fail => {
                    try std.testing.expect(result == .fail);
                    try std.testing.expectEqualStrings("captured codec configuration", result.fail.value.message);
                    try std.testing.expectEqual(@as(usize, 1), live.load(.acquire));
                },
                .finalization_failed => {
                    try std.testing.expect(result == .finalization_failed);
                    try std.testing.expectEqualStrings("captured codec configuration", result.finalization_failed.primary.failed.value.message);
                    try std.testing.expectEqualStrings("captured codec configuration", result.finalization_failed.first.release_failure.value.message);
                    try std.testing.expectEqual(@as(usize, 1), live.load(.acquire));
                },
                .die => try std.testing.expect(result == .die),
                .interrupted, .cancel => try std.testing.expect(result == .interrupted),
                else => unreachable,
            }
            result.deinit();
            try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
        }
    }
}

test "CODEC-03 CAP-11 captured codec storage and execution allocate in their own domains with projected services" {
    var construction_allocator: std.testing.FailingAllocator = .init(std.testing.allocator, .{});
    var execution_allocator: std.testing.FailingAllocator = .init(std.testing.allocator, .{});
    var live: std.atomic.Value(usize) = .init(0);
    var calls: usize = 0;
    var cancellation: fx.Cancellation = .{};
    const construction: Construction = .{ .allocator = construction_allocator.allocator() };
    const Env = struct { decoding: DecodeEnv, encoding: EncodeEnv };
    const project = struct {
        fn decode(env: *const Env) *const DecodeEnv {
            return &env.decoding;
        }
        fn encode(env: *const Env) *const EncodeEnv {
            return &env.encoding;
        }
    };
    const env: Env = .{
        .decoding = .{ .allocator = execution_allocator.allocator(), .offset = 5, .calls = &calls, .cancellation = &cancellation },
        .encoding = .{ .allocator = execution_allocator.allocator(), .offset = 7, .calls = &calls, .cancellation = &cancellation },
    };
    var description = describe(construction.allocator, try Configuration.init(construction.allocator, &live));
    var made = fx.run(&construction, description);
    description.deinit();
    var wire = made.success.representation(11);
    var decode_description = made.success.decodeIn(&wire, project.decode);
    const allocated_before = construction_allocator.allocations;
    var decoded = fx.run(&env, decode_description);
    // The kernel's retained description link follows its description owner;
    // the codec's execution frame and callback storage follow projected R.
    try std.testing.expectEqual(allocated_before + 1, construction_allocator.allocations);
    try std.testing.expect(execution_allocator.allocations > 0);
    decode_description.deinit();
    var encode_description = made.success.encodeIn(&decoded.success, project.encode);
    var encoded = fx.run(&env, encode_description);
    encode_description.deinit();
    try std.testing.expectEqual(@as(usize, 23), encoded.success.value);
    encoded.deinit();
    decoded.deinit();
    wire.deinit();
    made.deinit();
    try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
    try std.testing.expectEqual(construction_allocator.allocated_bytes, construction_allocator.freed_bytes);
    try std.testing.expectEqual(execution_allocator.allocated_bytes, execution_allocator.freed_bytes);
}

test "INF-03 INF-04 CODEC-04 captured codec specializes generic callback inputs from paired results without executing construction work" {
    const Generic = struct {
        fn decode(configuration: anytype, input: anytype, env: anytype) fx.Effect(Raw, Error, DecodeEnv) {
            return Callbacks.decode(configuration, input, env);
        }
        fn encode(configuration: anytype, input: anytype, env: anytype) fx.Effect(usize, Error, EncodeEnv) {
            return Callbacks.encode(configuration, input, env);
        }
    };
    var live: std.atomic.Value(usize) = .init(0);
    var calls: usize = 0;
    var cancellation: fx.Cancellation = .{};
    const construction: Construction = .{ .allocator = std.testing.allocator };
    const decoding: DecodeEnv = .{ .allocator = std.testing.allocator, .offset = 5, .calls = &calls, .cancellation = &cancellation };
    var description = fx.dsl.makeCodec(&construction, .{
        .definition = fx.dsl.declareMetadata(.{ .kind = fx.dsl.CodecKind.contextual, .decode = Generic.decode, .encode = Generic.encode }),
        .inputs = try Configuration.init(std.testing.allocator, &live),
    });
    var made = fx.run(&construction, description);
    try std.testing.expectEqual(@as(usize, 0), calls);
    var input = made.success.representation(11);
    var operation = made.success.decode(&input);
    var result = fx.run(&decoding, operation);
    operation.deinit();
    input.deinit();
    made.deinit();
    description.deinit();
    try std.testing.expectEqual(@as(i64, 16), result.success.value.number);
    try std.testing.expectEqualStrings("captured codec configuration", result.success.value.text);
    result.deinit();
    try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
}

test "CAP-04 LIFE-07 makeCodec without construction allocator releases runtime inputs before returning a lazy configuration defect" {
    var live: std.atomic.Value(usize) = .init(0);
    const env = {};
    var described = fx.dsl.makeCodec(&env, .{ .definition = metadata, .inputs = try Configuration.init(std.testing.allocator, &live) });
    defer described.deinit();
    try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
    var result = fx.run(&env, described);
    defer result.deinit();
    try std.testing.expect(result == .die and result.die.kind == .invalid_configuration);
}

test "INF-12 INF-13 AUD-01 captured codec resolves either determinate direction before asking for an anchor" {
    const Cfg = struct { offset: u16, calls: *usize };
    const E = enum { failure };
    const RightFirst = struct {
        fn decode(configuration: *const Cfg, input: anytype, env: *const DecodeEnv) fx.Effect(u32, E, DecodeEnv) {
            configuration.calls.* += 1;
            return fx.Effect(u32, E, DecodeEnv).succeed(@as(u32, input.*) + @as(u32, @intCast(env.offset)));
        }
        fn encode(configuration: anytype, input: *const u32, _: *const EncodeEnv) fx.Effect(@TypeOf(configuration.offset), E, EncodeEnv) {
            configuration.calls.* += 1;
            return fx.Effect(@TypeOf(configuration.offset), E, EncodeEnv).succeed(@intCast(input.*));
        }
    };
    const LeftFirst = struct {
        fn decode(configuration: anytype, input: *const u32, _: *const DecodeEnv) fx.Effect(@TypeOf(configuration.offset), E, DecodeEnv) {
            configuration.calls.* += 1;
            return fx.Effect(@TypeOf(configuration.offset), E, DecodeEnv).succeed(@intCast(input.*));
        }
        fn encode(configuration: *const Cfg, input: anytype, env: *const EncodeEnv) fx.Effect(u32, E, EncodeEnv) {
            configuration.calls.* += 1;
            return fx.Effect(u32, E, EncodeEnv).succeed(@as(u32, input.*) + @as(u32, @intCast(env.offset)));
        }
    };
    inline for (.{ RightFirst, LeftFirst }) |CallbacksWithDependentReturn| {
        var calls: usize = 0;
        var cancellation: fx.Cancellation = .{};
        const construction: Construction = .{ .allocator = std.testing.allocator };
        const decoding: DecodeEnv = .{ .allocator = std.testing.allocator, .offset = 5, .calls = &calls, .cancellation = &cancellation };
        const encoding: EncodeEnv = .{ .allocator = std.testing.allocator, .offset = 7, .calls = &calls, .cancellation = &cancellation };
        var description = fx.dsl.makeCodec(&construction, .{
            .definition = fx.dsl.declareMetadata(.{ .kind = fx.dsl.CodecKind.contextual, .decode = CallbacksWithDependentReturn.decode, .encode = CallbacksWithDependentReturn.encode }),
            .inputs = Cfg{ .offset = 1, .calls = &calls },
        });
        var made = fx.run(&construction, description);
        try std.testing.expectEqual(@as(usize, 0), calls);
        const Codec = @TypeOf(made).Success;
        comptime {
            if (Codec.RawDecoded != (if (CallbacksWithDependentReturn == RightFirst) u32 else u16)) @compileError("incorrect inferred application value");
            if (Codec.RawRepresentation != (if (CallbacksWithDependentReturn == RightFirst) u16 else u32)) @compileError("incorrect inferred representation");
        }
        var input = made.success.representation(11);
        var decode_description = made.success.decode(&input);
        var decoded = fx.run(&decoding, decode_description);
        decode_description.deinit();
        var encode_description = made.success.encode(&decoded.success);
        var encoded = fx.run(&encoding, encode_description);
        encode_description.deinit();
        try std.testing.expectEqual(@as(Codec.RawRepresentation, if (CallbacksWithDependentReturn == RightFirst) 16 else 18), encoded.success.value);
        try std.testing.expectEqual(@as(usize, 2), calls);
        encoded.deinit();
        decoded.deinit();
        input.deinit();
        made.deinit();
        description.deinit();
    }
}

test "CODEC-02 CODEC-06 LIFE-07 generated codec owners release noncloneable owning values representations and errors before configuration on every allocation failure" {
    const Bytes = struct {
        pub const effect_owned_payload = true;
        allocator: std.mem.Allocator,
        bytes: []u8,
        configuration_bytes: []const u8,
        configuration_live: *std.atomic.Value(usize),
        business_live: *usize,
        fn make(allocator: std.mem.Allocator, bytes: []const u8, configuration_bytes: []const u8, configuration_live: *std.atomic.Value(usize), business_live: *usize) !@This() {
            const owned = try allocator.dupe(u8, bytes);
            business_live.* += 1;
            return .{ .allocator = allocator, .bytes = owned, .configuration_bytes = configuration_bytes, .configuration_live = configuration_live, .business_live = business_live };
        }
        pub fn deinit(self: *@This()) void {
            std.debug.assert(self.configuration_live.load(.acquire) == 1);
            std.debug.assert(std.mem.eql(u8, self.configuration_bytes, "captured codec configuration"));
            self.allocator.free(self.bytes);
            self.business_live.* -= 1;
        }
    };
    const OwningCallbacks = struct {
        fn copy(comptime R: type, configuration: *const Configuration, input: *const Bytes, env: *const R) fx.Effect(Bytes, Bytes, R) {
            const Capture = struct { configuration: *const Configuration, input: *const Bytes };
            return fx.Effect(Bytes, Bytes, R).fromOwned(env, Capture{ .configuration = configuration, .input = input }, struct {
                fn run(capture: *const Capture, current: *const R) fx.Exit(Bytes, Bytes) {
                    var value = Bytes.make(current.allocator, capture.input.bytes, capture.configuration.bytes, capture.configuration.live, capture.input.business_live) catch return .{ .die = fx.Defect.out_of_memory };
                    return switch (capture.input.bytes[0]) {
                        '!' => .{ .fail = value },
                        '#' => blk: {
                            const cleanup = Bytes.make(current.allocator, "cleanup", capture.configuration.bytes, capture.configuration.live, capture.input.business_live) catch {
                                value.deinit();
                                break :blk .{ .die = fx.Defect.out_of_memory };
                            };
                            break :blk .{ .finalization_failed = .{ .primary = .{ .failed = value }, .first = .{ .release_failure = cleanup } } };
                        },
                        else => .{ .success = value },
                    };
                }
            }.run);
        }
        fn decode(configuration: *const Configuration, input: *const Bytes, env: *const DecodeEnv) fx.Effect(Bytes, Bytes, DecodeEnv) {
            return copy(DecodeEnv, configuration, input, env);
        }
        fn encode(configuration: *const Configuration, input: *const Bytes, env: *const EncodeEnv) fx.Effect(Bytes, Bytes, EncodeEnv) {
            return copy(EncodeEnv, configuration, input, env);
        }
    };
    const Check = struct {
        fn run(allocator: std.mem.Allocator, mode: []const u8) !void {
            var live: std.atomic.Value(usize) = .init(0);
            var business_live: usize = 0;
            defer std.debug.assert(live.load(.acquire) == 0 and business_live == 0);
            var calls: usize = 0;
            var cancellation: fx.Cancellation = .{};
            const construction: Construction = .{ .allocator = allocator };
            const decoding: DecodeEnv = .{ .allocator = allocator, .offset = 0, .calls = &calls, .cancellation = &cancellation };
            const encoding: EncodeEnv = .{ .allocator = allocator, .offset = 0, .calls = &calls, .cancellation = &cancellation };
            const configuration = try Configuration.init(allocator, &live);
            const configuration_bytes = configuration.bytes;
            var description = fx.dsl.makeCodec(&construction, .{
                .definition = fx.dsl.declareMetadata(.{ .kind = fx.dsl.CodecKind.contextual, .decode = OwningCallbacks.decode, .encode = OwningCallbacks.encode }),
                .inputs = configuration,
            });
            defer description.deinit();
            var made = fx.run(&construction, description);
            defer made.deinit();
            if (made == .die and made.die.kind == .out_of_memory) return error.OutOfMemory;
            var input = made.success.representation(try Bytes.make(allocator, mode, configuration_bytes, &live, &business_live));
            defer input.deinit();
            var decode_description = made.success.decode(&input);
            defer decode_description.deinit();
            var decoded = fx.run(&decoding, decode_description);
            defer decoded.deinit();
            if (decoded == .die and decoded.die.kind == .out_of_memory) return error.OutOfMemory;
            if (mode[0] == '!') {
                try std.testing.expect(decoded == .fail);
                try std.testing.expectEqualStrings(mode, decoded.fail.value.bytes);
            } else if (mode[0] == '#') {
                try std.testing.expect(decoded == .finalization_failed);
                try std.testing.expectEqualStrings(mode, decoded.finalization_failed.primary.failed.value.bytes);
                try std.testing.expectEqualStrings("cleanup", decoded.finalization_failed.first.release_failure.value.bytes);
            } else {
                try std.testing.expect(decoded == .success);
                var encode_description = made.success.encode(&decoded.success);
                defer encode_description.deinit();
                var encoded = fx.run(&encoding, encode_description);
                defer encoded.deinit();
                if (encoded == .die and encoded.die.kind == .out_of_memory) return error.OutOfMemory;
                try std.testing.expectEqualStrings(mode, encoded.success.value.bytes);
            }
        }
    };
    inline for (.{ "valid", "!failure", "#primary" }) |mode| try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{mode});
}

test "CODEC-04 LIFE-12 rejected root admission drains pending codec work before configuration release" {
    var live: std.atomic.Value(usize) = .init(0);
    var calls: usize = 0;
    var cancellation: fx.Cancellation = .{};
    var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 0, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
    defer threads.deinit();
    var gate: Gate = .{ .result = .{ .success = .{ .text = "", .number = 0 } } };
    const env: DecodeEnv = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .gate = &gate, .offset = 5, .calls = &calls, .cancellation = &cancellation };
    const construction: Construction = .{ .allocator = std.testing.allocator };
    var description = describe(std.testing.allocator, try Configuration.init(std.testing.allocator, &live));
    var made = fx.run(&construction, description);
    description.deinit();
    var input = made.success.representation(11);
    var operation = made.success.decode(&input);
    var pending = fx.runAsync(&env, operation);
    operation.deinit();
    made.deinit();
    input.deinit();
    try std.testing.expect(gate.cancelled.load(.acquire) and gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
    var result = pending.join();
    pending.deinit();
    try std.testing.expect(result.primary() == .defected);
    result.deinit();
    try std.testing.expectEqual(@as(usize, 1), calls);
    try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
}

test "CODEC-04 LIFE-07 late cancellation discards a completed captured result and validation executes one encoder" {
    var live: std.atomic.Value(usize) = .init(0);
    var calls: usize = 0;
    var cancellation: fx.Cancellation = .{};
    var encoding_cancellation: fx.Cancellation = .{};
    const construction: Construction = .{ .allocator = std.testing.allocator };
    const decoding: DecodeEnv = .{ .allocator = std.testing.allocator, .offset = 5, .calls = &calls, .cancel_after_success = true, .cancellation = &cancellation };
    const encoding: EncodeEnv = .{ .allocator = std.testing.allocator, .offset = 7, .calls = &calls, .cancellation = &encoding_cancellation };
    var description = describe(std.testing.allocator, try Configuration.init(std.testing.allocator, &live));
    var made = fx.run(&construction, description);
    var input = made.success.representation(11);
    var operation = made.success.decode(&input);
    var result = fx.run(&decoding, operation);
    operation.deinit();
    try std.testing.expect(result == .interrupted);
    result.deinit();
    var value = made.success.value(.{ .text = "validation input", .number = 11 });
    var validate = made.success.validate(&value);
    comptime {
        const Codec = @TypeOf(made).Success;
        if (@TypeOf(validate) != fx.Effect(void, Codec.Error, EncodeEnv)) @compileError("codec validation must remain exact Effect");
    }
    var validated = fx.run(&encoding, validate);
    try std.testing.expect(validated == .success);
    validated.deinit();
    validate.deinit();
    value.deinit();
    input.deinit();
    made.deinit();
    description.deinit();
    try std.testing.expectEqual(@as(usize, 2), calls);
    try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
}

test "CAP-02 DECL-03 CODEC-04 anonymous runtime codec configuration derives capture cleanup and preserves explicit shape without pure evidence" {
    const Selected = struct {
        fn decode(configuration: anytype, _: *const u16, _: *const DecodeEnv) fx.Effect([]const u8, Error, DecodeEnv) {
            configuration.calls.* += 1;
            return fx.Effect([]const u8, Error, DecodeEnv).fromBorrowed(Configuration, &configuration.owned, struct {
                fn run(owned: *const Configuration, _: *const DecodeEnv) fx.Exit([]const u8, Error) {
                    return .{ .success = owned.bytes };
                }
            }.run);
        }
        fn encode(configuration: anytype, text: *const []const u8, _: *const EncodeEnv) fx.Effect(@TypeOf(configuration.offset), Error, EncodeEnv) {
            configuration.calls.* += 1;
            return fx.Effect(@TypeOf(configuration.offset), Error, EncodeEnv).succeed(configuration.offset + @as(u16, @intCast(text.len)));
        }
    };
    var live: std.atomic.Value(usize) = .init(0);
    var calls: usize = 0;
    var cancellation: fx.Cancellation = .{};
    const construction: Construction = .{ .allocator = std.testing.allocator };
    const decoding: DecodeEnv = .{ .allocator = std.testing.allocator, .offset = 0, .calls = &calls, .cancellation = &cancellation };
    const encoding: EncodeEnv = .{ .allocator = std.testing.allocator, .offset = 0, .calls = &calls, .cancellation = &cancellation };
    var description = fx.dsl.makeCodec(&construction, .{
        .definition = fx.dsl.declareMetadata(.{ .kind = fx.dsl.CodecKind.contextual, .decode = Selected.decode, .encode = Selected.encode, .shape = fx.JsonCodec.signed_integer.encoded_shape }),
        .inputs = .{ .owned = try Configuration.init(std.testing.allocator, &live), .calls = &calls, .offset = @as(u16, 2) },
    });
    var made = fx.run(&construction, description);
    const Codec = @TypeOf(made).Success;
    comptime {
        if (Codec.pure_decoding or Codec.pure_encoding) @compileError("runtime configuration cannot fabricate pure evidence");
        if (Codec.RawRepresentation != u16) @compileError("configuration return evidence must determine the representation");
    }
    try std.testing.expectEqual(@as(usize, 0), calls);
    try std.testing.expectEqual(fx.JsonCodec.signed_integer.encoded_shape, made.success.encoded_shape);
    var input = made.success.representation(1);
    var decode = made.success.decode(&input);
    var decoded = fx.run(&decoding, decode);
    decode.deinit();
    var encode = made.success.encode(&decoded.success);
    var encoded = fx.run(&encoding, encode);
    encode.deinit();
    var composed_decode = made.success.decode(&input);
    var measured = composed_decode.mapAllocated(construction.allocator, struct {
        fn length(value: anytype) usize {
            var owned = value;
            defer owned.deinit();
            return owned.value.len;
        }
    }.length);
    var length = fx.run(&decoding, measured);
    measured.deinit();
    try std.testing.expectEqual(@as(usize, "captured codec configuration".len), length.success);
    length.deinit();
    input.deinit();
    made.deinit();
    description.deinit();
    try std.testing.expectEqual(@as(u16, 2 + "captured codec configuration".len), encoded.success.value);
    encoded.deinit();
    try std.testing.expectEqualStrings("captured codec configuration", decoded.success.value);
    decoded.deinit();
    try std.testing.expectEqual(@as(usize, 3), calls);
    try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
}

test "CODEC-03 CODEC-04 DECL-03 captured pure codec preserves direct Result and exact environment free Effect while retaining noncloneable configuration" {
    var live: std.atomic.Value(usize) = .init(0);
    var calls: usize = 0;
    const creation: Construction = .{ .allocator = std.testing.allocator };
    var description = fx.dsl.makeCodec(&creation, .{ .definition = fx.dsl.declareMetadata(.{ .kind = fx.dsl.CodecKind.pure, .decode = PureCallbacks.decode, .encode = PureCallbacks.encode }), .inputs = .{ .owner = try Configuration.init(std.testing.allocator, &live), .offset = @as(i64, 3), .calls = &calls } });
    try std.testing.expectEqual(@as(usize, 0), calls);
    var made = fx.run(&creation, description);
    const Codec = @TypeOf(made).Success;
    comptime if (!Codec.pure_decoding or !Codec.pure_encoding or Codec.DecodingEnvironment != void or Codec.EncodingEnvironment != void) @compileError("pure directions must preserve explicit purity and require no execution services");
    var input = made.success.representation(7);
    var decoded = made.success.decodeDirect(&input);
    try std.testing.expectEqual(@as(i64, 10), decoded.ok.value.number);
    var encoded = made.success.encodeDirect(&decoded.ok);
    try std.testing.expectEqual(@as(usize, 7), encoded.ok.value);
    var zero = made.success.representation(0);
    var rejected = made.success.decodeDirect(&zero);
    var lifted = made.success.decode(&input);
    comptime if (@TypeOf(lifted) != fx.Effect(Codec.Decoded, Codec.Error, void)) @compileError("captured pure lifting must use the exact existing Effect");
    const unit: void = {};
    var result = fx.run(&unit, lifted);
    try std.testing.expect(result == .success);
    lifted.deinit();
    description.deinit();
    made.deinit();
    input.deinit();
    zero.deinit();
    encoded.deinit();
    decoded.deinit();
    try std.testing.expectEqualStrings("captured codec configuration", result.success.value.text);
    try std.testing.expectEqualStrings("captured codec configuration", rejected.err.value.message);
    try std.testing.expectEqual(@as(usize, 1), live.load(.acquire));
    result.deinit();
    rejected.deinit();
    try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
}

test "CODEC-03 CODEC-04 captured mixed codec infers only its contextual direction services and retains explicit direct purity" {
    inline for (.{ fx.dsl.CodecKind.pure_decode, fx.dsl.CodecKind.pure_encode }) |kind| {
        var live: std.atomic.Value(usize) = .init(0);
        var calls: usize = 0;
        var cancellation: fx.Cancellation = .{};
        const creation: Construction = .{ .allocator = std.testing.allocator };
        const de: DecodeEnv = .{ .allocator = std.testing.allocator, .offset = 2, .calls = &calls, .cancellation = &cancellation };
        const en: EncodeEnv = .{ .allocator = std.testing.allocator, .offset = 5, .calls = &calls, .cancellation = &cancellation };
        var description = fx.dsl.makeCodec(&creation, .{ .definition = fx.dsl.declareMetadata(.{ .kind = kind, .decode = if (kind == .pure_decode) PureCallbacks.decode else PureCallbacks.contextualDecode, .encode = if (kind == .pure_encode) PureCallbacks.encode else PureCallbacks.contextualEncode }), .inputs = .{ .owner = try Configuration.init(std.testing.allocator, &live), .offset = @as(i64, 3), .calls = &calls } });
        var made = fx.run(&creation, description);
        const Codec = @TypeOf(made).Success;
        comptime if (Codec.DecodingEnvironment != (if (kind == .pure_decode) void else DecodeEnv) or Codec.EncodingEnvironment != (if (kind == .pure_encode) void else EncodeEnv)) @compileError("mixed codec must preserve only the contextual direction environment");
        comptime if (Codec.pure_decoding != (kind == .pure_decode) or Codec.pure_encoding != (kind == .pure_encode)) @compileError("mixed codec purity evidence must match the declaration");
        var input = made.success.representation(7);
        var decoding = made.success.decode(&input);
        const unit: void = {};
        var decoded = if (comptime kind == .pure_decode) fx.run(&unit, decoding) else fx.run(&de, decoding);
        decoding.deinit();
        try std.testing.expect(decoded == .success);
        var encoding = made.success.encode(&decoded.success);
        var encoded = if (comptime kind == .pure_encode) fx.run(&unit, encoding) else fx.run(&en, encoding);
        encoding.deinit();
        try std.testing.expectEqual(@as(usize, if (kind == .pure_decode) 15 else 6), encoded.success.value);
        if (comptime kind == .pure_decode) {
            var direct = made.success.decodeDirect(&input);
            direct.deinit();
        } else {
            var direct = made.success.encodeDirect(&decoded.success);
            direct.deinit();
        }
        input.deinit();
        description.deinit();
        made.deinit();
        encoded.deinit();
        try std.testing.expectEqualStrings("captured codec configuration", decoded.success.value.text);
        decoded.deinit();
        try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
    }
}

test "CODEC-02 CODEC-04 CAP-08 captured pure directions derive cleanup for unmarked composite success representation and error without business Clone" {
    const Token = struct {
        pub const effect_owned_payload = true;
        count: *usize,
        configuration: *std.atomic.Value(usize),
        pub fn deinit(self: *@This()) void {
            std.debug.assert(self.configuration.load(.acquire) == 1);
            self.count.* -= 1;
        }
    };
    const Value = struct { token: Token, value: usize };
    const Rep = struct { token: Token, value: usize };
    const Err = struct { token: Token };
    const Functions = struct {
        fn token(configuration: anytype) Token {
            configuration.tokens.* += 1;
            return .{ .count = configuration.tokens, .configuration = configuration.owner.live };
        }
        fn decode(configuration: anytype, input: *const Rep) fx.Result(Value, Err) {
            if (input.value == 0) return .{ .err = .{ .token = token(configuration) } };
            return .{ .ok = .{ .token = token(configuration), .value = input.value } };
        }
        fn encode(configuration: anytype, input: *const Value) fx.Result(Rep, Err) {
            return .{ .ok = .{ .token = token(configuration), .value = input.value } };
        }
    };
    var live: std.atomic.Value(usize) = .init(0);
    var tokens: usize = 0;
    const creation: Construction = .{ .allocator = std.testing.allocator };
    var description = fx.dsl.makeCodec(&creation, .{ .definition = fx.dsl.declareMetadata(.{ .kind = fx.dsl.CodecKind.pure, .decode = Functions.decode, .encode = Functions.encode }), .inputs = .{ .owner = try Configuration.init(std.testing.allocator, &live), .tokens = &tokens } });
    var made = fx.run(&creation, description);
    tokens += 1;
    var input = made.success.representation(.{ .token = .{ .count = &tokens, .configuration = &live }, .value = 7 });
    var decoded = made.success.decodeDirect(&input);
    var encoded = made.success.encodeDirect(&decoded.ok);
    const unit: void = {};
    var success_description = made.success.decode(&input);
    var success = fx.run(&unit, success_description);
    success_description.deinit();
    try std.testing.expect(success == .success);
    input.value.value = 0;
    var rejected = made.success.decodeDirect(&input);
    var lifted = made.success.decode(&input);
    var failed = fx.run(&unit, lifted);
    lifted.deinit();
    try std.testing.expect(failed == .fail);
    description.deinit();
    made.deinit();
    input.deinit();
    decoded.deinit();
    encoded.deinit();
    rejected.deinit();
    success.deinit();
    try std.testing.expectEqual(@as(usize, 1), tokens);
    failed.deinit();
    try std.testing.expectEqual(@as(usize, 0), tokens);
    try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
}

test "CODEC-04 LIFE-07 every captured pure and mixed construction decode encode allocation failure releases configuration" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator, comptime kind: fx.dsl.CodecKind) !void {
            var live: std.atomic.Value(usize) = .init(0);
            defer std.debug.assert(live.load(.acquire) == 0);
            var calls: usize = 0;
            var cancellation: fx.Cancellation = .{};
            const creation: Construction = .{ .allocator = allocator };
            const de: DecodeEnv = .{ .allocator = allocator, .offset = 2, .calls = &calls, .cancellation = &cancellation };
            const en: EncodeEnv = .{ .allocator = allocator, .offset = 5, .calls = &calls, .cancellation = &cancellation };
            var description = fx.dsl.makeCodec(&creation, .{ .definition = fx.dsl.declareMetadata(.{ .kind = kind, .decode = if (kind == .pure or kind == .pure_decode) PureCallbacks.decode else PureCallbacks.contextualDecode, .encode = if (kind == .pure or kind == .pure_encode) PureCallbacks.encode else PureCallbacks.contextualEncode }), .inputs = .{ .owner = try Configuration.init(allocator, &live), .offset = @as(i64, 3), .calls = &calls } });
            defer description.deinit();
            var made = fx.run(&creation, description);
            defer made.deinit();
            if (made == .die and made.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expect(made == .success);
            var input = made.success.representation(7);
            defer input.deinit();
            var decoding = made.success.decode(&input);
            defer decoding.deinit();
            const unit: void = {};
            var decoded = if (comptime kind == .pure or kind == .pure_decode) fx.run(&unit, decoding) else fx.run(&de, decoding);
            defer decoded.deinit();
            if (decoded == .die and decoded.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expect(decoded == .success);
            var encoding = made.success.encode(&decoded.success);
            defer encoding.deinit();
            var encoded = if (comptime kind == .pure or kind == .pure_encode) fx.run(&unit, encoding) else fx.run(&en, encoding);
            defer encoded.deinit();
            if (encoded == .die and encoded.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expect(encoded == .success);
        }
    };
    inline for (.{ fx.dsl.CodecKind.pure, fx.dsl.CodecKind.pure_decode, fx.dsl.CodecKind.pure_encode }) |kind| {
        const Case = struct {
            fn run(allocator: std.mem.Allocator) !void {
                return Check.run(allocator, kind);
            }
        };
        try std.testing.checkAllAllocationFailures(std.testing.allocator, Case.run, .{});
    }
}

test "CODEC-04 LIFE-12 mixed contextual directions retain their own configuration independently of the borrowed input codec through suspension and owner destruction" {
    const Cfg = struct { owner: Configuration, offset: i64, calls: *usize };
    inline for (.{ fx.dsl.CodecKind.pure_decode, fx.dsl.CodecKind.pure_encode }) |kind| {
        inline for (.{ false, true }) |destroy| {
            var active: std.atomic.Value(usize) = .init(0);
            var foreign: std.atomic.Value(usize) = .init(0);
            var calls: usize = 0;
            var cancellation: fx.Cancellation = .{};
            const G = if (kind == .pure_decode) EncodeGate else Gate;
            var gate: G = .{ .result = .{ .fail = .{ .message = "" } } };
            var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
            defer threads.deinit();
            const creation: Construction = .{ .allocator = std.testing.allocator };
            const de: DecodeEnv = .{ .allocator = std.testing.allocator, .offset = 2, .calls = &calls, .cancellation = &cancellation, .async_engine = threads.executor(), .gate = if (comptime kind == .pure_encode) &gate else null };
            const en: EncodeEnv = .{ .allocator = std.testing.allocator, .offset = 5, .calls = &calls, .cancellation = &cancellation, .async_engine = threads.executor(), .gate = if (comptime kind == .pure_decode) &gate else null };
            const definition = fx.dsl.declareMetadata(.{ .kind = kind, .decode = if (kind == .pure_decode) PureCallbacks.decode else PureCallbacks.contextualDecode, .encode = if (kind == .pure_encode) PureCallbacks.encode else PureCallbacks.contextualEncode });
            var first_description = fx.dsl.makeCodec(&creation, .{ .definition = definition, .inputs = Cfg{ .owner = try Configuration.init(std.testing.allocator, &active), .offset = 3, .calls = &calls } });
            var second_description = fx.dsl.makeCodec(&creation, .{ .definition = definition, .inputs = Cfg{ .owner = try Configuration.init(std.testing.allocator, &foreign), .offset = 4, .calls = &calls } });
            var first = fx.run(&creation, first_description);
            var second = fx.run(&creation, second_description);
            var input = if (comptime kind == .pure_decode) second.success.value(.{ .text = "independent input", .number = 7 }) else second.success.representation(7);
            var program = if (comptime kind == .pure_decode) first.success.encode(&input) else first.success.decode(&input);
            var pending = if (comptime kind == .pure_decode) fx.runAsync(&en, program) else fx.runAsync(&de, program);
            gate.started.waitUncancelable(std.testing.io);
            program.deinit();
            first.deinit();
            first_description.deinit();
            try std.testing.expectEqual(@as(usize, 1), active.load(.acquire));
            if (comptime destroy) {
                pending.deinit();
                try std.testing.expect(gate.cancelled.load(.acquire));
                try std.testing.expectEqual(@as(usize, 0), active.load(.acquire));
            } else {
                gate.proceed.set(std.testing.io);
                var result = pending.join();
                pending.deinit();
                try std.testing.expectEqualStrings("captured codec configuration", result.fail.value.message);
                try std.testing.expectEqual(@as(usize, 1), active.load(.acquire));
                result.deinit();
                try std.testing.expectEqual(@as(usize, 0), active.load(.acquire));
            }
            try std.testing.expect(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
            try std.testing.expectEqual(@as(usize, 1), foreign.load(.acquire));
            input.deinit();
            second.deinit();
            second_description.deinit();
            try std.testing.expectEqual(@as(usize, 0), foreign.load(.acquire));
        }
    }
}

test "CODEC-04 CODEC-07 captured explicit writer uses native bounds and retains configuration through failures cleanup metadata and late cancellation" {
    inline for (std.meta.tags(WriterMode)) |mode| {
        var live: std.atomic.Value(usize) = .init(0);
        var calls: usize = 0;
        var cancellation: fx.Cancellation = .{};
        const creation: Construction = .{ .allocator = std.testing.allocator };
        const env: WriterEnv = .{ .allocator = std.testing.allocator, .max_output = if (mode == .invalid) 0 else if (mode == .limited) 1 else 64, .cancellation = &cancellation };
        var description = fx.dsl.makeCodec(&creation, .{ .definition = fx.dsl.declareMetadata(.{ .kind = fx.dsl.CodecKind.pure, .decode = WriterCallbacks.decode, .encode = WriterCallbacks.encode, .writer = WriterCallbacks.write }), .inputs = .{ .owner = try Configuration.init(std.testing.allocator, &live), .offset = @as(i64, 3), .calls = &calls, .mode = mode, .cancellation = &cancellation } });
        var made = fx.run(&creation, description);
        const Codec = @TypeOf(made).Success;
        comptime if (!Codec.has_explicit_writer) @compileError("explicit validated writer evidence must be preserved");
        var input = made.success.value(39);
        var program = made.success.writeWith(&input, WriterCallbacks.limits);
        comptime if (@TypeOf(program) != fx.Effect(fx.OwnedSlice(u8), Codec.Error, WriterEnv)) @compileError("writer must infer its caller environment and return exact Effect");
        try std.testing.expectEqual(@as(usize, 0), calls);
        var result = fx.run(&env, program);
        errdefer result.deinit();
        program.deinit();
        input.deinit();
        made.deinit();
        description.deinit();
        switch (mode) {
            .normal => {
                try std.testing.expectEqualStrings("42", result.success.items);
                try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
            },
            .failure => {
                try std.testing.expectEqualStrings("captured codec configuration", result.fail.value.kind.custom);
                try std.testing.expectEqual(@as(usize, 1), live.load(.acquire));
            },
            .cleanup => {
                try std.testing.expectEqualStrings("captured codec configuration", result.finalization_failed.primary.failed.value.kind.custom);
                try std.testing.expectEqualStrings("captured codec configuration", result.finalization_failed.first.release_failure.value.kind.custom);
                try std.testing.expectEqual(@as(usize, 1), live.load(.acquire));
            },
            .limited => try std.testing.expect(result.fail.value.kind == .size_limit),
            .invalid => try std.testing.expect(result.fail.value.kind == .invalid_value),
            .late_cancel => try std.testing.expect(result == .interrupted),
        }
        try std.testing.expectEqual(@as(usize, if (mode == .invalid) 0 else 1), calls);
        result.deinit();
        result = .interrupted;
        try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
    }
}

test "CODEC-04 CODEC-07 captured direct writer copies bytes through the native Writer and retains a borrowed failure after codec teardown" {
    inline for (.{ WriterMode.normal, WriterMode.failure, WriterMode.cleanup }) |mode| {
        var live: std.atomic.Value(usize) = .init(0);
        var calls: usize = 0;
        var cancellation: fx.Cancellation = .{};
        const creation: Construction = .{ .allocator = std.testing.allocator };
        var description = fx.dsl.makeCodec(&creation, .{ .definition = fx.dsl.declareMetadata(.{ .kind = fx.dsl.CodecKind.pure, .decode = WriterCallbacks.decode, .encode = WriterCallbacks.encode, .writer = WriterCallbacks.write }), .inputs = .{ .owner = try Configuration.init(std.testing.allocator, &live), .offset = @as(i64, 3), .calls = &calls, .mode = mode, .cancellation = &cancellation } });
        var made = fx.run(&creation, description);
        var input = made.success.value(39);
        const env: WriterEnv = .{ .allocator = std.testing.allocator, .max_output = 64, .cancellation = &cancellation };
        var writer = fx.JsonWriter.Writer.init(&env, WriterCallbacks.limits(&env));
        defer writer.deinit();
        var result = made.success.writeJson(&input, &writer);
        input.deinit();
        made.deinit();
        description.deinit();
        if (mode == .normal) {
            try std.testing.expect(result == .success);
            var output = try writer.finish();
            defer output.deinit();
            try std.testing.expectEqualStrings("42", output.items);
            try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
        } else if (mode == .failure) {
            try std.testing.expectEqualStrings("captured codec configuration", result.fail.value.kind.custom);
            try std.testing.expectEqual(@as(usize, 1), live.load(.acquire));
        } else {
            try std.testing.expectEqualStrings("captured codec configuration", result.finalization_failed.primary.failed.value.kind.custom);
            try std.testing.expectEqualStrings("captured codec configuration", result.finalization_failed.first.release_failure.value.kind.custom);
        }
        result.deinit();
        try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
    }
}

fn writerAllocationFailure(result: anytype) bool {
    if (result == .die) return result.die.kind == .out_of_memory;
    if (result != .finalization_failed) return false;
    const detail = result.finalization_failed;
    if (detail.primary == .defected and detail.primary.defected.kind == .out_of_memory) return true;
    if (detail.first == .release_defect and detail.first.release_defect.kind == .out_of_memory) return true;
    var cursor = detail.rest;
    while (cursor) |node| {
        if (node.failure == .release_defect and node.failure.release_defect.kind == .out_of_memory) return true;
        cursor = node.next;
    }
    return false;
}

test "CODEC-04 CODEC-07 LIFE-07 every captured writer allocation failure preserves native output error cleanup and cancellation ownership" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator, mode: WriterMode) !void {
            var live: std.atomic.Value(usize) = .init(0);
            defer std.debug.assert(live.load(.acquire) == 0);
            var calls: usize = 0;
            var cancellation: fx.Cancellation = .{};
            const creation: Construction = .{ .allocator = allocator };
            const env: WriterEnv = .{ .allocator = allocator, .max_output = if (mode == .invalid) 0 else if (mode == .limited) 1 else 64, .cancellation = &cancellation };
            var description = fx.dsl.makeCodec(&creation, .{ .definition = fx.dsl.declareMetadata(.{ .kind = fx.dsl.CodecKind.pure, .decode = WriterCallbacks.decode, .encode = WriterCallbacks.encode, .writer = WriterCallbacks.write }), .inputs = .{ .owner = try Configuration.init(allocator, &live), .offset = @as(i64, 3), .calls = &calls, .mode = mode, .cancellation = &cancellation } });
            defer description.deinit();
            var made = fx.run(&creation, description);
            defer made.deinit();
            if (writerAllocationFailure(made)) return error.OutOfMemory;
            try std.testing.expect(made == .success);
            var input = made.success.value(39);
            defer input.deinit();
            var program = made.success.writeWith(&input, WriterCallbacks.limits);
            defer program.deinit();
            var result = fx.run(&env, program);
            defer result.deinit();
            if (writerAllocationFailure(result)) return error.OutOfMemory;
            switch (mode) {
                .normal => try std.testing.expectEqualStrings("42", result.success.items),
                .failure => try std.testing.expectEqualStrings("captured codec configuration", result.fail.value.kind.custom),
                .cleanup => {
                    try std.testing.expectEqualStrings("captured codec configuration", result.finalization_failed.primary.failed.value.kind.custom);
                    try std.testing.expectEqualStrings("captured codec configuration", result.finalization_failed.first.release_failure.value.kind.custom);
                },
                .limited => try std.testing.expect(result.fail.value.kind == .size_limit),
                .invalid => try std.testing.expect(result.fail.value.kind == .invalid_value),
                .late_cancel => try std.testing.expect(result == .interrupted),
            }
        }
    };
    inline for (std.meta.tags(WriterMode)) |mode|
        try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{mode});
}

test "CODEC-04 CODEC-07 LIFE-11 captured writer separates construction storage from fresh caller writer allocations and independent output owners" {
    const Audit = @import("support/memory_audit.zig").MemoryAudit;
    var storage: Audit = .{};
    var first: Audit = .{};
    var second: Audit = .{};
    var live: std.atomic.Value(usize) = .init(0);
    var calls: usize = 0;
    var cancellation: fx.Cancellation = .{};
    const creation: Construction = .{ .allocator = storage.allocator() };
    var description = fx.dsl.makeCodec(&creation, .{ .definition = fx.dsl.declareMetadata(.{ .kind = fx.dsl.CodecKind.pure, .decode = WriterCallbacks.decode, .encode = WriterCallbacks.encode, .writer = WriterCallbacks.write }), .inputs = .{ .owner = try Configuration.init(storage.allocator(), &live), .offset = @as(i64, 3), .calls = &calls, .mode = WriterMode.normal, .cancellation = &cancellation } });
    var made = fx.run(&creation, description);
    var input = made.success.value(39);
    var program = made.success.writeWith(&input, WriterCallbacks.limits);
    const before = storage.allocations;
    const retained_storage = storage.live;
    const a: WriterEnv = .{ .allocator = first.allocator(), .max_output = 64, .cancellation = &cancellation };
    const b: WriterEnv = .{ .allocator = second.allocator(), .max_output = 64, .cancellation = &cancellation };
    var output_a = fx.run(&a, program);
    var output_b = fx.run(&b, program);
    // One temporary kernel retention link per run follows the description
    // allocator; execution frames and output buffers follow each caller R.
    try std.testing.expectEqual(before + 2, storage.allocations);
    try std.testing.expectEqual(retained_storage, storage.live);
    try std.testing.expect(output_a.success.allocator.ptr == a.allocator.ptr);
    try std.testing.expect(output_b.success.allocator.ptr == b.allocator.ptr);
    program.deinit();
    input.deinit();
    made.deinit();
    description.deinit();
    try std.testing.expectEqual(@as(usize, 0), storage.live);
    try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
    try std.testing.expectEqual(@as(usize, 2), calls);
    try std.testing.expectEqualStrings("42", output_a.success.items);
    try std.testing.expectEqualStrings("42", output_b.success.items);
    output_a.deinit();
    output_b.deinit();
    try std.testing.expectEqual(@as(usize, 0), first.live);
    try std.testing.expectEqual(@as(usize, 0), second.live);
}

test "CODEC-03 CODEC-04 captured direct validation invokes one pure encoder disposes its owning representation and retains borrowed errors" {
    const Rep = struct {
        pub const effect_owned_payload = true;
        value: usize,
        live: *usize,
        configuration: *std.atomic.Value(usize),
        pub fn deinit(self: *@This()) void {
            std.debug.assert(self.configuration.load(.acquire) == 1);
            self.live.* -= 1;
        }
    };
    const Functions = struct {
        fn decode(_: anytype, input: *const Rep) fx.Result(usize, Error) {
            return .{ .ok = input.value };
        }
        fn contextualDecode(_: anytype, input: *const Rep, _: *const Construction) fx.Effect(usize, Error, Construction) {
            return fx.Effect(usize, Error, Construction).succeed(input.value);
        }
        fn encode(configuration: anytype, input: *const usize) fx.Result(Rep, Error) {
            configuration.calls.* += 1;
            if (input.* == 0) return .{ .err = .{ .message = configuration.owner.bytes } };
            configuration.representations.* += 1;
            return .{ .ok = .{ .value = input.*, .live = configuration.representations, .configuration = configuration.owner.live } };
        }
    };
    inline for (.{ fx.dsl.CodecKind.pure, fx.dsl.CodecKind.pure_encode }) |kind| {
        var live: std.atomic.Value(usize) = .init(0);
        var calls: usize = 0;
        var representations: usize = 0;
        const creation: Construction = .{ .allocator = std.testing.allocator };
        var description = fx.dsl.makeCodec(&creation, .{ .definition = fx.dsl.declareMetadata(.{ .kind = kind, .decode = if (kind == .pure) Functions.decode else Functions.contextualDecode, .encode = Functions.encode }), .inputs = .{ .owner = try Configuration.init(std.testing.allocator, &live), .calls = &calls, .representations = &representations } });
        var made = fx.run(&creation, description);
        var value = made.success.value(7);
        var checked = made.success.validateDirect(&value);
        try std.testing.expect(checked == .ok);
        checked.deinit();
        try std.testing.expectEqual(@as(usize, 1), calls);
        try std.testing.expectEqual(@as(usize, 0), representations);
        value.value = 0;
        var rejected = made.success.validateDirect(&value);
        try std.testing.expectEqual(@as(usize, 2), calls);
        value.deinit();
        made.deinit();
        description.deinit();
        try std.testing.expectEqualStrings("captured codec configuration", rejected.err.value.message);
        try std.testing.expectEqual(@as(usize, 1), live.load(.acquire));
        rejected.deinit();
        try std.testing.expectEqual(@as(usize, 0), live.load(.acquire));
    }
}
