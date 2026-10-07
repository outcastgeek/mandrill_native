const std = @import("std");
const fx = @import("effect");
const Env = struct { allocator: std.mem.Allocator, io: std.Io };
const limits: fx.JsonWriter.Limits = .{ .max_output_bytes = 4096, .max_json_depth = 24, .max_value_bytes = 1024, .max_integer_digits = 64 };
fn select(_: *const Env) fx.JsonWriter.Limits {
    return limits;
}
fn environment() Env {
    return .{ .allocator = std.testing.allocator, .io = std.testing.io };
}

test "exact decimals preserve big integers and exact multipleOf with bounded issues" {
    const N = fx.JsonNumberCodec.Number;
    try std.testing.expectEqual(std.math.Order.eq, (try N.parse("1.20e2")).order(try N.parse("120")));
    try std.testing.expectEqual(std.math.Order.gt, (try N.parse("9007199254740993")).order(try N.parse("9007199254740992")));
    try std.testing.expectEqual(std.math.minInt(i64), try (try N.parse("-9223372036854775808")).toInteger(i64, 19));
    const env_for_arithmetic = environment();
    try std.testing.expect(try (try N.parse("1e1000000")).multipleOf(try N.parse("0.2"), &env_for_arithmetic, 64));
    try std.testing.expect(!try (try N.parse("1e-1000000")).multipleOf(try N.parse("0.2"), &env_for_arithmetic, 64));
    const Schema = fx.JsonNumberCodec.exact(Env, .{ .integer = false, .multiple_of = "0.1", .minimum = "0", .maximum_integer_digits = 32, .maximum_value_bytes = 128 });
    const Bounded = fx.JsonConstraints.withLimits(Schema, select);
    const env = environment();
    const input: std.json.Value = .{ .number_string = "9007199254740993.1" };
    var decoded = Bounded.decodeDirect(&env, &input);
    defer decoded.deinit();
    try std.testing.expect(decoded == .success);
    var encoded = Bounded.encodeBytesDirect(&env, &decoded.success);
    defer encoded.deinit();
    try std.testing.expect(encoded == .success);
    try std.testing.expectEqualStrings(input.number_string, encoded.success.items);
    const invalid: std.json.Value = .{ .number_string = "0.15" };
    var rejected = Schema.decodeDirect(&env, &invalid);
    defer rejected.deinit();
    try std.testing.expect(rejected == .fail and rejected.fail.detail != null);
}

test "Unicode writer validates encode direction and issue wire owns its path" {
    const env = environment();
    const S = fx.JsonConstraints.String(Env, .{ .minimum_scalars = 1, .maximum_scalars = 1, .format = .none }, null);
    const text: []const u8 = "é";
    var program = fx.JsonCodec.requireWriter(S).writeWith(Env, &text, select);
    defer program.deinit();
    var encoded = fx.run(&env, program);
    defer encoded.deinit();
    try std.testing.expect(encoded == .success);
    try std.testing.expectEqualStrings("\"é\"", encoded.success.items);
    const too_long: []const u8 = "ab";
    var rejected = S.validateDirect(&env, &too_long);
    defer rejected.deinit();
    try std.testing.expect(rejected == .fail and rejected.fail.direction == .encode);
    try rejected.fail.prependPropertyName("name");
    const issue_limits: fx.SchemaIssueCodec.Limits = .{ .input_bytes = 4096, .issue_depth = 8, .issue_count = 8, .json = limits };
    var wire = fx.SchemaIssueCodec.encodeDirect(&env, &rejected.fail, issue_limits);
    defer wire.deinit();
    try std.testing.expect(wire == .success);
    var restored = fx.SchemaIssueCodec.decodeDirect(&env, wire.success.items, issue_limits);
    defer restored.deinit();
    try std.testing.expect(restored == .success);
    try std.testing.expectEqualStrings("name", restored.success.path.?[0].property_name.items);
}

test "alternatives distinguish ambiguity and anyOf retains typed matches" {
    const env = environment();
    const branches = .{ fx.JsonCodec.signed_integer, fx.JsonCodec.signed_integer };
    const One = fx.JsonAlternatives.oneOf(Env, branches, .{ .maximum_issues = 4, .limits = limits });
    const Any = fx.JsonConstraints.withLimits(fx.JsonAlternatives.anyOf(Env, branches, .{ .maximum_issues = 4, .limits = limits }), select);
    const input: std.json.Value = .{ .integer = 42 };
    var rejected = One.decodeDirect(&env, &input);
    defer rejected.deinit();
    try std.testing.expect(rejected == .fail);
    try std.testing.expectEqual(@as(usize, 2), rejected.fail.detail.?.alternatives.items.len);
    var accepted = Any.decodeDirect(&env, &input);
    defer accepted.deinit();
    try std.testing.expect(accepted == .success);
    var copied = try accepted.success.clone();
    defer copied.deinit();
    var output = Any.encodeBytesDirect(&env, &copied);
    defer output.deinit();
    try std.testing.expectEqualStrings("42", output.success.items);
}

test "prefix uniqueness compares number values and object schemas encode named fields" {
    const env = environment();
    const A = fx.JsonConstraints.withLimits(fx.JsonConstraints.Array(Env, fx.JsonCodec.signed_integer, .{ .minimum = 1, .maximum = 4, .unique = true }), select);
    var array = std.json.Array.init(env.allocator);
    defer array.deinit();
    try array.append(.{ .integer = 1 });
    try array.append(.{ .integer = 1 });
    const input: std.json.Value = .{ .array = array };
    var invalid = A.decodeDirect(&env, &input);
    defer invalid.deinit();
    try std.testing.expect(invalid == .fail);
    const O = fx.JsonConstraints.withLimits(fx.JsonConstraints.Object(Env, .{.{ .name = "answer", .codec = fx.JsonCodec.signed_integer, .required = true }}, null, .{}, null, .{ .minimum_properties = 1, .maximum_properties = 2 }, null), select);
    var object: std.json.ObjectMap = .{};
    defer object.deinit(env.allocator);
    try object.put(env.allocator, "answer", .{ .integer = 42 });
    const json: std.json.Value = .{ .object = object };
    var decoded = O.decodeDirect(&env, &json);
    defer decoded.deinit();
    try std.testing.expect(decoded == .success);
    var encoded = O.encodeBytesDirect(&env, &decoded.success);
    defer encoded.deinit();
    try std.testing.expectEqualStrings("{\"answer\":42}", encoded.success.items);
}

const NativeObject = struct {
    pub const Representation = std.json.Value;
    pub const Builder = std.json.ObjectMap;
    pub fn count(input: *const Representation) usize {
        return if (input.* == .object) input.object.count() else 0;
    }
    pub fn nameAt(input: *const Representation, index: usize) []const u8 {
        return input.object.keys()[index];
    }
    pub fn find(input: *const Representation, name: []const u8) ?*const Representation {
        return if (input.* == .object) input.object.getPtr(name) else null;
    }
    pub fn init(_: *const Env, _: usize) fx.Exit(Builder, fx.SchemaIssue.Issue) {
        return .{ .success = .{} };
    }
    pub fn put(env: *const Env, builder: *Builder, name: []const u8, value: *const Representation) fx.Exit(void, fx.SchemaIssue.Issue) {
        builder.put(env.allocator, name, value.*) catch return .{ .die = fx.Defect.out_of_memory };
        return .{ .success = {} };
    }
    pub fn finish(_: *const Env, builder: *Builder) fx.Exit(Representation, fx.SchemaIssue.Issue) {
        const value = builder.*;
        builder.* = .{};
        return .{ .success = .{ .object = value } };
    }
    pub fn discard(builder: *Builder) void {
        builder.deinit(std.testing.allocator);
    }
};
test "native record executes pure children on caller trampoline and domain optional stays pure" {
    const Application = struct { answer: i64, optional: ?i64 };
    const Record = fx.NativeRecordBuilder(Application, NativeObject, Env, Env).fields(.{ fx.NativeField.required("answer", fx.JsonCodec.signed_integer), fx.NativeField.optional("optional", fx.JsonCodec.signed_integer) }).build(true);
    const env = environment();
    var object: std.json.ObjectMap = .{};
    defer object.deinit(env.allocator);
    try object.put(env.allocator, "answer", .{ .integer = 42 });
    const input: std.json.Value = .{ .object = object };
    var program = Record.decode(&input);
    defer program.deinit();
    var decoded = fx.run(&env, program);
    defer decoded.deinit();
    try std.testing.expect(decoded == .success and decoded.success.optional == null);
    var encoding = Record.encode(&decoded.success);
    defer encoding.deinit();
    var encoded = fx.run(&env, encoding);
    defer encoded.deinit();
    try std.testing.expect(encoded == .success);
    encoded.success.object.deinit(env.allocator);
    const E = enum { invalid };
    const mapped = comptime fx.DomainCodec.ofSchema(E, fx.JsonCodec.signed_integer, struct {
        fn map(issue: fx.SchemaIssue.Issue) E {
            var owned = issue;
            owned.deinit();
            return .invalid;
        }
    }.map, struct {
        fn map(issue: fx.SchemaIssue.Issue) E {
            var owned = issue;
            owned.deinit();
            return .invalid;
        }
    }.map);
    const optional = comptime fx.DomainCodec.optional(mapped);
    const value: ?i64 = 42;
    var validated = fx.requirePure(optional).validateDirect(&value);
    defer validated.deinit();
    try std.testing.expect(validated == .ok);
}

test "SafeAllocator owns decoded metadata and callback pubsub rejects after shutdown" {
    var checked = std.heap.SafeAllocator.init(std.testing.allocator, .{});
    {
        const env: Env = .{ .allocator = checked.allocator(), .io = std.testing.io };
        const Enumeration = fx.JsonConstraints.withLimits(fx.JsonConstraints.Enumeration(Env, &.{"known"}, .preserve), select);
        const input: std.json.Value = .{ .string = "future" };
        var value = Enumeration.decodeDirect(&env, &input);
        defer value.deinit();
        try std.testing.expect(value == .success and value.success.known_index == null);
        var callback = fx.CallbackPubSub(u32).init(&env, 2, 0, .reject);
        defer callback.success.deinit();
        try std.testing.expect(callback == .success);
        try std.testing.expect(callback.success.publish(42));
        callback.success.shutdown();
        try std.testing.expect(!callback.success.publish(43));
    }
    try std.testing.expectEqual(@as(usize, 0), checked.deinit());
}

