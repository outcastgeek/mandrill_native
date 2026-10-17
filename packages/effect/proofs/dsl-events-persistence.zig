const std = @import("std");
const fx = @import("effect");
const gates = @import("support/dsl_gate.zig");
const Issue = fx.SchemaIssue.Issue;
const Counts = struct { states: usize = 0, payloads: usize = 0 };
const R = struct {
    allocator: std.mem.Allocator,
    io: std.Io = std.testing.io,
    async_engine: fx.Executor = undefined,
    handled: *usize,
    counts: ?*Counts = null,
    gate: ?*gates.Gate(void, Issue) = null,
    storage: ?*const fx.KeyValueStore(@This(), void) = null,
};
const F = enum { invalid_amount };
const Handler = struct {
    fn deposit(value: *const i64, env: *const R) fx.Effect(i64, void, R) {
        env.handled.* += 1;
        return fx.Effect(i64, void, R).succeed(value.*);
    }
    fn double(value: *const bool, env: *const R) fx.Effect(i64, void, R) {
        env.handled.* += 1;
        return fx.Effect(i64, void, R).succeed(if (value.*) 2 else 1);
    }
    fn add(state: *i64, value: *const i64) fx.Result(void, F) {
        if (value.* < 0) return .{ .err = .invalid_amount };
        state.* += value.*;
        return .{ .ok = {} };
    }
    fn multiply(state: *i64, value: *const bool) void {
        if (value.*) state.* *= 2;
    }
};
const deposit = fx.dsl.event(.{ .tag = "deposit", .version = 1, .payload = fx.JsonCodec.signed_integer, .success = fx.JsonCodec.signed_integer, .failure = fx.JsonCodec.null_value, .handler = Handler.deposit });
const double = fx.dsl.event(.{ .tag = "double", .version = 2, .payload = fx.JsonCodec.boolean, .success = fx.JsonCodec.signed_integer, .failure = fx.JsonCodec.null_value, .handler = Handler.double });
const Projection = fx.dsl.projection(.{ .state = fx.JsonCodec.signed_integer, .cases = .{ fx.dsl.eventCase(deposit, Handler.add), fx.dsl.eventCase(double, Handler.multiply) }, .limits = .{ .max_document_bytes = 256, .max_depth = 8, .max_value_bytes = 64, .max_integer_digits = 10 } });
fn page(allocator: std.mem.Allocator, tags: []const []const u8, texts: []const []const u8) !fx.EventReadPage {
    const entries = try allocator.alloc(fx.EventEntry, tags.len);
    var count: usize = 0;
    errdefer {
        for (entries[0..count]) |*entry| entry.deinit();
        allocator.free(entries);
    }
    for (tags, texts, 0..) |tag, text, i| {
        const tag_bytes = try allocator.dupe(u8, tag);
        errdefer allocator.free(tag_bytes);
        const event_id = try allocator.dupe(u8, "fixture-event-id");
        errdefer allocator.free(event_id);
        const encoded = try allocator.dupe(u8, text);
        entries[i] = .{ .tag = .{ .items = tag_bytes, .allocator = allocator }, .event_id = .{ .items = event_id, .allocator = allocator }, .encoded = .{ .items = encoded, .allocator = allocator }, .cursor = .{ .sequence = i + 1 }, .schema_version = if (std.mem.eql(u8, tag, "double")) 2 else 1 };
        count += 1;
    }
    return .{ .entries = .{ .items = entries, .allocator = allocator }, .next = .{ .sequence = entries.len }, .more = false };
}
fn check(allocator: std.mem.Allocator) !void {
    var handled: usize = 0;
    const env: R = .{ .allocator = allocator, .handled = &handled };
    var history = try page(allocator, &.{ "deposit", "double" }, &.{ "4", "true" });
    defer history.deinit();
    const initial: i64 = 1;
    var program = Projection.replayPage(&env, &initial, .before_first, &history);
    defer program.deinit();
    comptime if (@TypeOf(program) != fx.Effect(fx.EventProjection.Replayed(i64), fx.EventProjection.Error(F), R)) @compileError("typed replay must remain exact inferred Effect");
    var result = fx.run(&env, program);
    defer result.deinit();
    if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(i64, 10), result.success.state);
    try std.testing.expectEqual(@as(u64, 2), result.success.cursor.sequence);
    try std.testing.expectEqual(@as(usize, 0), handled);
    try std.testing.expectEqual(@as(i64, 1), initial);
}
test "EVENT-01 EVENT-02 INF-09 COMP-03 typed event cases derive decoder identity and pure fold associations" {
    try check(std.testing.allocator);
}
test "LIFE-07 typed case decoding and replay release partial parse owners at every allocation failure" {
    try std.testing.checkAllAllocationFailures(std.testing.allocator, check, .{});
}

