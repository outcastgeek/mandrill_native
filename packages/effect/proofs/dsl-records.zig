const std = @import("std");
const fx = @import("effect");
const gates = @import("support/dsl_gate.zig");
const Threads = @import("support/thread_executor.zig").ThreadExecutor;
const Issue = fx.SchemaIssue.Issue;
const DefaultMode = enum { success, fail, die, interrupted };
const Env = struct { allocator: std.mem.Allocator, defaults: *usize, live: ?*usize = null, default_mode: DefaultMode = .success, invalid_default: bool = false };
const policy: fx.SchemaBuilder.RecordPolicy = .{ .unknown_fields = .reject, .duplicate_fields = .reject, .issues = .first };
const limits: fx.SchemaBuilder.ParseLimits = .{ .max_document_bytes = 256, .max_depth = 8, .max_value_bytes = 128, .max_integer_digits = 16 };
fn defaultCount(env: *const Env) i64 {
    env.defaults.* += 1;
    return 9;
}

const DefaultBytes = struct {
    pub const effect_owned_payload = true;
    bytes: fx.OwnedSlice(u8),
    live: *usize,
    valid: bool,
    pub fn deinit(self: *@This()) void {
        std.debug.assert(self.live.* > 0);
        self.live.* -= 1;
        self.bytes.deinit();
    }
};
const defaultCodec = fx.PureCodec(DefaultBytes, std.json.Value).init(struct {
    fn decode(_: *const std.json.Value) fx.Result(DefaultBytes, Issue) {
        return .{ .err = Issue.pure(.decode, .invalid_value) };
    }
}.decode, struct {
    fn encode(value: *const DefaultBytes) fx.Result(std.json.Value, Issue) {
        if (!value.valid) return .{ .err = Issue.pure(.encode, .invalid_value) };
        return .{ .ok = .{ .string = value.bytes.items } };
    }
}.encode);
fn freshDefault(env: *const Env) fx.Exit(DefaultBytes, Issue) {
    env.defaults.* += 1;
    const bytes = env.allocator.dupe(u8, "fresh default") catch return .{ .die = fx.Defect.out_of_memory };
    env.live.?.* += 1;
    return .{ .success = .{ .bytes = .{ .items = bytes, .allocator = env.allocator }, .live = env.live.?, .valid = !env.invalid_default } };
}
fn laterDefault(env: *const Env) fx.Exit(i64, Issue) {
    return switch (env.default_mode) {
        .success => .{ .success = 7 },
        .fail => .{ .fail = Issue.init(env, .decode, .invalid_value) },
        .die => .{ .die = fx.Defect.init("default defect") },
        .interrupted => .interrupted,
    };
}

fn JsonObject(comptime W: type) type {
    return struct {
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
        pub fn init(env: *const W, _: usize) fx.Exit(Builder, Issue) {
            return .{ .success = .{ .allocator = env.allocator } };
        }
        pub fn put(_: *const W, builder: *Builder, name: []const u8, value: *const Representation) fx.Exit(void, Issue) {
            builder.values.put(builder.allocator, name, value.*) catch return .{ .die = fx.Defect.out_of_memory };
            return .{ .success = {} };
        }
        pub fn finish(_: *const W, builder: *Builder) fx.Exit(Representation, Issue) {
            const values = builder.values;
            builder.values = .{};
            return .{ .success = .{ .object = values } };
        }
        pub fn discard(builder: *Builder) void {
            builder.values.deinit(builder.allocator);
        }
    };
}