const CacheError = enum { capacity };
const CacheEnv = struct { allocator: std.mem.Allocator, io: std.Io, async_engine: fx.Executor, gate: ?*DependencyGate = null };
const Dep = fx.DependencyCache(u32, u32, u32, CacheError, CacheEnv);
const Keys = struct {
    fn equal(_: ?*anyopaque, a: u32, b: u32) bool {
        return a == b;
    }
    fn retain(_: ?*anyopaque, key: u32) error{OutOfMemory}!u32 {
        return key;
    }
    fn release(_: ?*anyopaque, _: *u32) void {}
    fn failure(_: CacheError) bool {
        return false;
    }
    fn capacity() CacheError {
        return .capacity;
    }
    const policy: fx.DependencyPolicy(u32) = .{ .userdata = null, .equal = equal, .retain = retain, .release = release };
    fn config(_: *const CacheEnv) Dep.Config {
        return .{ .retention = .{ .keys = policy, .time_to_live = null, .capacity = 2, .maximum_inflight = 2, .cache_failure = failure }, .maximum_inflight = 2, .maximum_dependencies_per_entry = 2, .dependency_policy = policy, .capacity_error = capacity };
    }
    fn load(_: u32) fx.Effect(Dep.Loaded, CacheError, CacheEnv) {
        return fx.Effect(Dep.Loaded, CacheError, CacheEnv).fromOutcome(struct {
            fn run(env: *const CacheEnv) fx.Exit(Dep.Loaded, CacheError) {
                return .{ .success = Dep.Loaded.init(env, 42, &.{7}, policy) catch return .{ .die = fx.Defect.out_of_memory } };
            }
        }.run);
    }
};
test "dependency cache selects observations and set copies dependency metadata" {
    const Threads = @import("support/thread_executor.zig").ThreadExecutor;
    var engine: Threads = .{ .allocator = std.testing.allocator, .capacity = 8, .capabilities = .{ .independent = true, .cpu = false, .blocking = false } };
    defer engine.deinit();
    const env: CacheEnv = .{ .allocator = std.testing.allocator, .io = std.testing.io, .async_engine = engine.executor() };
    var scope = fx.Scope(CacheError).init(&env);
    defer scope.deinit();
    defer {
        var closed = fx.run(&env, fx.Effect(void, CacheError, CacheEnv).closeScope(&scope));
        closed.deinit();
    }
    var make = Dep.makeDependent(&scope, Keys.config, Keys.load);
    defer make.deinit();
    var made = fx.run(&env, make);
    defer made.deinit();
    try std.testing.expect(made == .success);
    const cache = made.success;
    var peek = Dep.tryGet(cache, 1);
    defer peek.deinit();
    var absent = fx.run(&env, peek);
    defer absent.deinit();
    try std.testing.expect(absent == .success and absent.success == null);
    var request = Dep.getObserved(cache, 1);
    defer request.deinit();
    var loaded = fx.run(&env, request);
    defer loaded.deinit();
    try std.testing.expect(loaded == .success and loaded.success.observation == .loaded);
    var next = Dep.getObserved(cache, 1);
    defer next.deinit();
    var retained = fx.run(&env, next);
    defer retained.deinit();
    try std.testing.expect(retained == .success and retained.success.observation == .retained);
    var dependency: [1]u32 = .{8};
    const replacement: u32 = 99;
    var set = Dep.set(cache, 1, &replacement, &dependency, Keys.config);
    defer set.deinit();
    var replaced = fx.run(&env, set);
    defer replaced.deinit();
    try std.testing.expect(replaced == .success);
    dependency[0] = 9;
    var invalidate = Dep.invalidateDependency(cache, 8);
    defer invalidate.deinit();
    var invalidated = fx.run(&env, invalidate);
    defer invalidated.deinit();
    try std.testing.expect(invalidated == .success);
    var after = Dep.tryGet(cache, 1);
    defer after.deinit();
    var missing = fx.run(&env, after);
    defer missing.deinit();
    try std.testing.expect(missing == .success and missing.success == null);
}

const NativeBytes = struct {
    pub const Representation = [3]u8;
    pub const Builder = struct { bytes: [3]u8 = undefined, next: usize = 0 };
    pub fn count(_: *const Representation) usize {
        return 3;
    }
    pub fn at(input: *const Representation, index: usize) *const u8 {
        return &input[index];
    }
    pub fn init(_: *const Env, count_: usize) fx.Exit(Builder, fx.SchemaIssue.Issue) {
        return if (count_ == 3) .{ .success = .{} } else .{ .fail = fx.SchemaIssue.Issue.pure(.encode, .size_limit) };
    }
    pub fn append(_: *const Env, builder: *Builder, value: *const u8) fx.Exit(void, fx.SchemaIssue.Issue) {
        builder.bytes[builder.next] = value.*;
        builder.next += 1;
        return .{ .success = {} };
    }
    pub fn finish(_: *const Env, builder: *Builder) fx.Exit(Representation, fx.SchemaIssue.Issue) {
        return .{ .success = builder.bytes };
    }
    pub fn discard(_: *Builder) void {}
};
test "native sequences compose byte representations and record defaults decode once" {
    const Byte = comptime fx.PureCodec(u8, u8).init(struct {
        fn copy(input: *const u8) fx.Result(u8, fx.SchemaIssue.Issue) {
            return .{ .ok = input.* };
        }
    }.copy, struct {
        fn copy(input: *const u8) fx.Result(u8, fx.SchemaIssue.Issue) {
            return .{ .ok = input.* };
        }
    }.copy);
    const Sequence = fx.NativeCodec.sequence(Byte, NativeBytes, Env, Env, 3);
    const env = environment();
    const input: [3]u8 = .{ 1, 2, 3 };
    var decode = Sequence.decode(&input);
    defer decode.deinit();
    var decoded = fx.run(&env, decode);
    defer decoded.deinit();
    try std.testing.expect(decoded == .success);
    var encode = Sequence.encode(&decoded.success);
    defer encode.deinit();
    var encoded = fx.run(&env, encode);
    defer encoded.deinit();
    try std.testing.expect(encoded == .success);
    try std.testing.expectEqualSlices(u8, &input, &encoded.success);
    const Application = struct { answer: i64 };
    const Record = fx.NativeCodec.Record(Application, NativeObject, Env, Env, .{fx.NativeField.constantDefault("answer", fx.JsonCodec.signed_integer, .{ .integer = 42 })}, true);
    const empty: std.json.Value = .{ .object = .{} };
    var default = Record.decode(&empty);
    defer default.deinit();
    var defaulted = fx.run(&env, default);
    defer defaulted.deinit();
    try std.testing.expect(defaulted == .success and defaulted.success.answer == 42);
}

test "allOf checks every branch and exported object accepts its declared property" {
    const env = environment();
    const input: std.json.Value = .{ .integer = 42 };
    const All = fx.JsonConstraints.withLimits(fx.JsonAlternatives.allOf(Env, .{ fx.JsonCodec.signed_integer, fx.JsonCodec.signed_integer }, .{ .maximum_issues = 4, .limits = limits }), select);
    var decoded = All.decodeDirect(&env, &input);
    defer decoded.deinit();
    try std.testing.expect(decoded == .success);
    var encoded = All.encodeBytesDirect(&env, &decoded.success);
    defer encoded.deinit();
    try std.testing.expectEqualStrings("42", encoded.success.items);
    const O = fx.JsonConstraints.Object(Env, .{.{ .name = "answer", .codec = fx.SchemaRepresentation.signed_integer, .required = true }}, null, .{}, null, .{ .minimum_properties = 1, .maximum_properties = 2 }, null);
    var exported = fx.JsonSchema.exportSchema(&env, &.{.{ .name = "object", .root = O.encodedNode() }}, "object", .{ .max_depth = 16, .max_output_bytes = 4096 }, .draft_2020_12);
    defer exported.deinit();
    try std.testing.expect(exported == .success);
    var document = try std.json.parseFromSlice(std.json.Value, env.allocator, exported.success.items, .{});
    defer document.deinit();
    const definition = document.value.object.get("$defs").?.object.get("object").?;
    const additional = definition.object.get("allOf").?.array.items[1];
    try std.testing.expect(additional.object.get("properties").?.object.contains("answer"));
    try std.testing.expect(!additional.object.get("additionalProperties").?.bool);
}

test "pattern capability returns owned timeout details in encode direction" {
    const Pattern = fx.JsonConstraints.PatternCapability(Env);
    const Matcher = struct {
        fn select(_: *const Env) Pattern {
            return .{ .match = run };
        }
        fn run(_: *const Env, _: fx.JsonConstraints.Pattern, _: []const u8) fx.Exit(bool, fx.JsonConstraints.PatternError) {
            return .{ .fail = .timeout };
        }
    };
    const S = fx.JsonConstraints.String(Env, .{ .minimum_scalars = 1, .maximum_scalars = 32, .format = .none, .pattern = .{ .dialect = "test", .source = "bounded", .timeout = .fromMilliseconds(1), .maximum_steps = 8 } }, Matcher.select);
    const env = environment();
    const text: []const u8 = "test";
    var result = S.validateDirect(&env, &text);
    defer result.deinit();
    try std.testing.expect(result == .fail and result.fail.kind == .pattern_timeout and result.fail.direction == .encode);
    try std.testing.expectEqualStrings("bounded", result.fail.detail.?.pattern.source.items);
}

