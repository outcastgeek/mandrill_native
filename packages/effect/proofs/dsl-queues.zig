const std = @import("std");
const fx = @import("effect");
const Issue = fx.SchemaIssue.Issue;
const Store = fx.PersistedQueue(R, void, .@"volatile");
const Failure = fx.PersistenceError(void);
const Tracker = struct {
    backing: [256]u8 = undefined,
    memory: std.heap.FixedBufferAllocator = undefined,
    decoded: usize = 0,
    disposed: usize = 0,
    fail_owner: ?*std.testing.FailingAllocator = null,
};
var active_tracker: *Tracker = undefined;
const Payload = struct {
    pub const effect_owned_payload = true;
    bytes: []const u8,
    tracker: *Tracker,
    pub fn deinit(self: *@This()) void {
        // Assert allocator lifetime before inspecting the borrowed representation.
        std.debug.assert(self.tracker.memory.end_index != 0);
        std.debug.assert(std.mem.eql(u8, self.bytes, "payload"));
        self.tracker.disposed += 1;
    }
};
const R = struct {
    allocator: std.mem.Allocator,
    io: std.Io = std.testing.io,
    tracker: *Tracker,
    store: *const Store,
};
const Fixture = struct {
    fn decode(bytes: *const []const u8) fx.Result(Payload, Issue) {
        active_tracker.decoded += 1;
        if (active_tracker.fail_owner) |failing| failing.fail_index = failing.alloc_index;
        return .{ .ok = .{ .bytes = bytes.*, .tracker = active_tracker } };
    }
    fn encode(payload: *const Payload) fx.Result([]const u8, Issue) {
        return .{ .ok = payload.bytes };
    }
    const wire = fx.PureSchemaBinary(fx.PureCodec(Payload, []const u8).init(decode, encode), "queue-payload");
    const limits: wire.LimitsType = .{ .max_schema_name_bytes = 16, .max_payload_bytes = 32, .max_frame_bytes = 64 };
    fn store(env: *const R) *const Store {
        return env.store;
    }
    fn key(_: *const R) []const u8 {
        return "unused";
    }
    fn offerKey(_: *const void, _: *const Payload, _: *const R) []const u8 {
        return "unused";
    }
    fn visible(_: *const R) fx.WallInstant {
        return .{ .unix_nanoseconds = 0 };
    }
    fn retention(_: *const R) fx.Deadline {
        return .never;
    }
    fn generation(_: *const R) u64 {
        return 1;
    }
    fn body(view: anytype, env: *const R) fx.Effect(@typeInfo(@TypeOf(view)).pointer.child.DecodedClaim, Failure, R) {
        const Out = fx.Effect(@typeInfo(@TypeOf(view)).pointer.child.DecodedClaim, Failure, R);
        const source = Payload{ .bytes = "payload", .tracker = env.tracker };
        const encoded = wire.encodeDirect(&.{ .allocator = env.tracker.memory.allocator() }, &source, limits);
        std.debug.assert(encoded == .success);
        var item: @typeInfo(@FieldType(@FieldType(Store.ClaimEffect.Success, "claimed"), "items")).pointer.child = .{
            .queue_key = .{ .items = env.allocator.dupe(u8, "queue") catch {
                var bytes = encoded.success;
                bytes.deinit();
                return Out.die(fx.Defect.out_of_memory);
            }, .allocator = env.allocator },
            .offer_key = undefined,
            .encoded_payload = encoded.success,
            .owner_id = undefined,
            .generation = 1,
            .attempt = 1,
        };
        item.offer_key = .{ .items = env.allocator.dupe(u8, "offer") catch {
            item.queue_key.deinit();
            item.encoded_payload.deinit();
            return Out.die(fx.Defect.out_of_memory);
        }, .allocator = env.allocator };
        item.owner_id = .{ .items = env.allocator.dupe(u8, "worker") catch {
            item.queue_key.deinit();
            item.offer_key.deinit();
            item.encoded_payload.deinit();
            return Out.die(fx.Defect.out_of_memory);
        }, .allocator = env.allocator };
        return view.decode(env, item);
    }
    const metadata = .{ .store = store, .codec = wire, .limits = limits, .key = key, .offer_key = offerKey, .visible_at = visible, .retention_until = retention, .owner = key, .generation = generation, .claim_policy = .{ .max_items = 1, .lease_until = fx.Deadline{ .at = .{ .unix_nanoseconds = 100 } } }, .body = body };
};
fn check(allocator: std.mem.Allocator, retain: bool) !void {
    var tracker: Tracker = .{};
    tracker.memory = .init(&tracker.backing);
    active_tracker = &tracker;
    const store: Store = undefined; // Decode does not call any provider hook.
    const env: R = .{ .allocator = allocator, .tracker = &tracker, .store = &store };
    var program = fx.dsl.queues.scoped(&env, {}, Fixture.metadata);
    var result = fx.run(&env, program);
    program.deinit();
    const oom = result == .die and result.die.kind == .out_of_memory;
    if (result == .success) {
        try std.testing.expectEqualSlices(u8, "payload", result.success.value().bytes);
        if (retain) {
            var retained = try result.success.clone();
            result.deinit();
            try std.testing.expectEqual(@as(usize, 0), tracker.disposed);
            try std.testing.expect(tracker.memory.end_index != 0);
            try std.testing.expectEqualSlices(u8, "payload", retained.value().bytes);
            retained.deinit();
        } else result.deinit();
    } else result.deinit();
    try std.testing.expectEqual(tracker.decoded, tracker.disposed);
    try std.testing.expectEqual(@as(usize, 0), tracker.memory.end_index);
    if (oom) return error.OutOfMemory;
    try std.testing.expectEqual(@as(usize, 1), tracker.decoded);
}
test "EVENT-05 LIFE-07 LIFE-11 decoded queue payload is disposed before its claimed representation including retained clones and every allocation failure" {
    try check(std.testing.allocator, false);
    try check(std.testing.allocator, true);
    try std.testing.checkAllAllocationFailures(std.testing.allocator, check, .{false});
    try std.testing.checkAllAllocationFailures(std.testing.allocator, check, .{true});
}