test "CODEC-01 INF-07 inferred JSON record fields preserve required optional default presence and existing schema methods" {
    const Record = comptime fx.dsl.schemaRecord(Env, .{ .fields = .{
        .answer = .{ .codec = fx.JsonCodec.signed_integer, .presence = .required },
        .name = .{ .codec = fx.JsonCodec.borrowed_string, .presence = .optional },
        .count = .{ .codec = fx.JsonCodec.signed_integer, .presence = .defaulted, .default = defaultCount },
    }, .policy = policy });
    comptime if (@FieldType(Record.Decoded, "answer") != i64 or @FieldType(Record.Decoded, "name") != ?[]const u8 or @FieldType(Record.Decoded, "count") != i64) @compileError("record field value types must derive from actual codecs and presence");
    var defaults: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .defaults = &defaults };
    var object: std.json.ObjectMap = .{};
    defer object.deinit(env.allocator);
    try object.put(env.allocator, "answer", .{ .integer = 41 });
    var input: std.json.Value = .{ .object = object };
    var result = fx.run(&env, Record.decode(&input));
    defer result.deinit();
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(i64, 41), Record.get("answer", &result.success).*);
    try std.testing.expect(Record.get("name", &result.success).* == null);
    try std.testing.expectEqual(@as(i64, 9), Record.get("count", &result.success).*);
    try std.testing.expectEqual(@as(usize, 1), defaults);
    var encoded = fx.run(&env, Record.encodeBytes(&result.success));
    defer encoded.deinit();
    try std.testing.expect(encoded == .success);
    try std.testing.expectEqualStrings("{\"answer\":41,\"count\":9}", encoded.success.items);
    try object.put(env.allocator, "count", .{ .integer = 3 });
    input = .{ .object = object };
    var present = fx.run(&env, Record.decode(&input));
    defer present.deinit();
    try std.testing.expect(present == .success and Record.get("count", &present.success).* == 3);
    try std.testing.expectEqual(@as(usize, 1), defaults);
}

test "CODEC-01 CAP-08 inferred preserving JSON record generates extension ownership without outer cleanup code" {
    const Record = comptime fx.dsl.schemaRecord(Env, .{ .fields = .{
        .answer = .{ .codec = fx.JsonCodec.signed_integer, .presence = .required },
        .extras = .{ .extensions = true },
    }, .policy = .{ .unknown_fields = .preserve, .duplicate_fields = .reject, .issues = .first } });
    var defaults: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .defaults = &defaults };
    var object: std.json.ObjectMap = .{};
    defer object.deinit(env.allocator);
    try object.put(env.allocator, "answer", .{ .integer = 41 });
    try object.put(env.allocator, "extra", .{ .string = "retained by parsed document" });
    const input: std.json.Value = .{ .object = object };
    var result = fx.run(&env, Record.decode(&input));
    defer result.deinit();
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(i64, 41), Record.get("answer", &result.success).*);
    try std.testing.expectEqualStrings("retained by parsed document", Record.get("extras", &result.success).values.get("extra").?.string);
    var encoded = fx.run(&env, Record.encodeBytes(&result.success));
    defer encoded.deinit();
    try std.testing.expect(encoded == .success);
    try std.testing.expectEqualStrings("{\"answer\":41,\"extra\":\"retained by parsed document\"}", encoded.success.items);
}

test "CODEC-01 CODEC-02 existing custom owned records retain a direct value field containing an untagged union" {
    const Inner = struct { raw: union { a: u8, b: u16 } };
    const Value = struct {
        pub const effect_owned_payload = true;
        value: Inner,
        pub fn deinit(_: *@This()) void {}
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            return self.*;
        }
    };
    const C = comptime fx.PureCodec(Inner, std.json.Value).init(struct {
        fn decode(input: *const std.json.Value) fx.Result(Inner, Issue) {
            return .{ .ok = .{ .raw = .{ .b = @intCast(input.integer) } } };
        }
    }.decode, struct {
        fn encode(value: *const Inner) fx.Result(std.json.Value, Issue) {
            return .{ .ok = .{ .integer = value.raw.b } };
        }
    }.encode);
    const Schema = fx.SchemaBuilder.Record(Value, Env, .{fx.SchemaBuilder.required("value", C)}, policy);
    const Native = fx.NativeCodec.Record(Value, JsonObject(Env), Env, Env, .{fx.NativeField.required("value", C)}, true);
    var defaults: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .defaults = &defaults };
    var object: std.json.ObjectMap = .{};
    defer object.deinit(env.allocator);
    try object.put(env.allocator, "value", .{ .integer = 42 });
    const input: std.json.Value = .{ .object = object };
    var schema = fx.run(&env, Schema.decode(&input));
    defer schema.deinit();
    try std.testing.expect(schema == .success and schema.success.value.raw.b == 42);
    try std.testing.expectEqual(@as(u16, 42), Schema.get("value", &schema.success).raw.b);
    var native = fx.run(&env, Native.decode(&input));
    defer native.deinit();
    try std.testing.expect(native == .success and native.success.value.raw.b == 42);
    var encoded = fx.run(&env, Native.encode(&native.success));
    defer encoded.deinit();
    try std.testing.expect(encoded == .success);
    defer encoded.success.object.deinit(env.allocator);
    try std.testing.expectEqual(@as(i64, 42), encoded.success.object.get("value").?.integer);
}