const DependencyGate = struct {
    entered: std.Io.Event = .unset,
    proceed: std.Io.Event = .unset,
    large: bool = false,
    calls: std.atomic.Value(usize) = .init(0),
    fn config(env: *const CacheEnv) Dep.Config {
        var c = Keys.config(env);
        c.maximum_inflight = 1;
        c.maximum_dependencies_per_entry = 1;
        return c;
    }
    fn load(_: u32) fx.Effect(Dep.Loaded, CacheError, CacheEnv) {
        return fx.Effect(Dep.Loaded, CacheError, CacheEnv).fromOutcome(struct {
            fn run(env: *const CacheEnv) fx.Exit(Dep.Loaded, CacheError) {
                const g = env.gate.?;
                _ = g.calls.fetchAdd(1, .acq_rel);
                g.entered.set(env.io);
                g.proceed.waitUncancelable(env.io);
                return .{ .success = Dep.Loaded.init(env, 42, if (g.large) &.{ 7, 8 } else &.{7}, Keys.policy) catch return .{ .die = fx.Defect.out_of_memory } };
            }
        }.run);
    }
};
const DepObserver = struct {
    cache: *Dep.Core,
    result: fx.Exit(fx.CacheLookup(Dep.Loaded), CacheError) = undefined,
    fn run(self: *@This()) void {
        self.result = self.cache.getObservedToken(1, null);
    }
};
const DepMutation = struct {
    const Operation = enum { replacement, key, dependency, clear };
    cache: *Dep.Core,
    env: *const CacheEnv,
    operation: Operation,
    result: fx.Exit(void, CacheError) = undefined,
    finished: std.atomic.Value(bool) = .init(false),
    fn run(self: *@This()) void {
        self.result = switch (self.operation) {
            .key => self.cache.invalidate(1),
            .dependency => self.cache.invalidateDependency(999),
            .clear => self.cache.clear(),
            .replacement => blk: {
                var value = Dep.Loaded.init(self.env, 99, &.{8}, Keys.policy) catch break :blk .{ .die = fx.Defect.out_of_memory };
                defer value.deinit();
                break :blk self.cache.set(1, &value);
            },
        };
        self.finished.store(true, .release);
    }
};
fn awaitDependencyState(cache: *Dep.Core, comptime excluded: bool, observers: usize) !void {
    const deadline = std.Io.Timestamp.now(std.testing.io, .awake).addDuration(.fromSeconds(5));
    while (true) {
        cache.mutex.lockUncancelable(cache.io);
        var ready = false;
        if (excluded) {
            var entry = cache.entries;
            while (entry) |current| {
                if (!current.settled and current.invalidated) {
                    ready = true;
                    break;
                }
                entry = current.next;
            }
        } else if (cache.entries) |entry| {
            entry.completion.mutex.lockUncancelable(cache.io);
            var waiter = entry.completion.waiters;
            var count: usize = 0;
            while (waiter) |current| {
                count += 1;
                waiter = current.next;
            }
            ready = count == observers;
            entry.completion.mutex.unlock(cache.io);
        }
        cache.mutex.unlock(cache.io);
        if (ready) return;
        if (std.Io.Timestamp.now(std.testing.io, .awake).nanoseconds >= deadline.nanoseconds) return error.DependencyTransitionTimeout;
        // Bounded scheduler yield waits for actual registration/exclusion; no timed sleep or stress repetitions.
        std.Thread.yield() catch {};
    }
}
test "dependency cache gates shared observation bounds and every active invalidation" {
    const Threads = @import("support/thread_executor.zig").ThreadExecutor;
    var engine: Threads = .{ .allocator = std.testing.allocator, .capacity = 8, .capabilities = .{ .independent = true, .cpu = false, .blocking = false } };
    defer engine.deinit();
    var gate: DependencyGate = .{};
    const env: CacheEnv = .{ .allocator = std.testing.allocator, .io = std.testing.io, .async_engine = engine.executor(), .gate = &gate };
    var scope = fx.Scope(CacheError).init(&env);
    defer scope.deinit();
    defer {
        gate.proceed.set(env.io);
        var closed = fx.run(&env, fx.Effect(void, CacheError, CacheEnv).closeScope(&scope));
        closed.deinit();
    }
    var make = Dep.makeDependent(&scope, DependencyGate.config, DependencyGate.load);
    defer make.deinit();
    var made = fx.run(&env, make);
    defer made.deinit();
    try std.testing.expect(made == .success);
    const cache = made.success;
    {
        var first: DepObserver = .{ .cache = cache };
        var second: DepObserver = .{ .cache = cache };
        const a = try std.Thread.spawn(.{}, DepObserver.run, .{&first});
        var a_joined = false;
        defer first.result.deinit();
        defer if (!a_joined) a.join();
        defer gate.proceed.set(env.io);
        try gate.entered.waitTimeout(env.io, .{ .duration = .{ .raw = .fromSeconds(5), .clock = .awake } });
        const b = try std.Thread.spawn(.{}, DepObserver.run, .{&second});
        var b_joined = false;
        defer second.result.deinit();
        defer if (!b_joined) b.join();
        defer gate.proceed.set(env.io);
        try awaitDependencyState(cache, false, 2);
        var rejected = cache.getObservedToken(2, null);
        defer rejected.deinit();
        try std.testing.expect(rejected == .fail and rejected.fail == .capacity);
        try std.testing.expectEqual(@as(usize, 1), gate.calls.load(.acquire));
        gate.proceed.set(env.io);
        b.join();
        b_joined = true;
        a.join();
        a_joined = true;
        try std.testing.expect(first.result == .success and first.result.success.observation == .loaded);
        try std.testing.expect(second.result == .success and second.result.success.observation == .shared);
    }
    var cleared = cache.clear();
    cleared.deinit();
    gate.large = true;
    var bounded = cache.getToken(1, null);
    defer bounded.deinit();
    try std.testing.expect(bounded == .fail and bounded.fail == .capacity);
    const value: u32 = 99;
    var invalid_set = Dep.set(cache, 1, &value, &.{ 7, 8 }, DependencyGate.config);
    defer invalid_set.deinit();
    var set_bound = fx.run(&env, invalid_set);
    defer set_bound.deinit();
    try std.testing.expect(set_bound == .fail and set_bound.fail == .capacity);
    cleared = cache.clear();
    cleared.deinit();
    gate.large = false;
    for ([_]DepMutation.Operation{ .replacement, .key, .dependency, .clear }) |operation| {
        gate.entered.reset();
        gate.proceed.reset();
        var observer: DepObserver = .{ .cache = cache };
        const loading = try std.Thread.spawn(.{}, DepObserver.run, .{&observer});
        var load_joined = false;
        defer observer.result.deinit();
        defer if (!load_joined) loading.join();
        defer gate.proceed.set(env.io);
        try gate.entered.waitTimeout(env.io, .{ .duration = .{ .raw = .fromSeconds(5), .clock = .awake } });
        var mutation: DepMutation = .{ .cache = cache, .env = &env, .operation = operation };
        const mutating = try std.Thread.spawn(.{}, DepMutation.run, .{&mutation});
        var mutation_joined = false;
        defer mutation.result.deinit();
        defer if (!mutation_joined) mutating.join();
        defer gate.proceed.set(env.io);
        try awaitDependencyState(cache, true, 0);
        try std.testing.expect(!mutation.finished.load(.acquire));
        var before = cache.tryGet(1);
        defer before.deinit();
        try std.testing.expect(before == .success);
        if (operation == .replacement) try std.testing.expect(before.success != null and before.success.?.value == 99) else try std.testing.expect(before.success == null);
        gate.proceed.set(env.io);
        loading.join();
        load_joined = true;
        mutating.join();
        mutation_joined = true;
        try std.testing.expect(mutation.result == .success);
        var after = cache.tryGet(1);
        defer after.deinit();
        try std.testing.expect(after == .success);
        if (operation == .replacement) try std.testing.expect(after.success != null and after.success.?.value == 99) else try std.testing.expect(after.success == null);
        cleared = cache.clear();
        cleared.deinit();
    }
}

fn surfaceLimits(_: *const Env) fx.JsonSurface.Limits {
    return .{ .input = .{ .max_document_bytes = 256, .max_depth = 8, .max_value_bytes = 128 }, .schema = .{ .max_nodes = 64, .max_edges = 64, .max_depth = 16 }, .maximum_issues = 4, .maximum_integer_digits = 32, .output = .{ .max_output_bytes = 3, .max_json_depth = 8, .max_value_bytes = 128, .max_integer_digits = 32 } };
}
test "bounded UTF8 parsing separates input output and duplicate policies" {
    const env = environment();
    const S = fx.JsonConstraints.String(Env, .{ .minimum_scalars = 0, .maximum_scalars = 64, .format = .none }, null);
    const B = fx.JsonConstraints.withLimits(S, surfaceLimits);
    var parsed = B.parseBytesDirect(&env, "\"hello\"", .reject);
    defer parsed.deinit();
    try std.testing.expect(parsed == .success);
    try std.testing.expectEqualStrings("hello", parsed.success.value);
    var encoded = B.encodeBytesDirect(&env, &parsed.success.value);
    defer encoded.deinit();
    try std.testing.expect(encoded == .fail and encoded.fail.detail.?.resource.resource == .output_bytes);
    const App = struct { answer: i64 };
    const Record = fx.SchemaRecordBuilder(App, Env).begin().fields(.{fx.SchemaBuilder.required("answer", fx.SchemaRepresentation.signed_integer)}).policy(.{ .unknown_fields = .reject, .duplicate_fields = .reject, .issues = .first }).build();
    const R = fx.JsonConstraints.withLimits(Record, surfaceLimits);
    var duplicated = R.parseBytesDirect(&env, "{\"answer\":1,\"answer\":2}", .reject);
    defer duplicated.deinit();
    try std.testing.expect(duplicated == .fail and duplicated.fail.kind == .duplicate_field);
    var last = R.parseBytesDirect(&env, "{\"answer\":1,\"answer\":2}", .last_wins);
    defer last.deinit();
    try std.testing.expect(last == .success and last.success.value.answer == 2);
    const Number = fx.JsonNumberCodec.exact(Env, .{ .integer = true, .maximum_integer_digits = 32, .maximum_value_bytes = 128 });
    const N = fx.JsonConstraints.withLimits(Number, surfaceLimits);
    var huge = N.parseBytesDirect(&env, "1e1000000", .reject);
    defer huge.deinit();
    try std.testing.expect(huge == .fail and huge.fail.detail.?.integer_digits.limit == 32);
}

