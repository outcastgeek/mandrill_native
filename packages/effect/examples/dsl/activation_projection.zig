//! EX-08: activation resources own typed replay and atomic checkpoint work.
const std = @import("std");
const fx = @import("effect");
const p = fx.dsl.program;
const rpc = @import("rpc_session.zig");
const A = fx.dsl.activation;
pub const E = enum { stale, unavailable, invalid, projection, persistence, conflict, rpc };
pub const R = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    generation: u64 = 1,
    acquired: *usize,
    released: *usize,
    resource: *const Resource = undefined,
    checkpoints: *const Store = undefined,
    rpc: ?*const rpc.Root = null,
};
const Store = fx.AtomicProjectionCheckpoint(R, void);
pub const Live = struct { generation: u64 };
const Resource = fx.ClusterEntityResource(*Live, E, R);
const Number = fx.Effect(i64, E, R);
const Event = fx.dsl.event(.{ .tag = "credit", .version = 1, .payload = fx.JsonCodec.signed_integer, .success = fx.JsonCodec.signed_integer, .failure = fx.JsonCodec.null_value, .handler = Calls.event });
const Projection = fx.dsl.projection(.{ .state = fx.JsonCodec.signed_integer, .cases = .{fx.dsl.eventCase(Event, Calls.fold)}, .limits = .{ .max_document_bytes = 128, .max_depth = 4, .max_value_bytes = 32 } });
const Loaded = struct { state: i64, cursor: fx.EventCursor, revision: fx.ProjectionCheckpointRevision };
const Input = struct { id: []const u8, generation: u64, history: *const fx.EventReadPage };
const Work = struct { request: *const Input, resource: fx.ResourceValue(*Live) };
pub const Calls = struct {
    fn event(value: *const i64, _: *const R) fx.Effect(i64, void, R) {
        return fx.Effect(i64, void, R).succeed(value.*);
    }
    fn fold(state: *i64, value: *const i64) void {
        state.* += value.*;
    }
    fn select(env: *const R) *const Resource {
        return env.resource;
    }
    pub fn acquire(_: ?*anyopaque, env: *const R, selected_token: A.Token) Resource.AcquireEffect {
        return Resource.AcquireEffect.fromOwned(env, selected_token.generation, struct {
            fn create(generation: *const u64, current: *const R) fx.Exit(*Live, E) {
                const live = current.allocator.create(Live) catch return .{ .die = fx.Defect.out_of_memory };
                live.* = .{ .generation = generation.* };
                current.acquired.* += 1;
                return .{ .success = live };
            }
        }.create);
    }
    pub fn release(_: ?*anyopaque, env: *const R, live: *Live, _: fx.PrimaryOutcome(E)) fx.Effect(void, E, R) {
        return fx.Effect(void, E, R).fromOwned(env, live, struct {
            fn destroy(pointer: *const *Live, current: *const R) fx.Exit(void, E) {
                current.allocator.destroy(pointer.*);
                current.released.* += 1;
                return .{ .success = {} };
            }
        }.destroy);
    }
    fn token(input: *const Input, _: *const R) A.Token {
        return .{ .address = .{ .group = "accounts", .entity_key = input.id }, .owner_id = "projection-worker", .generation = input.generation };
    }
    fn enter(data: ?*anyopaque, selected: A.Token, _: fx.WallInstant) A.Fence.Enter {
        const env: *const R = @ptrCast(@alignCast(data.?));
        if (selected.generation != env.generation) return .stale;
        return .{ .held = .{ .userdata = null, .until = .never, .release_fn = struct {
            fn close(_: ?*anyopaque) void {}
        }.close } };
    }
    fn fence(_: *const Input, env: *const R) A.Fence {
        return .{ .userdata = @constCast(env), .enter = enter };
    }
    fn now(_: *const R) fx.WallInstant {
        return .{ .unix_nanoseconds = 0 };
    }
    fn activationError(problem: A.Error) E {
        return switch (problem) {
            .stale => .stale,
            .invalid_token => .invalid,
            .unavailable => .unavailable,
        };
    }
    fn storageError(problem: Store.Failure) E {
        var owned = problem;
        defer owned.deinit();
        return .persistence;
    }
    fn projectionError(problem: Projection.Error) E {
        var owned = problem;
        defer owned.deinit();
        return .projection;
    }
    fn load(work: *const Work, env: *const R) fx.Effect(Loaded, E, R) {
        var reading = env.checkpoints.vtable.read(env.checkpoints.userdata, env, work.request.id);
        var mapped = reading.mapErrorOwned(env, storageError);
        return mapped.flatMapOwned(env, struct {
            fn decode(checkpoint: ?fx.ProjectionCheckpoint, current: *const R) fx.Effect(Loaded, E, R) {
                var owned = checkpoint;
                defer if (owned) |*value| value.deinit();
                if (owned) |value| {
                    if (value.state_schema_version != 1) return fx.Effect(Loaded, E, R).fail(.invalid);
                    const amount = std.fmt.parseInt(i64, value.encoded_state.items, 10) catch return fx.Effect(Loaded, E, R).fail(.invalid);
                    return fx.Effect(Loaded, E, R).succeed(.{ .state = amount, .cursor = value.cursor, .revision = .{ .present = value.cursor } });
                }
                _ = current;
                return fx.Effect(Loaded, E, R).succeed(.{ .state = 0, .cursor = .before_first, .revision = .absent });
            }
        }.decode);
    }
    fn replay(work: *const Work, loaded: *const Loaded, env: *const R) fx.Effect(fx.EventProjection.Replayed(i64), E, R) {
        var described = Projection.replayPage(env, &loaded.state, loaded.cursor, work.request.history);
        return described.mapErrorOwned(env, projectionError);
    }
    fn encode(replayed: *const fx.EventProjection.Replayed(i64), _: *const R) fx.Effect(fx.OwnedSlice(u8), E, R) {
        return fx.Effect(fx.OwnedSlice(u8), E, R).fromBorrowed(fx.EventProjection.Replayed(i64), replayed, struct {
            fn bytes(value: *const fx.EventProjection.Replayed(i64), current: *const R) fx.Exit(fx.OwnedSlice(u8), E) {
                const encoded = std.fmt.allocPrint(current.allocator, "{d}", .{value.state}) catch return .{ .die = fx.Defect.out_of_memory };
                return .{ .success = .{ .items = encoded, .allocator = current.allocator } };
            }
        }.bytes);
    }
    fn publish(work: *const Work, loaded: *const Loaded, replayed: *const fx.EventProjection.Replayed(i64), encoded: *const fx.OwnedSlice(u8), env: *const R) Number {
        if (work.resource.value.generation != env.generation) return Number.fail(.stale);
        if (replayed.cursor.sequence == loaded.cursor.sequence) return Number.succeed(replayed.state);
        var committing = env.checkpoints.vtable.compare_and_publish(env.checkpoints.userdata, env, work.request.id, loaded.revision, replayed.cursor, 1, encoded.items);
        var mapped = committing.mapErrorOwned(env, storageError);
        return mapped.flatMapCapturedOwned(env, i64, replayed.state, struct {
            fn committed(value: *const i64, result: fx.ProjectionCheckpointCommit, _: *const R) Number {
                return switch (result) {
                    .committed => Number.succeed(value.*),
                    .conflict => Number.fail(.conflict),
                    .possibly_committed => Number.fail(.persistence),
                };
            }
        }.committed);
    }
    fn rpcEnvironment(env: *const R) *const rpc.Root {
        return env.rpc.?;
    }
    fn rpcError(problem: anytype) E {
        var owned = problem;
        defer owned.deinit();
        return .rpc;
    }
    fn session(total: *const i64, env: *const R) Number {
        if (env.rpc == null) return Number.succeed(total.*);
        var description = rpc.describe(env.rpc.?, false);
        var projected = description.provideSomeAllocated(env.allocator, R, rpcEnvironment);
        var mapped = projected.mapErrorOwned(env, rpcError);
        return mapped.flatMapCapturedOwned(env, i64, total.*, struct {
            fn done(value: *const i64, _: u32, _: *const R) Number {
                return Number.succeed(value.*);
            }
        }.done);
    }
    fn use(work: *const Work, _: A.Token, env: *const R) Number {
        return p.captured(env, work.*, .{ p.binding("loaded", .{"inputs"}, load), p.binding("replayed", .{ "inputs", "loaded" }, replay), p.binding("encoded", .{"replayed"}, encode), p.binding("published", .{ "inputs", "loaded", "replayed", "encoded" }, publish), p.binding("served", .{"published"}, session) });
    }
    fn acquired(request: *const *const Input, resource: fx.ResourceValue(*Live), _: *const R) @TypeOf(A.use(Work{ .request = request.*, .resource = resource }, use)) {
        return A.use(Work{ .request = request.*, .resource = resource }, use);
    }
    fn start(input: *const Input, env: *const R) Number {
        return A.run(env, input.*, A.bindCaptured(A.acquire(select), input, acquired), .{ .token = token, .fence = fence, .now = now, .map_error = activationError });
    }
};
pub fn project(env: *const R, input: Input) Number {
    return fx.dsl.owned(env, input, Calls.start);
}
pub fn history(allocator: std.mem.Allocator, sequence: u64, amount: []const u8) !fx.EventReadPage {
    const entries = try allocator.alloc(fx.EventEntry, 1);
    errdefer allocator.free(entries);
    const tag = try allocator.dupe(u8, "credit");
    errdefer allocator.free(tag);
    const id = try allocator.dupe(u8, "credit-identity");
    errdefer allocator.free(id);
    entries[0] = .{ .tag = .{ .items = tag, .allocator = allocator }, .event_id = .{ .items = id, .allocator = allocator }, .encoded = .{ .items = try allocator.dupe(u8, amount), .allocator = allocator }, .schema_version = 1, .cursor = .{ .sequence = sequence } };
    return .{ .entries = .{ .items = entries, .allocator = allocator }, .next = .{ .sequence = sequence }, .more = false };
}
fn check(allocator: std.mem.Allocator, io: std.Io) !void {
    var acquired: usize = 0;
    var released: usize = 0;
    var env: R = .{ .allocator = allocator, .io = io, .acquired = &acquired, .released = &released };
    const resource = Resource.fromProvider(null, &.{ .acquire = Calls.acquire, .release = Calls.release });
    env.resource = &resource;
    var memory = fx.MemoryProjectionCheckpoint(R).init(&env);
    defer memory.deinit();
    const store = memory.capability();
    env.checkpoints = &store;
    var encoded_id = fx.PrimaryKey.encodeDirect(&env, "account", &.{.{ .text = "one" }});
    defer encoded_id.deinit();
    if (encoded_id != .success) return error.OutOfMemory;
    const account_id = encoded_id.success.items;
    var first = try history(allocator, 1, "7");
    defer first.deinit();
    var description = project(&env, .{ .id = account_id, .generation = 1, .history = &first });
    var result = fx.run(&env, description);
    description.deinit();
    defer result.deinit();
    try std.testing.expect(result == .success);
    try std.testing.expectEqual(@as(i64, 7), result.success);
    var malformed = try history(allocator, 2, "invalid-json");
    defer malformed.deinit();
    var broken = project(&env, .{ .id = account_id, .generation = 1, .history = &malformed });
    var failed = fx.run(&env, broken);
    broken.deinit();
    defer failed.deinit();
    try std.testing.expect(failed == .fail and failed.fail == .projection);
    var unchanged_read = store.vtable.read(store.userdata, &env, account_id);
    defer unchanged_read.deinit();
    var unchanged = fx.run(&env, unchanged_read);
    defer unchanged.deinit();
    try std.testing.expect(unchanged == .success and unchanged.success != null);
    try std.testing.expectEqual(@as(u64, 1), unchanged.success.?.cursor.sequence);
    try std.testing.expectEqualStrings("7", unchanged.success.?.encoded_state.items);
    env.generation = 2;
    var second = try history(allocator, 2, "5");
    defer second.deinit();
    var stale = project(&env, .{ .id = account_id, .generation = 1, .history = &second });
    var rejected = fx.run(&env, stale);
    stale.deinit();
    defer rejected.deinit();
    try std.testing.expect(rejected == .fail and rejected.fail == .stale);
    var fresh = project(&env, .{ .id = account_id, .generation = 2, .history = &second });
    var updated = fx.run(&env, fresh);
    fresh.deinit();
    defer updated.deinit();
    try std.testing.expect(updated == .success);
    try std.testing.expectEqual(@as(i64, 12), updated.success);
    try std.testing.expectEqual(@as(usize, 3), acquired);
    try std.testing.expectEqual(acquired, released);
    var reading = store.vtable.read(store.userdata, &env, account_id);
    defer reading.deinit();
    var checkpoint = fx.run(&env, reading);
    defer checkpoint.deinit();
    try std.testing.expect(checkpoint == .success and checkpoint.success != null);
    try std.testing.expectEqual(@as(u64, 2), checkpoint.success.?.cursor.sequence);
    try std.testing.expectEqualStrings("12", checkpoint.success.?.encoded_state.items);
}
pub fn main(init: std.process.Init) !void {
    try check(init.gpa, init.io);
}
test "EX-08 activation reacquisition typed event replay and atomic state cursor persistence" {
    try check(std.testing.allocator, std.testing.io);
}