test "CODEC-02 INF-07 native record derives distinct direction services and default representation factories" {
    const D = struct { allocator: std.mem.Allocator, increment: i64, defaults: *usize };
    const W = struct { allocator: std.mem.Allocator, offset: i64 };
    const Roles = struct {
        fn decode(input: *const std.json.Value) fx.Effect(i64, Issue, D) {
            return fx.Effect(i64, Issue, D).fromBorrowed(std.json.Value, input, struct {
                fn run(value: *const std.json.Value, env: *const D) fx.Exit(i64, Issue) {
                    if (value.* != .integer) return .{ .fail = Issue.init(env, .decode, .invalid_value) };
                    return .{ .success = value.integer + env.increment };
                }
            }.run);
        }
        fn encode(value: *const i64) fx.Effect(std.json.Value, Issue, W) {
            return fx.Effect(std.json.Value, Issue, W).fromBorrowed(i64, value, struct {
                fn run(input: *const i64, env: *const W) fx.Exit(std.json.Value, Issue) {
                    return .{ .success = .{ .integer = input.* + env.offset } };
                }
            }.run);
        }
        fn default(env: *const D) std.json.Value {
            env.defaults.* += 1;
            return .{ .integer = 7 };
        }
    };
    const C = comptime fx.dsl.codec(.{ .kind = .contextual, .decode = Roles.decode, .encode = Roles.encode });
    const Record = fx.dsl.nativeRecord(JsonObject(W), .{ .fields = .{
        .answer = .{ .codec = C, .presence = .required },
        .name = .{ .codec = fx.JsonCodec.borrowed_string, .presence = .optional },
        .count = .{ .codec = fx.JsonCodec.signed_integer, .presence = .defaulted, .default = Roles.default },
    }, .reject_unknown = true });
    comptime if (Record.DecodingEnvironment != D or Record.EncodingEnvironment != W) @compileError("native record must preserve independent services inferred from its actual callbacks");
    var defaults: usize = 0;
    const d: D = .{ .allocator = std.testing.allocator, .increment = 1, .defaults = &defaults };
    const w: W = .{ .allocator = std.testing.allocator, .offset = 2 };
    var object: std.json.ObjectMap = .{};
    defer object.deinit(d.allocator);
    try object.put(d.allocator, "answer", .{ .integer = 41 });
    const input: std.json.Value = .{ .object = object };
    const decoding = Record.decode(&input);
    comptime if (@TypeOf(decoding) != fx.Effect(Record.Decoded, Issue, D)) @compileError("native decoding must expose exact Effect");
    var result = fx.run(&d, decoding);
    defer result.deinit();
    try std.testing.expect(result == .success and result.success.answer == 42 and result.success.name == null and result.success.count == 7);
    try std.testing.expectEqual(@as(usize, 1), defaults);
    const encoding = Record.encode(&result.success);
    comptime if (@TypeOf(encoding) != fx.Effect(std.json.Value, Issue, W)) @compileError("native encoding must expose exact Effect");
    var encoded = fx.run(&w, encoding);
    defer encoded.deinit();
    try std.testing.expect(encoded == .success);
    defer encoded.success.object.deinit(w.allocator);
    try std.testing.expectEqual(@as(i64, 44), encoded.success.object.get("answer").?.integer);
    try std.testing.expectEqual(@as(i64, 7), encoded.success.object.get("count").?.integer);
    try std.testing.expect(!encoded.success.object.contains("name"));
}