test "direct union writer agrees with DOM and counts nested string and number depth" {
    const env = environment();
    const Text = fx.JsonConstraints.String(Env, .{ .minimum_scalars = 1, .maximum_scalars = 8, .format = .none }, null);
    const U = union(enum) { text: []const u8, flag: bool, unit: void };
    const V = fx.VariantSchema.Variant(U, Env, "tag", "payload", .{ fx.VariantSchema.nestedCase("text", "text", Text), fx.VariantSchema.taggedCase("flag", "flag", fx.JsonCodec.boolean), fx.VariantSchema.taggedCase("unit", "unit", fx.JsonCodec.null_value) });
    const value: U = .{ .text = "é" };
    var direct = V.encodeBytesWith(&env, &value, limits);
    defer direct.deinit();
    var arena = std.heap.ArenaAllocator.init(env.allocator);
    defer arena.deinit();
    const dom = V.encodeValueDirect(&env, &value, arena.allocator());
    try std.testing.expect(dom == .success);
    var generic = fx.JsonWriter.Writer.init(&env, limits);
    defer generic.deinit();
    try generic.json(&dom.success);
    try std.testing.expectEqualStrings(generic.output.items, direct.success.items);
    const Budgets = struct {
        fn select(_: *const Env) fx.JsonSurface.Limits {
            var budget = surfaceLimits(undefined);
            budget.output = limits;
            return budget;
        }
        fn unit(_: *const Env) fx.JsonSurface.Limits {
            var budget = @This().select(undefined);
            budget.schema = .{ .max_nodes = 1, .max_edges = 1, .max_depth = 1 };
            return budget;
        }
    };
    const Bounded = fx.JsonConstraints.withLimits(V, Budgets.select);
    var ordinary = Bounded.parseBytesDirect(&env, direct.success.items, .reject);
    defer ordinary.deinit();
    try std.testing.expect(ordinary == .success);
    try std.testing.expectEqualStrings("é", ordinary.success.value.text);
    var bounded_encoded = Bounded.encodeBytesDirect(&env, &ordinary.success.value);
    defer bounded_encoded.deinit();
    try std.testing.expectEqualStrings(direct.success.items, bounded_encoded.success.items);
    var unit_value = Bounded.parseBytesDirect(&env, "{\"tag\":\"unit\",\"payload\":null}", .reject);
    defer unit_value.deinit();
    try std.testing.expect(unit_value == .success and unit_value.success.value == .unit);
    var unit_encoded = Bounded.encodeBytesDirect(&env, &unit_value.success.value);
    defer unit_encoded.deinit();
    try std.testing.expectEqualStrings("{\"tag\":\"unit\",\"payload\":null}", unit_encoded.success.items);
    // An AST unit case has no payload node or edge. The typed JSON schema below
    // validates that discriminator-only representation without inventing a child.
    const Unit = struct {
        pub const Decoded = std.json.Value;
        pub const Environment = Env;
        const node: fx.SchemaAst.Node = .{ .tagged_union = .{ .discriminator = "tag", .payload_name = "payload", .cases = &.{.{ .tag = "unit", .payload = null }} } };
        pub fn encodedNode() *const fx.SchemaAst.Node {
            return &node;
        }
        pub fn decodeWithin(env_: *const Env, json: *const std.json.Value, _: anytype) fx.Exit(Decoded, fx.SchemaIssue.Issue) {
            if (json.* != .object or json.object.count() != 1) return .{ .fail = fx.SchemaIssue.Issue.init(env_, .decode, .invalid_value) };
            const tag = json.object.get("tag") orelse return .{ .fail = fx.SchemaIssue.Issue.init(env_, .decode, .missing_field) };
            if (tag != .string or !std.mem.eql(u8, tag.string, "unit")) return .{ .fail = fx.SchemaIssue.Issue.init(env_, .decode, .invalid_value) };
            return .{ .success = json.* };
        }
        pub fn encodeWithin(env_: *const Env, json: *const Decoded, state: anytype) fx.Exit(std.json.Value, fx.SchemaIssue.Issue) {
            return decodeWithin(env_, json, state);
        }
    };
    const UnitBounded = fx.JsonConstraints.withLimits(Unit, Budgets.unit);
    var no_payload = UnitBounded.parseBytesDirect(&env, "{\"tag\":\"unit\"}", .reject);
    defer no_payload.deinit();
    try std.testing.expect(no_payload == .success);
    var no_payload_encoded = UnitBounded.encodeBytesDirect(&env, &no_payload.success.value);
    defer no_payload_encoded.deinit();
    try std.testing.expect(no_payload_encoded == .success);
    try std.testing.expectEqualStrings("{\"tag\":\"unit\"}", no_payload_encoded.success.items);

    var shallow = limits;
    shallow.max_json_depth = 1;
    var denied = V.encodeBytesWith(&env, &value, shallow);
    defer denied.deinit();
    try std.testing.expect(denied == .fail and denied.fail.detail.?.resource.resource == .json_depth);
    const N = fx.JsonNumberCodec.exact(Env, .{ .integer = true, .maximum_integer_digits = 32, .maximum_value_bytes = 128 });
    var number = N.decodeDirect(&env, &.{ .integer = 42 });
    defer number.deinit();
    var nested = fx.JsonWriter.Writer.init(&env, shallow);
    defer nested.deinit();
    try nested.enter();
    var denied_number = N.writeJson(&env, &number.success, &nested);
    defer denied_number.deinit();
    try std.testing.expect(denied_number == .fail and denied_number.fail.detail.?.resource.actual == 2);
    nested.leave();
}

test "nested prefix object issues survive released JSON source and owned wire roundtrip" {
    const env = environment();
    const Text = fx.JsonConstraints.String(Env, .{ .minimum_scalars = 1, .maximum_scalars = 1, .format = .none }, null);
    const P = fx.JsonConstraints.Prefix(Env, .{.{ .codec = Text, .required = true }}, null, .{ .minimum = 1, .maximum = 1, .unique = false });
    const O = fx.JsonConstraints.withLimits(fx.JsonConstraints.Object(Env, .{.{ .name = "items", .codec = P, .required = true }}, null, .{}, null, .{ .minimum_properties = 1, .maximum_properties = 1 }, null), select);
    var document = try std.json.parseFromSlice(std.json.Value, env.allocator, "{\"items\":[\"ab\"]}", .{ .allocate = .alloc_always });
    var rejected = O.decodeDirect(&env, &document.value);
    document.deinit();
    defer rejected.deinit();
    try std.testing.expect(rejected == .fail);
    var copied = try rejected.fail.clone();
    defer copied.deinit();
    try std.testing.expectEqualStrings("items", copied.path.?[0].field.items);
    try std.testing.expectEqual(@as(usize, 0), copied.path.?[1].index);
    try std.testing.expectEqualStrings("ab", copied.detail.?.constraint.actual.items);
    const il: fx.SchemaIssueCodec.Limits = .{ .input_bytes = 4096, .issue_depth = 8, .issue_count = 8, .json = limits };
    var wire = fx.SchemaIssueCodec.encodeDirect(&env, &copied, il);
    defer wire.deinit();
    var restored = fx.SchemaIssueCodec.decodeDirect(&env, wire.success.items, il);
    defer restored.deinit();
    try std.testing.expect(restored == .success);
    try std.testing.expectEqualStrings("ab", restored.success.detail.?.constraint.actual.items);
}

test "UUID v4 validation and bounded arbitrary width integer conversion" {
    const env = environment();
    const S = fx.JsonConstraints.String(Env, .{ .minimum_scalars = 36, .maximum_scalars = 36, .format = .uuid_v4 }, null);
    const good: []const u8 = "12345678-1234-4abc-8def-123456789abc";
    const wrong_version: []const u8 = "12345678-1234-1abc-8def-123456789abc";
    const wrong_variant: []const u8 = "12345678-1234-4abc-7def-123456789abc";
    var accepted = S.validateDirect(&env, &good);
    defer accepted.deinit();
    try std.testing.expect(accepted == .success);
    for ([_][]const u8{ wrong_version, wrong_variant }) |text| {
        var denied = S.validateDirect(&env, &text);
        defer denied.deinit();
        try std.testing.expect(denied == .fail and denied.fail.detail != null);
    }
    const N = fx.JsonNumberCodec.Number;
    const wide = try N.parse("340282366920938463463374607431768211456");
    try std.testing.expectEqual(@as(i512, 1) << 128, try wide.toInteger(i512, 39));
    try std.testing.expectError(error.IntegerDigits, wide.toInteger(i512, 38));
    var exact = fx.JsonSchema.exportSchema(&env, &.{.{ .name = "v4", .root = S.encodedNode() }}, "v4", .{ .max_depth = 16, .max_output_bytes = 4096 }, .draft_2020_12);
    defer exact.deinit();
    try std.testing.expect(exact == .fail);
    var conservative = fx.JsonSchema.exportSchema(&env, &.{.{ .name = "v4", .root = S.encodedNode(), .accuracy = .conservative }}, "v4", .{ .max_depth = 16, .max_output_bytes = 4096 }, .draft_2020_12);
    defer conservative.deinit();
    try std.testing.expect(conservative == .success);
}