test "EVENT-05 LIFE-07 decoded claim owner allocation failure disposes the payload while representation is live" {
    var failing = std.testing.FailingAllocator.init(std.testing.allocator, .{});
    var tracker: Tracker = .{ .fail_owner = &failing };
    tracker.memory = .init(&tracker.backing);
    active_tracker = &tracker;
    const store: Store = undefined;
    const env: R = .{ .allocator = failing.allocator(), .tracker = &tracker, .store = &store };
    var program = fx.dsl.queues.scoped(&env, {}, Fixture.metadata);
    var result = fx.run(&env, program);
    program.deinit();
    defer result.deinit();
    try std.testing.expect(result == .die and result.die.kind == .out_of_memory);
    try std.testing.expectEqual(@as(usize, 1), tracker.decoded);
    try std.testing.expectEqual(@as(usize, 1), tracker.disposed);
    try std.testing.expectEqual(@as(usize, 0), tracker.memory.end_index);
    try std.testing.expect(failing.has_induced_failure);
    try std.testing.expectEqual(failing.allocated_bytes, failing.freed_bytes);
}

const Memory = struct {
    const Env = struct {
        allocator: std.mem.Allocator,
        io: std.Io = std.testing.io,
        provider: ?*const Provider = null,
    };
    const Provider = fx.PersistedQueue(Env, void, .@"volatile");
    const Out = fx.Effect(i64, Failure, Env);
    const Input = struct {
        queue_key: fx.OwnedSlice(u8),
        offer_key: fx.OwnedSlice(u8),
        owner: struct {
            pub const effect_owned_payload = true;
            disposed: *usize,
            pub fn deinit(self: *@This()) void {
                self.disposed.* += 1;
            }
        },
    };
    fn decode(bytes: *const []const u8) fx.Result(i64, Issue) {
        return .{ .ok = std.fmt.parseInt(i64, bytes.*, 10) catch return .{ .err = Issue.pure(.decode, .invalid_number) } };
    }
    fn encode(value: *const i64) fx.Result([]const u8, Issue) {
        return if (value.* == 27) .{ .ok = "27" } else .{ .err = Issue.pure(.encode, .invalid_value) };
    }
    const wire = fx.PureSchemaBinary(fx.PureCodec(i64, []const u8).init(decode, encode), "number-v1");
    fn store(env: *const Env) *const Provider {
        return env.provider.?;
    }
    fn key(input: *const Input, _: *const Env) []const u8 {
        return input.queue_key.items;
    }
    fn offerKey(input: *const Input, _: *const i64, _: *const Env) []const u8 {
        return input.offer_key.items;
    }
    fn owner(_: *const Env) []const u8 {
        return "worker";
    }
    fn generation(_: *const Env) u64 {
        return 7;
    }
    fn now(_: *const Env) fx.WallInstant {
        return .{ .unix_nanoseconds = 0 };
    }
    fn retention(_: *const Env) fx.Deadline {
        return .never;
    }
    fn consume(view: anytype, result_claim: Provider.ClaimEffect.Success, env: *const Env) Out {
        std.debug.assert(result_claim == .claimed and result_claim.claimed.items.len == 1);
        const item = result_claim.claimed.items[0];
        result_claim.claimed.allocator.free(result_claim.claimed.items);
        std.debug.assert(item.generation == 7 and (item.attempt == 1 or item.attempt == 2));
        var decoding = view.decode(env, item);
        return decoding.flatMapCapturedOwned(env, i64, view, struct {
            fn decoded(chosen: *const @TypeOf(view), value: @typeInfo(@TypeOf(view)).pointer.child.DecodedClaim, current: *const Env) Out {
                std.debug.assert(value.value().* == 27);
                if (value.token().attempt == 1) {
                    var retrying = chosen.*.retry(current, value, now(current));
                    return retrying.flatMapCapturedOwned(current, i64, chosen.*, struct {
                        fn retried(pointer: *const @TypeOf(view), result: Provider.RetryEffect.Success, latest: *const Env) Out {
                            std.debug.assert(result == .scheduled);
                            return claim(pointer.*, latest);
                        }
                    }.retried);
                }
                var acknowledging = chosen.*.acknowledge(current, value);
                return acknowledging.mapOwned(current, struct {
                    fn acknowledged(result: Provider.AckEffect.Success) i64 {
                        std.debug.assert(result == .acknowledged);
                        return 27;
                    }
                }.acknowledged);
            }
        }.decoded);
    }
    fn claim(view: anytype, env: *const Env) Out {
        var claiming = view.claim(env);
        return claiming.flatMapCapturedOwned(env, i64, view, struct {
            fn claimed(pointer: *const @TypeOf(view), result: Provider.ClaimEffect.Success, current: *const Env) Out {
                return consume(pointer.*, result, current);
            }
        }.claimed);
    }
    fn body(view: anytype, env: *const Env) Out {
        var offering = view.offer(env, 27);
        comptime if (@TypeOf(offering) != Provider.OfferEffect) @compileError("queue offer must be an exact native Effect");
        return offering.flatMapCapturedOwned(env, i64, view, struct {
            fn offered(pointer: *const @TypeOf(view), result: Provider.OfferEffect.Success, current: *const Env) Out {
                std.debug.assert(result == .accepted);
                return claim(pointer.*, current);
            }
        }.offered);
    }
    const metadata = .{ .store = store, .codec = wire, .limits = .{ .max_schema_name_bytes = 16, .max_payload_bytes = 32, .max_frame_bytes = 64 }, .key = key, .offer_key = offerKey, .owner = owner, .generation = generation, .visible_at = now, .retention_until = retention, .claim_policy = .{ .max_items = 1, .lease_until = fx.Deadline{ .at = .{ .unix_nanoseconds = 100 } } }, .body = body };
};
fn checkMemory(allocator: std.mem.Allocator) !void {
    var disposed: usize = 0;
    var env: Memory.Env = .{ .allocator = allocator };
    var memory = fx.MemoryPersistedQueue(Memory.Env, Memory.now).init(&env);
    defer memory.deinit();
    const store = memory.capability();
    env.provider = &store;
    const key = fx.PrimaryKey.encodeDirect(&env, "queue", &.{.{ .text = "jobs" }});
    if (key != .success) return error.OutOfMemory;
    var queue_key = key.success;
    const offered = fx.PrimaryKey.encodeDirect(&env, "offer", &.{.{ .text = "job-1" }});
    if (offered != .success) {
        queue_key.deinit();
        return error.OutOfMemory;
    }
    var program = fx.dsl.queues.declared(&env, .{ .definition = fx.dsl.declareMetadata(Memory.metadata), .inputs = Memory.Input{ .queue_key = queue_key, .offer_key = offered.success, .owner = .{ .disposed = &disposed } } });
    comptime if (@TypeOf(program) != Memory.Out) @compileError("queue scope must expose exact inferred Effect");
    var result = fx.run(&env, program);
    program.deinit();
    defer result.deinit();
    try std.testing.expectEqual(@as(usize, 1), disposed);
    if (result == .die and result.die.kind == .out_of_memory) return error.OutOfMemory;
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(i64, 27), result.success);
}
test "EVENT-05 DECL-01 DECL-03 COMP-03 typed queues retain runtime keys through native offer claim decode retry and acknowledgement including every allocation failure" {
    try checkMemory(std.testing.allocator);
    try std.testing.checkAllAllocationFailures(std.testing.allocator, checkMemory, .{});
}