test "CODEC-01 inferred record retains duplicate unknown nullable and absent field policies with bounded parsing" {
    var defaults: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .defaults = &defaults };
    inline for (.{ .reject, .first_wins, .last_wins }) |duplicate| {
        const Record = fx.dsl.schemaRecord(Env, .{ .fields = .{
            .answer = .{ .codec = fx.JsonCodec.signed_integer, .presence = .required },
            .maybe = .{ .codec = fx.JsonCodec.nullable(fx.JsonCodec.signed_integer), .presence = .optional },
        }, .policy = .{ .unknown_fields = .ignore, .duplicate_fields = duplicate, .issues = .first } });
        var parsed = Record.parseBytesDirect(&env, "{\"answer\":1,\"answer\":2,\"extra\":3}", limits);
        defer parsed.deinit();
        if (duplicate == .reject) {
            try std.testing.expect(parsed == .fail and parsed.fail.kind == .duplicate_field);
        } else {
            try std.testing.expect(parsed == .success);
            try std.testing.expectEqual(@as(i64, if (duplicate == .first_wins) 1 else 2), Record.get("answer", &parsed.success.value).*);
            try std.testing.expect(Record.get("maybe", &parsed.success.value).* == null);
        }
        var nullable = Record.parseBytesDirect(&env, "{\"answer\":1,\"maybe\":null}", limits);
        defer nullable.deinit();
        try std.testing.expect(nullable == .success);
        try std.testing.expect(Record.get("maybe", &nullable.success.value).* != null);
        try std.testing.expect(Record.get("maybe", &nullable.success.value).*.? == null);
        var limited = Record.parseBytesDirect(&env, "{\"answer\":1}", .{ .max_document_bytes = 8, .max_depth = 8, .max_value_bytes = 128 });
        defer limited.deinit();
        try std.testing.expect(limited == .fail and limited.fail.kind == .size_limit);
    }
    const Reject = fx.dsl.schemaRecord(Env, .{ .fields = .{ .answer = .{ .codec = fx.JsonCodec.signed_integer, .presence = .required } }, .policy = policy });
    var unknown = Reject.parseBytesDirect(&env, "{\"answer\":1,\"extra\":3}", limits);
    defer unknown.deinit();
    try std.testing.expect(unknown == .fail and unknown.fail.kind == .unknown_field);
    try std.testing.expectEqualStrings("extra", unknown.fail.path.?[0].field.items);
}

test "CODEC-01 LIFE-11 parsed inferred record keeps borrowed fields and extensions after source buffer and first owner are released" {
    const Record = fx.dsl.schemaRecord(Env, .{ .fields = .{
        .name = .{ .codec = fx.JsonCodec.borrowed_string, .presence = .required },
        .extras = .{ .extensions = true },
    }, .policy = .{ .unknown_fields = .preserve, .duplicate_fields = .reject, .issues = .first } });
    var defaults: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .defaults = &defaults };
    const source = try env.allocator.dupe(u8, "{\"name\":\"retained\",\"extra\":\"also retained\"}");
    var parsed = Record.parseBytesDirect(&env, source, limits);
    env.allocator.free(source);
    try std.testing.expect(parsed == .success);
    var retained = try parsed.success.clone();
    parsed.deinit();
    defer retained.deinit();
    try std.testing.expectEqualStrings("retained", Record.get("name", &retained.value).*);
    try std.testing.expectEqualStrings("also retained", Record.get("extras", &retained.value).values.get("extra").?.string);
    var encoded = fx.run(&env, Record.encodeBytes(&retained.value));
    defer encoded.deinit();
    try std.testing.expect(encoded == .success);
    try std.testing.expectEqualStrings("{\"name\":\"retained\",\"extra\":\"also retained\"}", encoded.success.items);
}