const DirectionEnv = struct { allocator: std.mem.Allocator, io: std.Io, offset: u8 };
const DomainError = enum { invalid };
const Mixed = fx.DomainCodec.Directed(u8, u8, fx.SchemaIssue.Issue, void, DirectionEnv, true, false).init(struct {
    fn decode(v: *const u8) fx.Result(u8, fx.SchemaIssue.Issue) {
        return .{ .ok = v.* };
    }
}.decode, struct {
    fn encode(v: *const u8) fx.Effect(u8, fx.SchemaIssue.Issue, DirectionEnv) {
        return fx.Effect(u8, fx.SchemaIssue.Issue, DirectionEnv).fromBorrowed(u8, v, struct {
            fn run(value: *const u8, env: *const DirectionEnv) fx.Exit(u8, fx.SchemaIssue.Issue) {
                return .{ .success = value.* + env.offset };
            }
        }.run);
    }
}.encode);
fn domainIssue(issue: fx.SchemaIssue.Issue) DomainError {
    var owned = issue;
    owned.deinit();
    return .invalid;
}
test "domain and native options retain independent decoding encoding evidence" {
    const mapped = comptime fx.DomainCodec.ofSchema(DomainError, Mixed, domainIssue, domainIssue);
    const optional = comptime fx.DomainCodec.optional(mapped);
    comptime {
        if (!@TypeOf(optional).pure_decoding or @TypeOf(optional).pure_encoding) @compileError("direction evidence lost");
    }
    const input: @TypeOf(optional).Representation = .{ .some = 4 };
    var decoded = optional.decodeDirect(&input);
    defer decoded.deinit();
    try std.testing.expect(decoded == .ok and decoded.ok.? == 4);
    const env: DirectionEnv = .{ .allocator = std.testing.allocator, .io = std.testing.io, .offset = 3 };
    var encoding = optional.encode(&decoded.ok);
    defer encoding.deinit();
    var encoded = fx.run(&env, encoding);
    defer encoded.deinit();
    try std.testing.expect(encoded == .success and encoded.success.some == 7);
    const native = comptime fx.NativeCodec.option(Mixed);
    var native_value = native.decodeDirect(&input);
    defer native_value.deinit();
    try std.testing.expect(native_value == .ok);
    const Reverse = comptime fx.DomainCodec.Directed(u8, u8, DomainError, DirectionEnv, void, false, true).init(struct {
        fn decode(v: *const u8) fx.Effect(u8, DomainError, DirectionEnv) {
            return fx.Effect(u8, DomainError, DirectionEnv).fromBorrowed(u8, v, struct {
                fn run(value: *const u8, supplied: *const DirectionEnv) fx.Exit(u8, DomainError) {
                    return .{ .success = value.* + supplied.offset };
                }
            }.run);
        }
    }.decode, struct {
        fn encode(v: *const u8) fx.Result(u8, DomainError) {
            return .{ .ok = v.* };
        }
    }.encode);
    const reverse = comptime fx.DomainCodec.optional(Reverse);
    const value: ?u8 = 4;
    var direct = reverse.validateDirect(&value);
    defer direct.deinit();
    try std.testing.expect(direct == .ok);
}

test "workflow reset compares terminal reply revision atomically and preserves replay" {
    const env = environment();
    const Time = struct {
        fn now(_: *const Env) fx.WallInstant {
            return .{ .unix_nanoseconds = 0 };
        }
    };
    var memory = fx.MemoryWorkflowEngine(Env, Time.now).init(&env);
    defer memory.deinit();
    const engine = memory.capability();
    var identity = fx.PrimaryKey.encodeDirect(&env, "checkpoint-test", &.{.{ .text = "run" }});
    defer identity.deinit();
    try std.testing.expect(identity == .success);
    const execution: fx.WorkflowTypes.ExecutionId = .{ .encoded = identity.success.items };
    const definition: fx.WorkflowTypes.Definition = .{ .name = "checkpoint-test", .version = 1, .payload_schema_version = 1, .success_schema_version = 1, .error_schema_version = 1 };
    var submit = engine.vtable.submit(engine.userdata, &env, .{ .execution = execution, .definition = definition, .encoded_payload = "payload" });
    defer submit.deinit();
    var submitted = fx.run(&env, submit);
    defer submitted.deinit();
    try std.testing.expect(submitted == .success);
    var activate = engine.vtable.activate(engine.userdata, &env, execution, "owner", .{ .lease_until = .{ .at = .{ .unix_nanoseconds = 1000000 } } });
    defer activate.deinit();
    var active = fx.run(&env, activate);
    defer active.deinit();
    try std.testing.expect(active == .success and active.success == .activated);
    const activation = active.success.activated.activation();
    var complete = engine.vtable.complete(engine.userdata, &env, activation, "typed-failure-checkpoint");
    defer complete.deinit();
    var completed = fx.run(&env, complete);
    defer completed.deinit();
    try std.testing.expect(completed == .success and completed.success == .committed);
    var poll = engine.vtable.poll(engine.userdata, &env, execution);
    defer poll.deinit();
    var observed = fx.run(&env, poll);
    defer observed.deinit();
    try std.testing.expect(observed == .success and observed.success == .completed);
    const checkpoint = observed.success.completed;
    var reset = engine.vtable.resume_execution(engine.userdata, &env, .{ .execution = execution, .expected_revision = checkpoint.revision, .expected_terminal_reply = checkpoint.items });
    defer reset.deinit();
    var reset_result = fx.run(&env, reset);
    defer reset_result.deinit();
    try std.testing.expect(reset_result == .success and reset_result.success == .recorded);
    var stale = engine.vtable.resume_execution(engine.userdata, &env, .{ .execution = execution, .expected_revision = checkpoint.revision, .expected_terminal_reply = checkpoint.items });
    defer stale.deinit();
    var stale_result = fx.run(&env, stale);
    defer stale_result.deinit();
    try std.testing.expect(stale_result == .success and stale_result.success == .stale_revision);
    var second = engine.vtable.activate(engine.userdata, &env, execution, "new-owner", .{ .lease_until = .{ .at = .{ .unix_nanoseconds = 1000000 } } });
    defer second.deinit();
    var second_result = fx.run(&env, second);
    defer second_result.deinit();
    try std.testing.expect(second_result == .success and second_result.success == .activated);
    var late = engine.vtable.complete(engine.userdata, &env, activation, "old-completion");
    defer late.deinit();
    var late_result = fx.run(&env, late);
    defer late_result.deinit();
    try std.testing.expect(late_result == .success and late_result.success == .stale_activation);
}

fn streamEnvelope(env: *const Env, correlation: u64, kind: fx.RpcEnvelope.Kind, sequence: u64, payload: []const u8) !fx.RpcEnvelope.Envelope {
    const operation = try env.allocator.dupe(u8, "stream");
    errdefer env.allocator.free(operation);
    const bytes = try env.allocator.dupe(u8, payload);
    errdefer env.allocator.free(bytes);
    return .{ .version = 1, .kind = kind, .correlation = correlation, .sequence = sequence, .operation = .{ .items = operation, .allocator = env.allocator }, .metadata = try env.allocator.alloc(fx.RpcEnvelope.Metadata, 0), .payload = .{ .items = bytes, .allocator = env.allocator }, .allocator = env.allocator };
}
test "RPC normal EOF drains accepted replies with per correlation decode isolation" {
    const env = environment();
    var registry = fx.RpcStreamRegistry(Env).init(&env, 2, 3).success;
    defer registry.deinit();
    const bad = registry.register(1, "stream", 1).success;
    defer registry.unregister(bad);
    const good = registry.register(2, "stream", 1).success;
    defer registry.unregister(good);
    var first = registry.deliver(try streamEnvelope(&env, 1, .stream_chunk, 0, "\"bad\""));
    defer first.deinit();
    var second = registry.deliver(try streamEnvelope(&env, 2, .stream_chunk, 0, "42"));
    defer second.deinit();
    var last = registry.deliver(try streamEnvelope(&env, 2, .stream_end, 1, "null"));
    defer last.deinit();
    registry.stopIntake();
    var denied = registry.register(3, "stream", 1);
    defer denied.deinit();
    try std.testing.expect(denied == .fail and denied.fail == .closed);
    var taking_bad = bad.take();
    defer taking_bad.deinit();
    var bad_frame = fx.run(&env, taking_bad);
    defer bad_frame.deinit();
    var bad_json = try std.json.parseFromSlice(std.json.Value, env.allocator, bad_frame.success.payload.items, .{});
    defer bad_json.deinit();
    var bad_decode = fx.JsonCodec.signed_integer.decodeDirect(&bad_json.value);
    defer bad_decode.deinit();
    try std.testing.expect(bad_decode == .err);
    var taking_good = good.take();
    defer taking_good.deinit();
    var good_frame = fx.run(&env, taking_good);
    defer good_frame.deinit();
    try std.testing.expect(good_frame == .success);
    try std.testing.expectEqualStrings("42", good_frame.success.payload.items);
    var terminal = good.take();
    defer terminal.deinit();
    var drained = fx.run(&env, terminal);
    defer drained.deinit();
    try std.testing.expect(drained == .success and drained.success.kind == .stream_end);
}

test "replica authorization rejection occurs before bind construction" {
    const Authority = fx.EventReplicaRemote.SessionAuthority(Env, void);
    const Adapter = struct {
        var calls: usize = 0;
        const authority = Authority.fromProvider(null, validate);
        fn validate(_: ?*anyopaque, _: *const Env, _: fx.EventReplicaMessage.Identity) Authority.ValidateEffect {
            return Authority.ValidateEffect.succeed(false);
        }
        fn selectAuthority(_: *const Env) *const Authority {
            return &authority;
        }
        fn bind(_: fx.EventReplicaMessage.Identity, _: *const Env) fx.Effect(u8, fx.EventReplicaRemote.Error(void), Env) {
            calls += 1;
            return fx.Effect(u8, fx.EventReplicaRemote.Error(void), Env).succeed(1);
        }
    };
    Adapter.calls = 0;
    const env = environment();
    var key = fx.PrimaryKey.encodeDirect(&env, "replica", &.{.{ .text = "source" }});
    defer key.deinit();
    const identity: fx.EventReplicaMessage.Identity = .{ .source = key.success.items, .destination = key.success.items, .session = "unencrypted" };
    var program = fx.EventReplicaRemote.bindAuthorized(u8, Env, void, &identity, Adapter.selectAuthority, Adapter.bind);
    defer program.deinit();
    var result = fx.run(&env, program);
    defer result.deinit();
    try std.testing.expect(result == .fail and result.fail == .rejected_session);
    try std.testing.expectEqual(@as(usize, 0), Adapter.calls);
}