const OwnedState = struct {
    pub const effect_owned_payload = true;
    allocator: std.mem.Allocator,
    bytes: []u8,
    amount: i64,
    counts: *Counts,
    pub fn deinit(self: *@This()) void {
        self.allocator.free(self.bytes);
        self.counts.states -= 1;
    }
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        const bytes = try self.allocator.dupe(u8, self.bytes);
        self.counts.states += 1;
        return .{ .allocator = self.allocator, .bytes = bytes, .amount = self.amount, .counts = self.counts };
    }
};
const OwnedAmount = struct {
    pub const effect_owned_payload = true;
    allocator: std.mem.Allocator,
    bytes: []u8,
    value: i64,
    counts: *Counts,
    pub fn deinit(self: *@This()) void {
        self.allocator.free(self.bytes);
        self.counts.payloads -= 1;
    }
};
const Owned = struct {
    const Decode = fx.Effect(OwnedAmount, Issue, R);
    const FoldError = enum { out_of_memory, invalid };
    fn decode(input: *const std.json.Value) Decode {
        return Decode.deferBorrowed(std.json.Value, input, struct {
            fn start(value: *const std.json.Value, env: *const R) Decode {
                if (env.gate != null) {
                    var waiting = fx.Effect(void, Issue, R).fromAsync(struct {
                        fn begin(current: *const R) fx.Async(void, Issue).Start {
                            return current.gate.?.start();
                        }
                    }.begin);
                    return waiting.flatMapCapturedOwned(env, OwnedAmount, value, struct {
                        fn resumed(pointer: *const *const std.json.Value, _: void, _: *const R) Decode {
                            return Decode.fromBorrowed(std.json.Value, pointer.*, convert);
                        }
                    }.resumed);
                }
                return Decode.fromBorrowed(std.json.Value, value, convert);
            }
        }.start);
    }
    fn convert(input: *const std.json.Value, env: *const R) fx.Exit(OwnedAmount, Issue) {
        if (input.* != .number_string) return .{ .fail = Issue.init(env, .decode, .invalid_value) };
        const value = std.fmt.parseInt(i64, input.number_string, 10) catch return .{ .fail = Issue.init(env, .decode, .invalid_number) };
        const bytes = env.allocator.dupe(u8, input.number_string) catch return .{ .die = fx.Defect.out_of_memory };
        env.counts.?.payloads += 1;
        return .{ .success = .{ .allocator = env.allocator, .bytes = bytes, .value = value, .counts = env.counts.? } };
    }
    fn encode(input: *const OwnedAmount) fx.Effect(std.json.Value, Issue, R) {
        return fx.Effect(std.json.Value, Issue, R).fromBorrowed(OwnedAmount, input, struct {
            fn value(payload: *const OwnedAmount, _: *const R) fx.Exit(std.json.Value, Issue) {
                return .{ .success = .{ .number_string = payload.bytes } };
            }
        }.value);
    }
    fn handler(input: *const OwnedAmount, _: *const R) fx.Effect(i64, void, R) {
        return fx.Effect(i64, void, R).succeed(input.value);
    }
    fn fold(state: *OwnedState, payload: *const OwnedAmount) fx.Result(void, FoldError) {
        if (payload.value < 0) return .{ .err = .invalid };
        const bytes = state.allocator.dupe(u8, payload.bytes) catch return .{ .err = .out_of_memory };
        state.allocator.free(state.bytes);
        state.bytes = bytes;
        state.amount += payload.value;
        return .{ .ok = {} };
    }
    fn decodeState(_: *const std.json.Value) fx.Result(OwnedState, Issue) {
        return .{ .err = Issue.pure(.decode, .invalid_value) };
    }
    fn encodeState(state: *const OwnedState) fx.Result(std.json.Value, Issue) {
        return .{ .ok = .{ .integer = state.amount } };
    }
};
const owned_codec = fx.Codec(OwnedAmount, std.json.Value, R, R).init(Owned.decode, Owned.encode);
const owned_handler = fx.dsl.event(.{ .tag = "owned", .version = 1, .payload = owned_codec, .success = fx.JsonCodec.signed_integer, .failure = fx.JsonCodec.null_value, .handler = Owned.handler });
const owned_state_codec = fx.PureCodec(OwnedState, std.json.Value).init(Owned.decodeState, Owned.encodeState);
const OwnedProjection = fx.dsl.projection(.{ .state = owned_state_codec, .cases = .{fx.dsl.events.caseWith(owned_handler, .{ .fold = Owned.fold, .duplicate_fields = .reject })}, .limits = .{ .max_document_bytes = 256, .max_depth = 8, .max_value_bytes = 64 } });
fn checkOwned(allocator: std.mem.Allocator) !void {
    var handled: usize = 0;
    var counts: Counts = .{};
    const env: R = .{ .allocator = allocator, .handled = &handled, .counts = &counts };
    var history = try page(allocator, &.{ "owned", "owned" }, &.{ "3", "4" });
    defer history.deinit();
    var initial: OwnedState = .{ .allocator = allocator, .bytes = try allocator.dupe(u8, "initial"), .amount = 1, .counts = &counts };
    counts.states += 1;
    defer initial.deinit();
    var program = OwnedProjection.replayPage(&env, &initial, .before_first, &history);
    defer program.deinit();
    var result = fx.run(&env, program);
    const oom = (result == .die and result.die.kind == .out_of_memory) or (result == .fail and result.fail == .fold and result.fail.fold == .out_of_memory);
    if (!oom) {
        try std.testing.expect(result == .success);
        try std.testing.expectEqual(@as(i64, 8), result.success.state.amount);
        try std.testing.expectEqualStrings("4", result.success.state.bytes);
    }
    result.deinit();
    try std.testing.expectEqual(@as(usize, 1), counts.states);
    try std.testing.expectEqual(@as(usize, 0), counts.payloads);
    try std.testing.expectEqualStrings("initial", initial.bytes);
    if (oom) return error.OutOfMemory;
}
test "EVENT-07 owning contextual event payloads and private projection state survive partial decoding and every allocation failure" {
    try checkOwned(std.testing.allocator);
    try std.testing.checkAllAllocationFailures(std.testing.allocator, checkOwned, .{});
}
test "EVENT-03 EVENT-07 suspended decoding retains parse storage and cancellation discards private owning state" {
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = true }, .maximum_owned = 4 };
    defer engine.deinit();
    var gate: gates.Gate(void, Issue) = .{ .result = .{ .success = {} } };
    var counts: Counts = .{};
    var handled: usize = 0;
    const env: R = .{ .allocator = std.testing.allocator, .async_engine = engine.executor(), .handled = &handled, .counts = &counts, .gate = &gate };
    var history = try page(env.allocator, &.{"owned"}, &.{"3"});
    defer history.deinit();
    var initial: OwnedState = .{ .allocator = env.allocator, .bytes = try env.allocator.dupe(u8, "initial"), .amount = 1, .counts = &counts };
    counts.states = 1;
    defer initial.deinit();
    var program = OwnedProjection.replayPage(&env, &initial, .before_first, &history);
    defer program.deinit();
    var pending = fx.runAsync(&env, program);
    defer pending.deinit();
    gate.started.waitUncancelable(env.io);
    try std.testing.expectEqual(@as(usize, 2), counts.states);
    pending.request();
    var result = pending.join();
    defer result.deinit();
    try std.testing.expect(result == .interrupted);
    try std.testing.expect(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
    try std.testing.expectEqual(@as(usize, 1), counts.states);
    try std.testing.expectEqual(@as(usize, 0), counts.payloads);
}