test "CODEC-06 LIFE-07 inferred preserving record cleans document fields extension metadata and encoded bytes at every allocation failure" {
    const Record = fx.dsl.schemaRecord(Env, .{ .fields = .{
        .name = .{ .codec = fx.JsonCodec.borrowed_string, .presence = .required },
        .extras = .{ .extensions = true },
    }, .policy = .{ .unknown_fields = .preserve, .duplicate_fields = .reject, .issues = .first } });
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            var defaults: usize = 0;
            const env: Env = .{ .allocator = allocator, .defaults = &defaults };
            var parsed = Record.parseBytesDirect(&env, "{\"name\":\"retained\",\"extra\":\"also retained\"}", limits);
            defer parsed.deinit();
            if (parsed == .die and parsed.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expect(parsed == .success);
            var encoded = fx.run(&env, Record.encodeBytes(&parsed.success.value));
            defer encoded.deinit();
            if (encoded == .die and encoded.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expect(encoded == .success);
            try std.testing.expectEqualStrings("{\"name\":\"retained\",\"extra\":\"also retained\"}", encoded.success.items);
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

test "CODEC-06 CAP-08 owned defaults move into generated record storage and release on later failure defect interruption or failed validation" {
    const Record = fx.dsl.schemaRecord(Env, .{ .fields = .{
        .first = .{ .codec = defaultCodec, .presence = .defaulted, .default = freshDefault },
        .later = .{ .codec = fx.JsonCodec.signed_integer, .presence = .defaulted, .default = laterDefault },
    }, .policy = policy });
    inline for (.{ .success, .fail, .die, .interrupted }) |mode| {
        var defaults: usize = 0;
        var live: usize = 0;
        const env: Env = .{ .allocator = std.testing.allocator, .defaults = &defaults, .live = &live, .default_mode = mode };
        var object: std.json.ObjectMap = .{};
        defer object.deinit(env.allocator);
        const input: std.json.Value = .{ .object = object };
        var result = fx.run(&env, Record.decode(&input));
        try std.testing.expectEqual(@as(usize, 1), defaults);
        switch (@as(DefaultMode, mode)) {
            .success => {
                try std.testing.expect(result == .success);
                try std.testing.expectEqual(@as(usize, 1), live);
                try std.testing.expectEqualStrings("fresh default", Record.get("first", &result.success).bytes.items);
                try std.testing.expectEqual(@as(i64, 7), Record.get("later", &result.success).*);
            },
            .fail => try std.testing.expect(result == .fail),
            .die => try std.testing.expect(result == .die),
            .interrupted => try std.testing.expect(result == .interrupted),
        }
        if (mode != .success) try std.testing.expectEqual(@as(usize, 0), live);
        result.deinit();
        try std.testing.expectEqual(@as(usize, 0), live);
    }
    var defaults: usize = 0;
    var live: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .defaults = &defaults, .live = &live, .invalid_default = true };
    const input: std.json.Value = .{ .object = .{} };
    var invalid = fx.run(&env, Record.decode(&input));
    defer invalid.deinit();
    try std.testing.expect(invalid == .fail and invalid.fail.direction == .decode);
    try std.testing.expectEqualStrings("first", invalid.fail.path.?[0].field.items);
    try std.testing.expectEqual(@as(usize, 0), live);
}

test "CODEC-06 LIFE-07 owned default and parsed record unwind partial fields at every allocation failure" {
    const Record = fx.dsl.schemaRecord(Env, .{ .fields = .{
        .first = .{ .codec = defaultCodec, .presence = .defaulted, .default = freshDefault },
        .later = .{ .codec = fx.JsonCodec.signed_integer, .presence = .required },
    }, .policy = policy });
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            inline for (.{ "{\"later\":7}", "{\"later\":true}" }) |bytes| {
                var defaults: usize = 0;
                var live: usize = 0;
                defer std.debug.assert(live == 0);
                const env: Env = .{ .allocator = allocator, .defaults = &defaults, .live = &live };
                var parsed = Record.parseBytesDirect(&env, bytes, limits);
                defer parsed.deinit();
                if (parsed == .die and parsed.die.kind == .out_of_memory) return error.OutOfMemory;
                if (comptime std.mem.eql(u8, bytes, "{\"later\":true}")) {
                    try std.testing.expect(parsed == .fail);
                    try std.testing.expectEqual(@as(usize, 0), live);
                } else {
                    try std.testing.expect(parsed == .success);
                    try std.testing.expectEqual(@as(usize, 1), live);
                    var encoded = fx.run(&env, Record.encodeBytes(&parsed.success.value));
                    defer encoded.deinit();
                    if (encoded == .die and encoded.die.kind == .out_of_memory) return error.OutOfMemory;
                    try std.testing.expect(encoded == .success);
                    try std.testing.expectEqualStrings("{\"first\":\"fresh default\",\"later\":7}", encoded.success.items);
                }
            }
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

test "CODEC-02 CODEC-06 LIFE-11 pending native record retains initialized owned fields through join and transfers them beyond frame teardown" {
    const Gate = gates.Gate(i64, Issue);
    const D = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, async_engine: fx.Executor, defaults: *usize, live: *usize, gate: *Gate };
    // The existing reusable Future.join clones its result. Give this proof's
    // future value that contract; synchronous default proofs remain exclusive.
    const PendingBytes = struct {
        pub const effect_owned_payload = true;
        storage: DefaultBytes,
        pub fn deinit(self: *@This()) void {
            self.storage.deinit();
        }
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            const bytes = try self.storage.bytes.clone();
            self.storage.live.* += 1;
            return .{ .storage = .{ .bytes = bytes, .live = self.storage.live, .valid = self.storage.valid } };
        }
    };
    const Roles = struct {
        fn decodeFirst(input: *const std.json.Value) fx.Effect(PendingBytes, Issue, D) {
            return fx.Effect(PendingBytes, Issue, D).fromBorrowed(std.json.Value, input, struct {
                fn run(_: *const std.json.Value, env: *const D) fx.Exit(PendingBytes, Issue) {
                    const made = freshDefault(&.{ .allocator = env.allocator, .defaults = env.defaults, .live = env.live });
                    return switch (made) {
                        .success => |value| .{ .success = .{ .storage = value } },
                        .fail => |issue| .{ .fail = issue },
                        .die => |defect| .{ .die = defect },
                        .interrupted => .interrupted,
                        .finalization_failed => |details| .{ .finalization_failed = details },
                    };
                }
            }.run);
        }
        fn encodeFirst(value: *const PendingBytes) fx.Effect(std.json.Value, Issue, D) {
            return fx.Effect(std.json.Value, Issue, D).fromBorrowed(PendingBytes, value, struct {
                fn run(input: *const PendingBytes, _: *const D) fx.Exit(std.json.Value, Issue) {
                    return .{ .success = .{ .string = input.storage.bytes.items } };
                }
            }.run);
        }
        fn decodeLater(_: *const std.json.Value) fx.Effect(i64, Issue, D) {
            return fx.Effect(i64, Issue, D).fromAsync(struct {
                fn start(env: *const D) fx.Async(i64, Issue).Start {
                    return env.gate.start();
                }
            }.start);
        }
        fn encodeLater(value: *const i64) fx.Effect(std.json.Value, Issue, D) {
            return fx.Effect(std.json.Value, Issue, D).fromBorrowed(i64, value, struct {
                fn run(input: *const i64, _: *const D) fx.Exit(std.json.Value, Issue) {
                    return .{ .success = .{ .integer = input.* } };
                }
            }.run);
        }
    };
    const first = comptime fx.dsl.codec(.{ .kind = .contextual, .decode = Roles.decodeFirst, .encode = Roles.encodeFirst });
    const later = comptime fx.dsl.codec(.{ .kind = .contextual, .decode = Roles.decodeLater, .encode = Roles.encodeLater });
    const Record = fx.dsl.nativeRecord(JsonObject(D), .{ .fields = .{
        .first = .{ .codec = first, .presence = .required },
        .later = .{ .codec = later, .presence = .required },
    }, .reject_unknown = true });
    inline for (.{ false, true }) |cancel| {
        var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
        defer threads.deinit();
        var gate: Gate = .{ .result = .{ .success = 7 } };
        var defaults: usize = 0;
        var live: usize = 0;
        const env: D = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .defaults = &defaults, .live = &live, .gate = &gate };
        var object: std.json.ObjectMap = .{};
        try object.put(env.allocator, "first", .null);
        try object.put(env.allocator, "later", .{ .integer = 7 });
        const input: std.json.Value = .{ .object = object };
        var described = Record.decode(&input);
        var pending = fx.runAsync(&env, described);
        described.deinit();
        gate.started.waitUncancelable(env.io);
        try std.testing.expectEqual(@as(usize, 1), live);
        try std.testing.expectEqual(@as(usize, 1), defaults);
        if (cancel) pending.request() else gate.proceed.set(env.io);
        var result = pending.join();
        pending.deinit();
        object.deinit(env.allocator);
        try std.testing.expect(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
        if (cancel) {
            try std.testing.expect(result == .interrupted);
            try std.testing.expectEqual(@as(usize, 0), live);
        } else {
            try std.testing.expect(result == .success);
            try std.testing.expectEqual(@as(usize, 1), live);
            try std.testing.expectEqualStrings("fresh default", result.success.value.first.storage.bytes.items);
            try std.testing.expectEqual(@as(i64, 7), result.success.value.later);
        }
        result.deinit();
        try std.testing.expectEqual(@as(usize, 0), live);
    }
}

test "CODEC-02 CODEC-06 LIFE-07 native record cleans initialized owners and partial object builders across every allocation failure" {
    const Roles = struct {
        fn decodeFirst(_: *const std.json.Value) fx.Effect(DefaultBytes, Issue, Env) {
            return fx.Effect(DefaultBytes, Issue, Env).fromOutcome(freshDefault);
        }
        fn encodeFirst(value: *const DefaultBytes) fx.Effect(std.json.Value, Issue, Env) {
            return fx.Effect(std.json.Value, Issue, Env).fromBorrowed(DefaultBytes, value, struct {
                fn run(input: *const DefaultBytes, _: *const Env) fx.Exit(std.json.Value, Issue) {
                    return .{ .success = .{ .string = input.bytes.items } };
                }
            }.run);
        }
        fn decodeLater(input: *const std.json.Value) fx.Result(i64, Issue) {
            return fx.JsonCodec.signed_integer.decodeDirect(input);
        }
        fn encodeLater(input: *const i64) fx.Effect(std.json.Value, Issue, Env) {
            return fx.Effect(std.json.Value, Issue, Env).fromBorrowed(i64, input, struct {
                fn run(value: *const i64, env: *const Env) fx.Exit(std.json.Value, Issue) {
                    return switch (env.default_mode) {
                        .success => .{ .success = .{ .integer = value.* } },
                        .fail => .{ .fail = Issue.init(env, .encode, .invalid_value) },
                        .die => .{ .die = fx.Defect.init("native encode defect") },
                        .interrupted => .interrupted,
                    };
                }
            }.run);
        }
    };
    const first = comptime fx.dsl.codec(.{ .kind = .contextual, .decode = Roles.decodeFirst, .encode = Roles.encodeFirst });
    const later = comptime fx.dsl.codec(.{ .kind = .pure_decode, .decode = Roles.decodeLater, .encode = Roles.encodeLater });
    const Record = fx.dsl.nativeRecord(JsonObject(Env), .{ .fields = .{
        .first = .{ .codec = first, .presence = .required },
        .later = .{ .codec = later, .presence = .required },
    }, .reject_unknown = true });
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            inline for (.{ false, true }) |invalid_field| inline for (.{ .success, .fail, .die, .interrupted }) |mode| {
                var defaults: usize = 0;
                var live: usize = 0;
                defer std.debug.assert(live == 0);
                const env: Env = .{ .allocator = allocator, .defaults = &defaults, .live = &live, .default_mode = mode };
                var object: std.json.ObjectMap = .{};
                defer object.deinit(std.testing.allocator);
                try object.put(std.testing.allocator, "first", .null);
                try object.put(std.testing.allocator, "later", if (invalid_field) .{ .bool = true } else .{ .integer = 7 });
                const input: std.json.Value = .{ .object = object };
                var decoded = fx.run(&env, Record.decode(&input));
                defer decoded.deinit();
                if (decoded == .die and decoded.die.kind == .out_of_memory) return error.OutOfMemory;
                if (invalid_field) {
                    try std.testing.expect(decoded == .fail);
                    try std.testing.expectEqual(@as(usize, 0), live);
                    continue;
                }
                try std.testing.expect(decoded == .success);
                var encoded = fx.run(&env, Record.encode(&decoded.success));
                defer encoded.deinit();
                if (encoded == .die and encoded.die.kind == .out_of_memory) return error.OutOfMemory;
                switch (@as(DefaultMode, mode)) {
                    .success => {
                        try std.testing.expect(encoded == .success);
                        defer encoded.success.object.deinit(allocator);
                        try std.testing.expectEqualStrings("fresh default", encoded.success.object.get("first").?.string);
                        try std.testing.expectEqual(@as(i64, 7), encoded.success.object.get("later").?.integer);
                    },
                    .fail => try std.testing.expect(encoded == .fail),
                    .die => try std.testing.expect(encoded == .die),
                    .interrupted => try std.testing.expect(encoded == .interrupted),
                }
            };
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}

test "CODEC-06 CAP-08 generated default ownership adapts Exit composite values without cloning their exclusive owners" {
    const Value = struct { token: DefaultBytes };
    const Roles = struct {
        fn decode(_: *const std.json.Value) fx.Result(Value, Issue) {
            return .{ .err = Issue.pure(.decode, .invalid_value) };
        }
        fn encode(value: *const Value) fx.Result(std.json.Value, Issue) {
            return .{ .ok = .{ .integer = @intCast(value.token.bytes.items.len) } };
        }
        fn default(env: *const Env) fx.Exit(Value, Issue) {
            const made = freshDefault(env);
            return switch (made) {
                .success => |token| .{ .success = .{ .token = token } },
                .fail => |issue| .{ .fail = issue },
                .die => |defect| .{ .die = defect },
                .interrupted => .interrupted,
                .finalization_failed => |details| .{ .finalization_failed = details },
            };
        }
    };
    const C = comptime fx.dsl.codec(.{ .kind = .pure, .decode = Roles.decode, .encode = Roles.encode });
    const Record = fx.dsl.schemaRecord(Env, .{ .fields = .{
        .owned = .{ .codec = C, .presence = .defaulted, .default = Roles.default },
    }, .policy = policy });
    var defaults: usize = 0;
    var live: usize = 0;
    const env: Env = .{ .allocator = std.testing.allocator, .defaults = &defaults, .live = &live };
    const input: std.json.Value = .{ .object = .{} };
    const described = Record.decode(&input);
    try std.testing.expectEqual(@as(usize, 0), defaults);
    for (0..2) |_| {
        var result = fx.run(&env, described);
        try std.testing.expect(result == .success);
        try std.testing.expectEqualStrings("fresh default", Record.get("owned", &result.success).value.token.bytes.items);
        try std.testing.expectEqual(@as(usize, 1), live);
        result.deinit();
        try std.testing.expectEqual(@as(usize, 0), live);
    }
    try std.testing.expectEqual(@as(usize, 2), defaults);
}

test "CODEC-06 LIFE-01 native default retains owned finalization metadata while prior field owners unwind at every allocation failure" {
    const Later = struct {
        fn create(env: *const Env) fx.Exit(i64, Issue) {
            var primary = Issue.init(env, .decode, .invalid_value);
            primary.prependField("factory") catch {
                primary.deinit();
                return .{ .die = fx.Defect.out_of_memory };
            };
            var cleanup = Issue.init(env, .decode, .invalid_value);
            cleanup.prependField("release") catch {
                primary.deinit();
                cleanup.deinit();
                return .{ .die = fx.Defect.out_of_memory };
            };
            return .{ .finalization_failed = .{ .primary = .{ .failed = primary }, .first = .{ .release_failure = cleanup } } };
        }
    };
    const Record = fx.dsl.schemaRecord(Env, .{ .fields = .{
        .first = .{ .codec = defaultCodec, .presence = .defaulted, .default = freshDefault },
        .later = .{ .codec = fx.JsonCodec.signed_integer, .presence = .defaulted, .default = Later.create },
    }, .policy = policy });
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            var defaults: usize = 0;
            var live: usize = 0;
            const env: Env = .{ .allocator = allocator, .defaults = &defaults, .live = &live };
            const input: std.json.Value = .{ .object = .{} };
            var described = Record.decode(&input);
            var result = fx.run(&env, described);
            described.deinit();
            defer result.deinit();
            try std.testing.expectEqual(@as(usize, 0), live);
            if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expect(result == .finalization_failed);
            try std.testing.expect(result.finalization_failed.primary == .failed);
            try std.testing.expect(result.finalization_failed.first == .release_failure);
            try std.testing.expectEqualStrings("factory", result.finalization_failed.primary.failed.path.?[0].field.items);
            try std.testing.expectEqualStrings("release", result.finalization_failed.first.release_failure.path.?[0].field.items);
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}