test "queue payload cleanup can reenter shutdown without lock retention" {
    const Owner = struct {
        pub const effect_owned_payload = true;
        callback: *const fn (*anyopaque) void,
        data: *anyopaque,
        released: *usize,
        pub fn deinit(self: *@This()) void {
            self.released.* += 1;
            self.callback(self.data);
        }
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            return self.*;
        }
    };
    const Q = fx.Queue(Owner);
    const Reentry = struct {
        fn close(data: *anyopaque) void {
            const q: *Q = @ptrCast(@alignCast(data));
            _ = q.size();
            q.shutdown(false);
        }
    };
    const env = environment();
    var queue = Q.init(&env, 2).success;
    defer queue.deinit();
    var released: usize = 0;
    var one = queue.offerToken(.{ .callback = Reentry.close, .data = &queue, .released = &released }, null);
    defer one.deinit();
    var two = queue.offerToken(.{ .callback = Reentry.close, .data = &queue, .released = &released }, null);
    defer two.deinit();
    queue.shutdown(false);
    try std.testing.expectEqual(@as(usize, 2), released);
}

const RpcFixtureEnv = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    session: *const fx.RpcSession(@This(), void),
    registry: *fx.RpcStreamRegistry(@This()),
    handle: ?*anyopaque = null,
    request: i64 = 1,
    cancelled: *[3]usize,
    acknowledged: *[3]usize,
};
const RpcFixture = struct {
    const R = RpcFixtureEnv;
    const Session = fx.RpcSession(R, void);
    const Int = fx.RpcWireCodec(i64, R);
    const Void = fx.RpcWireCodec(void, R);
    const integer = Int.init(decodeInt, encodeInt);
    const unit = Void.init(decodeVoid, encodeVoid);
    const operation = fx.RpcProtocolBuilder(void, R).streaming("stream", 1).request(fx.JsonCodec.signed_integer).chunk(fx.JsonCodec.signed_integer).terminal(fx.JsonCodec.null_value).failure(fx.JsonCodec.null_value).definition;
    const Client = fx.RpcStreamClient(operation, R, void, void, integer, integer, unit, unit, session, registry, policy, correlation, metadata);
    fn decodeInt(bytes: *const fx.OwnedSlice(u8)) fx.Effect(i64, fx.SchemaIssue.Issue, R) {
        return fx.Effect(i64, fx.SchemaIssue.Issue, R).fromBorrowed(fx.OwnedSlice(u8), bytes, struct {
            fn run(input: *const fx.OwnedSlice(u8), env: *const R) fx.Exit(i64, fx.SchemaIssue.Issue) {
                return .{ .success = std.fmt.parseInt(i64, input.items, 10) catch return .{ .fail = fx.SchemaIssue.Issue.init(env, .decode, .invalid_number) } };
            }
        }.run);
    }
    fn encodeInt(value: *const i64) fx.Effect(fx.OwnedSlice(u8), fx.SchemaIssue.Issue, R) {
        return fx.Effect(fx.OwnedSlice(u8), fx.SchemaIssue.Issue, R).fromBorrowed(i64, value, struct {
            fn run(input: *const i64, env: *const R) fx.Exit(fx.OwnedSlice(u8), fx.SchemaIssue.Issue) {
                return .{ .success = .{ .items = env.allocator.print("{d}", .{input.*}) catch return .{ .die = fx.Defect.out_of_memory }, .allocator = env.allocator } };
            }
        }.run);
    }
    fn decodeVoid(bytes: *const fx.OwnedSlice(u8)) fx.Effect(void, fx.SchemaIssue.Issue, R) {
        return fx.Effect(void, fx.SchemaIssue.Issue, R).fromBorrowed(fx.OwnedSlice(u8), bytes, struct {
            fn run(input: *const fx.OwnedSlice(u8), env: *const R) fx.Exit(void, fx.SchemaIssue.Issue) {
                return if (std.mem.eql(u8, input.items, "null")) .{ .success = {} } else .{ .fail = fx.SchemaIssue.Issue.init(env, .decode, .invalid_value) };
            }
        }.run);
    }
    fn encodeVoid(_: *const void) fx.Effect(fx.OwnedSlice(u8), fx.SchemaIssue.Issue, R) {
        return fx.Effect(fx.OwnedSlice(u8), fx.SchemaIssue.Issue, R).fromOutcome(struct {
            fn run(env: *const R) fx.Exit(fx.OwnedSlice(u8), fx.SchemaIssue.Issue) {
                return .{ .success = .{ .items = env.allocator.dupe(u8, "null") catch return .{ .die = fx.Defect.out_of_memory }, .allocator = env.allocator } };
            }
        }.run);
    }
    fn session(env: *const R) *const Session {
        return env.session;
    }
    fn registry(env: *const R) *fx.RpcStreamRegistry(R) {
        return env.registry;
    }
    fn policy(_: *const R) fx.RpcSessionPolicy {
        return .{ .max_inflight = 2, .max_pending_frames = 3, .frame_limits = .{ .max_frame_bytes = 2048, .max_buffered_bytes = 2052 }, .envelope_limits = .{ .max_envelope_bytes = 2048, .max_operation_bytes = 64, .max_payload_bytes = 1024, .max_metadata_entries = 4, .max_metadata_bytes = 128 } };
    }
    fn correlation(_: *const R, request_: *const i64) u128 {
        return @intCast(request_.*);
    }
    fn metadata(_: *const R, _: *const i64) []const fx.RpcEnvelope.View.Entry {
        return &.{};
    }
    fn request(env: *const R) *const i64 {
        return &env.request;
    }
    fn handle(env: *const R) *Client.Handle {
        return @ptrCast(@alignCast(env.handle.?));
    }
    fn send(_: ?*anyopaque, _: *const R, _: []const u8) Session.SendEffect {
        return Session.SendEffect.succeed(.accepted);
    }
    fn receive(_: ?*anyopaque, _: *const R) Session.ReceiveEffect {
        return Session.ReceiveEffect.fromOutcome(struct {
            fn closed(_: *const R) fx.Exit(Session.ReceiveEffect.Success, Session.Failure) {
                return .{ .success = .closed };
            }
        }.closed);
    }
    fn acknowledge(_: ?*anyopaque, env: *const R, id: u128, _: u64) Session.ControlEffect {
        env.acknowledged[@intCast(id)] += 1;
        return Session.ControlEffect.succeed({});
    }
    fn cancel(_: ?*anyopaque, env: *const R, id: u128) Session.ControlEffect {
        env.cancelled[@intCast(id)] += 1;
        return Session.ControlEffect.succeed({});
    }
};
test "public RPC StreamClient failed decode cancels only its correlation and releases frames" {
    const env = environment();
    const session = RpcFixture.Session.fromProvider(null, &.{ .send = RpcFixture.send, .receive = RpcFixture.receive, .acknowledge = RpcFixture.acknowledge, .cancel = RpcFixture.cancel });
    var cancelled: [3]usize = @splat(0);
    var acknowledged: [3]usize = @splat(0);
    var rpc: RpcFixtureEnv = .{ .allocator = env.allocator, .io = env.io, .session = &session, .registry = undefined, .cancelled = &cancelled, .acknowledged = &acknowledged };
    var registry = fx.RpcStreamRegistry(RpcFixtureEnv).init(&rpc, 2, 3).success;
    defer registry.deinit();
    rpc.registry = &registry;
    var open = RpcFixture.Client.open(RpcFixture.request);
    defer open.deinit();
    var first = fx.run(&rpc, open);
    defer first.deinit();
    try std.testing.expect(first == .success);
    rpc.request = 2;
    var second = fx.run(&rpc, open);
    defer second.deinit();
    try std.testing.expect(second == .success);
    var bad = registry.deliver(try streamEnvelope(&env, 1, .stream_chunk, 0, "bad"));
    defer bad.deinit();
    var good = registry.deliver(try streamEnvelope(&env, 2, .stream_chunk, 0, "42"));
    defer good.deinit();
    var end = registry.deliver(try streamEnvelope(&env, 2, .stream_end, 1, "null"));
    defer end.deinit();
    rpc.handle = &first.success;
    var next = RpcFixture.Client.next(RpcFixture.handle);
    defer next.deinit();
    var rejected = fx.run(&rpc, next);
    defer rejected.deinit();
    try std.testing.expect(rejected == .fail);
    try std.testing.expectEqual(@as(usize, 1), cancelled[1]);
    try std.testing.expectEqual(@as(usize, 0), cancelled[2]);
    rpc.handle = &second.success;
    var usable = fx.run(&rpc, next);
    defer usable.deinit();
    try std.testing.expect(usable == .success and usable.success == .chunk and usable.success.chunk == 42);
    var finished = fx.run(&rpc, next);
    defer finished.deinit();
    try std.testing.expect(finished == .success and finished.success == .terminal);
    try std.testing.expectEqual(@as(usize, 1), acknowledged[2]);
    try std.testing.expectEqual(@as(usize, 0), cancelled[2]);
}