const Persistence = struct {
    const Failure = fx.PersistenceError(void);
    const Out = fx.Effect(i64, Failure, R);
    const Input = struct {
        key: fx.OwnedSlice(u8),
        owner: struct {
            pub const effect_owned_payload = true;
            disposed: *usize,
            pub fn deinit(self: *@This()) void {
                self.disposed.* += 1;
            }
        },
    };
    fn encode(value: *const i64, env: *const R) fx.Exit(fx.OwnedSlice(u8), Issue) {
        const bytes = std.fmt.allocPrint(env.allocator, "{d}", .{value.*}) catch return .{ .die = fx.Defect.out_of_memory };
        return .{ .success = .{ .items = bytes, .allocator = env.allocator } };
    }
    fn encodeError(_: *const void, env: *const R) fx.Exit(fx.OwnedSlice(u8), Issue) {
        return .{ .success = .{ .items = env.allocator.dupe(u8, "unit") catch return .{ .die = fx.Defect.out_of_memory }, .allocator = env.allocator } };
    }
    fn decode(bytes: []const u8, env: *const R) fx.Exit(i64, Issue) {
        return .{ .success = std.fmt.parseInt(i64, bytes, 10) catch return .{ .fail = Issue.init(env, .decode, .invalid_number) } };
    }
    fn decodeError(_: []const u8, _: *const R) fx.Exit(void, Issue) {
        return .{ .success = {} };
    }
    const codec = fx.dsl.persistence.payloadCodec(.{ .encode_success = encode, .encode_failure = encodeError, .decode_success = decode, .decode_failure = decodeError });
    fn store(_: *const Input, env: *const R) *const fx.KeyValueStore(R, void) {
        return env.storage.?;
    }
    fn key(input: *const Input, _: *const R) []const u8 {
        return input.key.items;
    }
    fn expiry(_: *const Input, _: *const R) fx.Deadline {
        return .never;
    }
    fn body(view: anytype, env: *const R) Out {
        var writing = view.write(env, .{ .success = 27 });
        return writing.flatMapCapturedOwned(env, i64, view, struct {
            fn written(pointer: *const @TypeOf(view), ack: fx.WriteAcknowledgement, current: *const R) Out {
                if (ack != .committed) return Out.fromOwnedOutcome(current, .{ .fail = .uncertain_commit });
                var reading = pointer.*.read(current);
                return reading.mapOwned(current, struct {
                    fn value(read: fx.PersistedResultRead(i64, void)) i64 {
                        var owned = read;
                        defer owned.deinit();
                        std.debug.assert(owned == .found and owned.found.value == .success);
                        return owned.found.value.success;
                    }
                }.value);
            }
        }.written);
    }
    fn now(_: *const R) fx.WallInstant {
        return .{ .unix_nanoseconds = 0 };
    }
    const metadata = .{ .codec = codec, .schema_version = 3, .limits = .{ .max_payload_bytes = 1024, .max_cleanup_records = 8, .max_total_bytes = 4096 }, .store = store, .key = key, .expiry = expiry, .body = body };
};
fn checkPersistence(allocator: std.mem.Allocator) !void {
    var handled: usize = 0;
    var disposed: usize = 0;
    var env: R = .{ .allocator = allocator, .handled = &handled };
    var memory = fx.MemoryPersistence(R, Persistence.now).init(&env);
    defer memory.deinit();
    const store = memory.keyValue();
    env.storage = &store;
    const key = fx.PrimaryKey.encodeDirect(&env, "projection", &.{.{ .text = "account" }});
    if (key == .die and key.die.kind == .out_of_memory) return error.OutOfMemory;
    std.debug.assert(key == .success);
    var program = fx.dsl.persistence.declared(&env, .{ .definition = fx.dsl.declareMetadata(Persistence.metadata), .inputs = Persistence.Input{ .key = key.success, .owner = .{ .disposed = &disposed } } });
    comptime if (@TypeOf(program) != Persistence.Out) @compileError("captured persistence must expose exact inferred Effect");
    var result = fx.run(&env, program);
    program.deinit();
    defer result.deinit();
    try std.testing.expectEqual(@as(usize, 1), disposed);
    if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(i64, 27), result.success);
}
test "EVENT-05 DECL-01 DECL-03 typed persistence derives channels and owns runtime keys through write and read including every OOM" {
    try checkPersistence(std.testing.allocator);
    try std.testing.checkAllAllocationFailures(std.testing.allocator, checkPersistence, .{});
}
test "EVENT-03 EVENT-04 failed decode fold unknown identity and version never publish partial cursor or state" {
    const Mode = enum { decoding, fold, identity, version, history_gap, page_cursor };
    inline for (std.meta.tags(Mode)) |mode| {
        var handled: usize = 0;
        const env: R = .{ .allocator = std.testing.allocator, .handled = &handled };
        var history = try page(env.allocator, &.{ "deposit", "double" }, &.{ "4", if (mode == .decoding) "42" else "true" });
        defer history.deinit();
        if (mode == .fold) {
            env.allocator.free(history.entries.items[0].encoded.items);
            history.entries.items[0].encoded.items = try env.allocator.dupe(u8, "-1");
        }
        if (mode == .identity) {
            env.allocator.free(history.entries.items[1].tag.items);
            history.entries.items[1].tag.items = try env.allocator.dupe(u8, "foreign");
        }
        if (mode == .version) history.entries.items[1].schema_version = 3;
        if (mode == .history_gap) history.entries.items[1].cursor.sequence = 3;
        if (mode == .page_cursor) history.next.sequence = 3;
        const initial: i64 = 1;
        var program = Projection.replayPage(&env, &initial, .before_first, &history);
        defer program.deinit();
        var result = fx.run(&env, program);
        defer result.deinit();
        try std.testing.expect(result == .fail);
        switch (mode) {
            .decoding => try std.testing.expect(result.fail == .schema),
            .fold => try std.testing.expect(result.fail == .fold and result.fail.fold == .invalid_amount),
            .identity => try std.testing.expect(result.fail == .unknown_event),
            .version => try std.testing.expect(result.fail == .incompatible_version),
            .history_gap => try std.testing.expect(result.fail == .history_gap),
            .page_cursor => try std.testing.expect(result.fail == .malformed_page),
        }
        try std.testing.expectEqual(@as(i64, 1), initial);
        try std.testing.expectEqual(@as(usize, 0), handled);
    }
}