test "static default and direct transform callbacks run once and omitted option does not write" {
    const Counts = struct {
        var defaults: usize = 0;
        var backwards: usize = 0;
        fn default(_: *const Env) fx.Exit(i64, fx.SchemaIssue.Issue) {
            defaults += 1;
            return .{ .success = 42 };
        }
        fn forward(v: i64) fx.Result(i64, fx.SchemaIssue.Issue) {
            return .{ .ok = v };
        }
        fn backward(v: *const i64) fx.Result(i64, fx.SchemaIssue.Issue) {
            backwards += 1;
            return .{ .ok = v.* };
        }
    };
    Counts.defaults = 0;
    Counts.backwards = 0;
    const transformed = comptime fx.JsonCodec.transform(fx.JsonCodec.signed_integer, i64, Counts.forward, Counts.backward);
    const App = struct { answer: i64, omitted: ?i64 };
    const Record = fx.SchemaRecordBuilder(App, Env).begin().fields(.{ fx.SchemaBuilder.defaulted("answer", transformed, Counts.default), fx.SchemaBuilder.optional("omitted", transformed) }).policy(.{ .unknown_fields = .reject, .duplicate_fields = .reject, .issues = .first }).build();
    const env = environment();
    const input: std.json.Value = .{ .object = .{} };
    var decoded = Record.decodeDirect(&env, &input);
    defer decoded.deinit();
    try std.testing.expect(decoded == .success);
    try std.testing.expectEqual(@as(usize, 1), Counts.defaults);
    try std.testing.expectEqual(@as(usize, 1), Counts.backwards);
    var output = fx.JsonWriter.Writer.init(&env, limits);
    defer output.deinit();
    var encoded = Record.writeJson(&env, &decoded.success, &output);
    defer encoded.deinit();
    try std.testing.expect(encoded == .success);
    try std.testing.expectEqual(@as(usize, 2), Counts.backwards);
    try std.testing.expectEqualStrings("{\"answer\":42}", output.output.items);
    var represented = Record.encodeBytesDirect(&env, &decoded.success);
    defer represented.deinit();
    try std.testing.expect(represented == .success);
    try std.testing.expectEqual(@as(usize, 3), Counts.backwards);
    try std.testing.expectEqualStrings("{\"answer\":42}", represented.success.items);
    const refined = comptime fx.JsonCodec.refine(transformed, "positive", struct {
        fn positive(v: *const i64) bool {
            return v.* > 0;
        }
    }.positive);
    const root = comptime fx.SchemaRepresentation.nodePointer(refined);
    const mapped = comptime fx.DomainCodec.ofSchema(fx.SchemaIssue.Issue, refined, struct {
        fn identity(v: fx.SchemaIssue.Issue) fx.SchemaIssue.Issue {
            return v;
        }
    }.identity, struct {
        fn identity(v: fx.SchemaIssue.Issue) fx.SchemaIssue.Issue {
            return v;
        }
    }.identity);
    try std.testing.expectEqual(root, fx.SchemaRepresentation.nodePointer(mapped));
    try std.testing.expectEqual(root, fx.SchemaRepresentation.nodePointer(refined.asCodec()));
    try std.testing.expectEqualStrings("positive", root.constrained.rules[0].check);
    try std.testing.expectEqualStrings("transform", root.constrained.base.constrained.rules[0].check);
    var exact = fx.JsonSchema.exportSchema(&env, &.{.{ .name = "refined", .root = root }}, "refined", .{ .max_depth = 16, .max_output_bytes = 4096 }, .draft_2020_12);
    defer exact.deinit();
    try std.testing.expect(exact == .fail);
    var conservative = fx.JsonSchema.exportSchema(&env, &.{.{ .name = "refined", .root = root, .accuracy = .conservative }}, "refined", .{ .max_depth = 16, .max_output_bytes = 4096 }, .draft_2020_12);
    defer conservative.deinit();
    try std.testing.expect(conservative == .success);
    try std.testing.expect(std.mem.indexOf(u8, conservative.success.items, "integer") != null);
    const NullableApp = struct { item: ?i64 };
    const NullableRecord = fx.SchemaRecordBuilder(NullableApp, Env).begin().fields(.{fx.SchemaBuilder.required("item", fx.SchemaRepresentation.nullable(fx.SchemaRepresentation.signed_integer))}).policy(.{ .unknown_fields = .reject, .duplicate_fields = .reject, .issues = .first }).build();
    var shallow_limits = limits;
    shallow_limits.max_json_depth = 1;
    var shallow = fx.JsonWriter.Writer.init(&env, shallow_limits);
    defer shallow.deinit();
    var depth = NullableRecord.writeJson(&env, &.{ .item = null }, &shallow);
    defer depth.deinit();
    try std.testing.expect(depth == .fail and depth.fail.kind == .depth_limit);
}

test "new object partial owners and rich issue paths release across allocation failure" {
    const Scenario = struct {
        fn run(allocator: std.mem.Allocator) !void {
            const env: Env = .{ .allocator = allocator, .io = std.testing.io };
            const Enum = fx.JsonConstraints.Enumeration(Env, &.{"known"}, .preserve);
            const Text = fx.JsonConstraints.String(Env, .{ .minimum_scalars = 1, .maximum_scalars = 1, .format = .none }, null);
            const O = fx.JsonConstraints.Object(Env, .{ .{ .name = "first", .codec = Enum, .required = true }, .{ .name = "second", .codec = Text, .required = true } }, null, .{}, null, .{ .minimum_properties = 2, .maximum_properties = 2 }, null);
            var document = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, "{\"first\":\"future\",\"second\":\"bad\"}", .{ .allocate = .alloc_always });
            defer document.deinit();
            const Bounded = fx.JsonConstraints.withLimits(O, select);
            var result = Bounded.decodeDirect(&env, &document.value);
            defer result.deinit();
            if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expect(result == .fail);
            try std.testing.expectEqualStrings("second", result.fail.path.?[0].field.items);
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Scenario.run, .{});
}

test "cache set samples settlement after retaining replacement key" {
    const C = fx.Cache(u32, u32, CacheError);
    const Timing = struct {
        time: std.atomic.Value(i64) = .init(0),
        fn now(data: ?*anyopaque) std.Io.Timestamp {
            const self: *@This() = @ptrCast(@alignCast(data.?));
            return .{ .nanoseconds = self.time.load(.acquire) };
        }
        fn sleep(_: ?*anyopaque, _: std.Io.Duration) fx.Clock.SleepResult {
            return .done;
        }
        fn retain(data: ?*anyopaque, key: u32) error{OutOfMemory}!u32 {
            const self: *@This() = @ptrCast(@alignCast(data.?));
            self.time.store(2 * std.time.ns_per_s, .release);
            return key;
        }
        fn load(_: *anyopaque, key: u32, _: *const fx.Cancellation) fx.Exit(u32, CacheError) {
            return .{ .success = key };
        }
        fn releaseLoader(_: *anyopaque) void {}
    };
    const Threads = @import("support/thread_executor.zig").ThreadExecutor;
    var engine: Threads = .{ .allocator = std.testing.allocator, .capacity = 4, .capabilities = .{ .independent = true, .cpu = false, .blocking = false } };
    defer engine.deinit();
    var timing: Timing = .{};
    const TEnv = struct { allocator: std.mem.Allocator, io: std.Io, async_engine: fx.Executor, clock: fx.Clock };
    const env: TEnv = .{ .allocator = std.testing.allocator, .io = std.testing.io, .async_engine = engine.executor(), .clock = .{ .userdata = &timing, .vtable = &.{ .now = Timing.now, .sleep = Timing.sleep } } };
    const cfg: fx.CacheConfig(u32, CacheError) = .{ .keys = .{ .userdata = &timing, .equal = Keys.equal, .retain = Timing.retain, .release = Keys.release }, .time_to_live = .fromSeconds(1), .capacity = 2, .maximum_inflight = 1, .cache_failure = Keys.failure };
    const cache = C.create(&env, cfg, .{ .userdata = &timing, .run = Timing.load, .deinit = Timing.releaseLoader }).success;
    defer cache.deinit();
    defer {
        var closed = cache.shutdown();
        closed.deinit();
    }
    const value: u32 = 42;
    var replaced = cache.set(1, &value);
    defer replaced.deinit();
    try std.testing.expect(replaced == .success);
    cache.mutex.lockUncancelable(env.io);
    const accessed = cache.entries.?.accessed;
    const expires = cache.entries.?.expires.?;
    cache.mutex.unlock(env.io);
    try std.testing.expectEqual(@as(i96, 2 * std.time.ns_per_s), accessed.nanoseconds);
    try std.testing.expectEqual(@as(i96, 3 * std.time.ns_per_s), expires.nanoseconds);
}

const RecursiveFixture = struct {
    const Node = struct {
        pub const effect_owned_payload = true;
        value: i64,
        next: ?fx.RecursiveSchema.OwnedBox(@This()),
        pub fn deinit(self: *@This()) void {
            if (self.next) |*value| value.deinit();
        }
        pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
            return .{ .value = self.value, .next = if (self.next) |*value| try value.clone() else null };
        }
    };
    const Box = fx.RecursiveSchema.Box(Node, Env, 2, "node", schema);
    fn schema() type {
        return fx.SchemaRecordBuilder(Node, Env).begin().fields(.{ fx.SchemaBuilder.required("value", fx.JsonCodec.signed_integer), fx.SchemaBuilder.nestedOptional("next", Box) }).policy(.{ .unknown_fields = .reject, .duplicate_fields = .reject, .issues = .first }).build();
    }
    const Run = struct {
        env: Env,
        input: *const std.json.Value,
        success: bool = false,
        fn run(self: *@This()) void {
            var decoded = Box.decodeDirect(&self.env, self.input);
            defer decoded.deinit();
            self.success = decoded == .success;
        }
    };
};
test "recursive schema reuses immutable definition with independent concurrent traversal" {
    const env = environment();
    var document = try std.json.parseFromSlice(std.json.Value, env.allocator, "{\"value\":1,\"next\":{\"value\":2}}", .{ .allocate = .alloc_always });
    defer document.deinit();
    var first: RecursiveFixture.Run = .{ .env = env, .input = &document.value };
    var second: RecursiveFixture.Run = .{ .env = env, .input = &document.value };
    const a = try std.Thread.spawn(.{ .allocator = env.allocator }, RecursiveFixture.Run.run, .{&first});
    const b = std.Thread.spawn(.{ .allocator = env.allocator }, RecursiveFixture.Run.run, .{&second}) catch |err| {
        a.join();
        return err;
    };
    a.join();
    b.join();
    try std.testing.expect(first.success and second.success);
    var again = RecursiveFixture.Box.decodeDirect(&env, &document.value);
    defer again.deinit();
    try std.testing.expect(again == .success);
}

test "native record retains distinct decode and encode service environments" {
    const ReadEnv = struct { allocator: std.mem.Allocator, io: std.Io, increment: u8 };
    const WriteEnv = DirectionEnv;
    const C = comptime fx.Codec(u8, std.json.Value, ReadEnv, WriteEnv).init(struct {
        fn decode(v: *const std.json.Value) fx.Effect(u8, fx.SchemaIssue.Issue, ReadEnv) {
            return fx.Effect(u8, fx.SchemaIssue.Issue, ReadEnv).fromBorrowed(std.json.Value, v, struct {
                fn run(input: *const std.json.Value, env: *const ReadEnv) fx.Exit(u8, fx.SchemaIssue.Issue) {
                    return .{ .success = @as(u8, @intCast(input.integer)) + env.increment };
                }
            }.run);
        }
    }.decode, struct {
        fn encode(v: *const u8) fx.Effect(std.json.Value, fx.SchemaIssue.Issue, WriteEnv) {
            return fx.Effect(std.json.Value, fx.SchemaIssue.Issue, WriteEnv).fromBorrowed(u8, v, struct {
                fn run(input: *const u8, env: *const WriteEnv) fx.Exit(std.json.Value, fx.SchemaIssue.Issue) {
                    return .{ .success = .{ .integer = input.* + env.offset } };
                }
            }.run);
        }
    }.encode);
    const Object = struct {
        pub const Representation = std.json.Value;
        pub const Builder = std.json.ObjectMap;
        pub fn count(v: *const Representation) usize {
            return v.object.count();
        }
        pub fn nameAt(v: *const Representation, index: usize) []const u8 {
            return v.object.keys()[index];
        }
        pub fn find(v: *const Representation, name: []const u8) ?*const Representation {
            return v.object.getPtr(name);
        }
        pub fn init(_: *const WriteEnv, _: usize) fx.Exit(Builder, fx.SchemaIssue.Issue) {
            return .{ .success = .{} };
        }
        pub fn put(env: *const WriteEnv, builder: *Builder, name: []const u8, value: *const Representation) fx.Exit(void, fx.SchemaIssue.Issue) {
            builder.put(env.allocator, name, value.*) catch return .{ .die = fx.Defect.out_of_memory };
            return .{ .success = {} };
        }
        pub fn finish(_: *const WriteEnv, builder: *Builder) fx.Exit(Representation, fx.SchemaIssue.Issue) {
            const value = builder.*;
            builder.* = .{};
            return .{ .success = .{ .object = value } };
        }
        pub fn discard(builder: *Builder) void {
            builder.deinit(std.testing.allocator);
        }
    };
    const App = struct { value: u8 };
    const S = fx.NativeRecordBuilder(App, Object, ReadEnv, WriteEnv).fields(.{fx.NativeField.required("value", C)}).build(true);
    const read: ReadEnv = .{ .allocator = std.testing.allocator, .io = std.testing.io, .increment = 1 };
    const write: WriteEnv = .{ .allocator = std.testing.allocator, .io = std.testing.io, .offset = 3 };
    var input: std.json.Value = .{ .object = .{} };
    defer input.object.deinit(read.allocator);
    try input.object.put(read.allocator, "value", .{ .integer = 4 });
    var decoding = S.decode(&input);
    defer decoding.deinit();
    var decoded = fx.run(&read, decoding);
    defer decoded.deinit();
    try std.testing.expect(decoded == .success and decoded.success.value == 5);
    var encoding = S.encode(&decoded.success);
    defer encoding.deinit();
    var encoded = fx.run(&write, encoding);
    defer encoded.deinit();
    try std.testing.expect(encoded == .success and encoded.success.object.get("value").?.integer == 8);
    encoded.success.object.deinit(write.allocator);
}

test "cancelled blocked producer releases its registration without consuming queued value" {
    const Queue = fx.Queue(u32);
    const Work = struct {
        queue: *Queue,
        token: *fx.Cancellation,
        result: ?fx.Exit(void, fx.QueueError) = null,
        fn run(self: *@This()) void {
            self.result = self.queue.offerToken(2, self.token);
        }
    };
    const env = environment();
    var queue = Queue.init(&env, 1).success;
    defer queue.deinit();
    var filled = queue.offerToken(1, null);
    defer filled.deinit();
    var cancellation: fx.Cancellation = .{};
    var work: Work = .{ .queue = &queue, .token = &cancellation };
    const thread = try std.Thread.spawn(.{ .allocator = env.allocator }, Work.run, .{&work});
    var registered = false;
    for (0..100000) |_| {
        queue.mutex.lockUncancelable(env.io);
        registered = queue.offers != null;
        queue.mutex.unlock(env.io);
        if (registered) break;
        std.Thread.yield() catch {};
    }
    cancellation.request();
    thread.join();
    defer work.result.?.deinit();
    try std.testing.expect(registered and work.result.? == .interrupted);
    var original = queue.takeToken(null);
    defer original.deinit();
    try std.testing.expect(original == .success and original.success == 1);
    try std.testing.expect(cancellation.first == null);
}

test "reactivity watches observe fresh subscription generations and can reenter invalidation" {
    const env = environment();
    var registry = fx.InvalidationRegistry.init(&env, 4).success;
    defer registry.deinit();
    var key = fx.PrimaryKey.encodeDirect(&env, "watch", &.{.{ .text = "key" }});
    defer key.deinit();
    const watch = registry.watch(key.success.items).success;
    defer watch.deinit();
    var first = registry.invalidateBatch(&.{key.success.items});
    defer first.deinit();
    var notice_program = @TypeOf(watch.*).next(Env, watch);
    defer notice_program.deinit();
    var notice = fx.run(&env, notice_program);
    defer notice.deinit();
    try std.testing.expect(notice == .success and notice.success == .changed);
    const newer = registry.watch(key.success.items).success;
    defer newer.deinit();
    var second = registry.invalidateBatch(&.{key.success.items});
    defer second.deinit();
    var next_program = @TypeOf(newer.*).next(Env, newer);
    defer next_program.deinit();
    var next = fx.run(&env, next_program);
    defer next.deinit();
    try std.testing.expect(next == .success and next.success.changed > notice.success.changed);
    var reentered = registry.generation(key.success.items);
    defer reentered.deinit();
    try std.testing.expect(reentered == .success and reentered.success == next.success.changed);
}

test "std JSON emission preserves escaping exact tokens bounds and deep caller policy" {
    const env = environment();
    const text = "\x00\x01\x08\t\n\x0b\x0c\r\x1f\"\\é🙂";
    var output = fx.JsonWriter.Writer.init(&env, limits);
    defer output.deinit();
    try output.string(text);
    var parsed = try std.json.parseFromSlice(std.json.Value, env.allocator, output.output.items, .{});
    defer parsed.deinit();
    try std.testing.expectEqualStrings(text, parsed.value.string);
    var exact_limit = limits;
    exact_limit.max_output_bytes = output.output.items.len;
    var exact_output = fx.JsonWriter.Writer.init(&env, exact_limit);
    defer exact_output.deinit();
    try exact_output.string(text);
    exact_limit.max_output_bytes -= 1;
    var too_small = fx.JsonWriter.Writer.init(&env, exact_limit);
    defer too_small.deinit();
    try std.testing.expectError(error.SizeLimit, too_small.string(text));
    var failure = fx.JsonWriter.failureWith(&env, .encode, error.SizeLimit, &too_small);
    defer failure.deinit();
    try std.testing.expect(failure == .fail and failure.fail.detail.?.resource.limit == exact_limit.max_output_bytes);
    try std.testing.expectError(error.InvalidUnicode, exact_output.string("\xff"));
    var arena = std.heap.ArenaAllocator.init(env.allocator);
    defer arena.deinit();
    var object: std.json.ObjectMap = .{};
    try object.put(arena.allocator(), text, .{ .number_string = "9007199254740993.000e0" });
    try object.put(arena.allocator(), "finite", .{ .float = -std.math.floatMax(f64) });
    try object.put(arena.allocator(), "enabled", .{ .bool = true });
    const value: std.json.Value = .{ .object = object };
    var encoded = fx.JsonWriter.Writer.init(&env, .{ .max_output_bytes = 4096, .max_json_depth = 4, .max_value_bytes = 1024, .max_integer_digits = 512 });
    defer encoded.deinit();
    try encoded.json(&value);
    var roundtrip = try std.json.parseFromSlice(std.json.Value, env.allocator, encoded.output.items, .{ .parse_numbers = false });
    defer roundtrip.deinit();
    try std.testing.expectEqualStrings("9007199254740993.000e0", roundtrip.value.object.get(text).?.number_string);
    var deep: std.json.Value = .{ .integer = 1 };
    for (0..300) |_| {
        var array = std.json.Array.init(arena.allocator());
        try array.append(deep);
        deep = .{ .array = array };
    }
    var deep_limits = limits;
    deep_limits.max_json_depth = 301;
    var deep_output = fx.JsonWriter.Writer.init(&env, deep_limits);
    defer deep_output.deinit();
    try deep_output.json(&deep);
    try std.testing.expectEqual(@as(usize, 601), deep_output.output.items.len);
    deep_limits.max_json_depth = 300;
    var depth_denied = fx.JsonWriter.Writer.init(&env, deep_limits);
    defer depth_denied.deinit();
    try std.testing.expectError(error.DepthLimit, depth_denied.json(&deep));
    try std.testing.expectEqual(@as(usize, 301), depth_denied.resource_failure.?.resource.actual);
    const shape: fx.SchemaAst.Node = .{ .primitive = .boolean };
    const root_name = "quote\"\\\n/~é";
    var schema = fx.JsonSchema.exportSchema(&env, &.{.{ .name = root_name, .root = &shape }}, root_name, .{ .max_depth = 4, .max_output_bytes = 1024 }, .draft_2020_12);
    defer schema.deinit();
    try std.testing.expect(schema == .success);
    var schema_doc = try std.json.parseFromSlice(std.json.Value, env.allocator, schema.success.items, .{});
    defer schema_doc.deinit();
    try std.testing.expect(schema_doc.value.object.get("$defs").?.object.contains(root_name));
    try std.testing.expectEqualStrings("#/$defs/quote\"\\\n~1~0é", schema_doc.value.object.get("$ref").?.string);
    // The AST nesting budget is independent of std's local grammar assertions.
    const nodes = try arena.allocator().alloc(fx.SchemaAst.Node, 301);
    nodes[0] = shape;
    for (nodes[1..], 1..) |*node, index| node.* = .{ .array = &nodes[index - 1] };
    var deep_schema = fx.JsonSchema.exportSchema(&env, &.{.{ .name = "deep", .root = &nodes[300] }}, "deep", .{ .max_depth = 301, .max_output_bytes = 20000 }, .draft_2020_12);
    defer deep_schema.deinit();
    try std.testing.expect(deep_schema == .success);
}