test "DECL-01 DECL-03 DECL-04 INF-08 entity and event declarations normalize codecs and mix with canonical legacy registries" {
    const Helpers = struct {
        fn entity(payload: *const i64, _: *const R) fx.Effect(i64, void, R) {
            return fx.Effect(i64, void, R).succeed(payload.*);
        }
        fn request(_: *const R) *const i64 {
            return &@as(i64, 11);
        }
    };
    const declared = comptime fx.dsl.entity(.{ .group = "account", .version = 1, .request = fx.JsonCodec.signed_integer, .success = fx.JsonCodec.signed_integer, .failure = fx.JsonCodec.null_value, .handler = Helpers.entity });
    const legacy = comptime fx.ClusterEntity(@TypeOf(fx.JsonCodec.signed_integer), @TypeOf(fx.JsonCodec.signed_integer), @TypeOf(fx.JsonCodec.null_value)).define("legacy", 1, fx.JsonCodec.signed_integer, fx.JsonCodec.signed_integer, fx.JsonCodec.null_value).handledBy(R, Helpers.entity);
    const Registry = fx.ClusterEntityRegistry(.{ declared, legacy });
    try std.testing.expect(Registry.resolve("account", 1) == .ok);
    try std.testing.expect(Registry.resolve("legacy", 1) == .ok);
    var handled: usize = 0;
    const env: R = .{ .allocator = std.testing.allocator, .handled = &handled };
    const description = comptime declared.invoke(Helpers.request);
    comptime if (@TypeOf(description) != fx.Effect(i64, void, R)) @compileError("entity invocation must remain exact Effect");
    var result = fx.run(&env, description);
    defer result.deinit();
    try std.testing.expect(result == .success and result.success == 11);
    const Events = fx.EventGroup(.{ deposit, double });
    try std.testing.expect(Events.resolve("deposit", 1) == .ok);
    try std.testing.expect(Events.resolve("double", 2) == .ok);
}
