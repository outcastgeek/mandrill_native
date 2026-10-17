const std = @import("std");
const fx = @import("effect");
const gates = @import("support/dsl_gate.zig");
const Threads = @import("support/thread_executor.zig").ThreadExecutor;
const Issue = fx.SchemaIssue.Issue;
const Gate = gates.Gate(i64, Issue);
const Counts = struct { live: usize = 0, defaults: usize = 0, decoded: usize = 0, released: usize = 0 };
const Env = struct { allocator: std.mem.Allocator, io: std.Io = std.testing.io, async_engine: fx.Executor, counts: *Counts, gate: *Gate };

// A provider-native representation owns scalar tokens and encoded object
// storage. The input empty object has no token or allocation to release.
const Wire = struct {
    pub const effect_owned_payload = true;
    kind: enum { empty_object, scalar, object },
    number: i64 = 0,
    counts: ?*Counts = null,
    child: ?*Wire = null,
    allocator: ?std.mem.Allocator = null,
    pub fn deinit(self: *@This()) void {
        if (self.kind == .object) {
            self.child.?.deinit();
            self.allocator.?.destroy(self.child.?);
        }
        if (self.counts) |counts| {
            std.debug.assert(counts.live > 0);
            counts.live -= 1;
            counts.released += 1;
        }
        self.* = undefined;
    }
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        var result = self.*;
        if (self.kind == .object) {
            const child = try self.allocator.?.create(Wire);
            errdefer self.allocator.?.destroy(child);
            child.* = try self.child.?.clone();
            result.child = child;
        }
        if (self.counts) |counts| counts.live += 1;
        return result;
    }
};
const Object = struct {
    pub const Representation = Wire;
    pub const Builder = struct { value: ?Wire = null };
    pub fn count(input: *const Wire) usize {
        return if (input.kind == .object) 1 else 0;
    }
    pub fn nameAt(_: *const Wire, index: usize) []const u8 {
        std.debug.assert(index == 0);
        return "answer";
    }
    pub fn find(input: *const Wire, name: []const u8) ?*const Wire {
        return if (input.kind == .object and std.mem.eql(u8, name, "answer")) input.child else null;
    }
    pub fn init(_: *const Env, _: usize) fx.Exit(Builder, Issue) {
        return .{ .success = .{} };
    }
    pub fn put(env: *const Env, builder: *Builder, name: []const u8, value: *const Wire) fx.Exit(void, Issue) {
        if (!std.mem.eql(u8, name, "answer") or builder.value != null) return .{ .fail = Issue.init(env, .encode, .invalid_value) };
        builder.value = value.clone() catch return .{ .die = fx.Defect.out_of_memory };
        return .{ .success = {} };
    }
    pub fn finish(env: *const Env, builder: *Builder) fx.Exit(Wire, Issue) {
        const child = env.allocator.create(Wire) catch return .{ .die = fx.Defect.out_of_memory };
        child.* = builder.value.?;
        builder.value = null;
        return .{ .success = .{ .kind = .object, .child = child, .allocator = env.allocator } };
    }
    pub fn discard(builder: *Builder) void {
        if (builder.value) |*value| value.deinit();
        builder.value = null;
    }
};
const Roles = struct {
    fn default(env: *const Env) Wire {
        env.counts.defaults += 1;
        env.counts.live += 1;
        return .{ .kind = .scalar, .number = 41, .counts = env.counts };
    }
    fn decode(input: *const Wire) fx.Effect(i64, Issue, Env) {
        return fx.Effect(i64, Issue, Env).deferBorrowed(Wire, input, struct {
            fn run(value: *const Wire, env: *const Env) fx.Effect(i64, Issue, Env) {
                std.debug.assert(value.kind == .scalar and value.counts.?.live > 0);
                env.counts.decoded += 1;
                if (env.gate.result == .success) env.gate.result.success = value.number + 1;
                return fx.Effect(i64, Issue, Env).fromAsync(struct {
                    fn start(current: *const Env) fx.Async(i64, Issue).Start {
                        return current.gate.start();
                    }
                }.start);
            }
        }.run);
    }
    fn encode(input: *const i64) fx.Effect(Wire, Issue, Env) {
        return fx.Effect(Wire, Issue, Env).fromBorrowed(i64, input, struct {
            fn run(value: *const i64, env: *const Env) fx.Exit(Wire, Issue) {
                env.counts.live += 1;
                return .{ .success = .{ .kind = .scalar, .number = value.*, .counts = env.counts } };
            }
        }.run);
    }
};
const codec = fx.dsl.codec(.{ .kind = .contextual, .decode = Roles.decode, .encode = Roles.encode });
const Record = fx.dsl.nativeRecord(Object, .{ .fields = .{
    .answer = .{ .codec = codec, .presence = .defaulted, .default = Roles.default },
}, .reject_unknown = true });

test "CODEC-02 CODEC-06 native owned default representation survives suspension and releases after success failure defect interruption or cancellation" {
    inline for (.{ .success, .fail, .die, .interrupted, .cancel }) |mode| {
        var threads: Threads = .{ .allocator = std.testing.allocator, .capacity = 1, .capabilities = .{ .independent = false, .cpu = false, .blocking = false } };
        defer threads.deinit();
        var counts: Counts = .{};
        var gate: Gate = .{ .result = switch (mode) {
            .success, .cancel => .{ .success = 0 },
            .fail => .{ .fail = Issue.pure(.decode, .invalid_value) },
            .die => .{ .die = fx.Defect.init("native wire failure") },
            .interrupted => .interrupted,
            else => unreachable,
        } };
        const env: Env = .{ .allocator = std.testing.allocator, .async_engine = threads.executor(), .counts = &counts, .gate = &gate };
        const input: Wire = .{ .kind = .empty_object };
        var described = Record.decode(&input);
        var pending = fx.runAsync(&env, described);
        described.deinit();
        gate.started.waitUncancelable(env.io);
        try std.testing.expectEqual(@as(usize, 1), counts.defaults);
        try std.testing.expectEqual(@as(usize, 1), counts.decoded);
        try std.testing.expectEqual(@as(usize, 1), counts.live);
        if (mode == .cancel) pending.request() else gate.proceed.set(env.io);
        var result = pending.join();
        pending.deinit();
        defer result.deinit();
        try std.testing.expect(gate.joined.load(.acquire) and gate.destroyed.load(.acquire));
        try std.testing.expectEqual(@as(usize, 0), counts.live);
        try std.testing.expectEqual(@as(usize, 1), counts.released);
        switch (mode) {
            .success => try std.testing.expect(result == .success and result.success.answer == 42),
            .fail => {
                try std.testing.expect(result == .fail);
                try std.testing.expectEqualStrings("answer", result.fail.path.?[0].field.items);
            },
            .die => try std.testing.expect(result == .die),
            .interrupted, .cancel => try std.testing.expect(result == .interrupted),
            else => unreachable,
        }
    }
}

test "CODEC-02 LIFE-11 native encoded representation owns its copied fields after record execution frames are gone" {
    var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = true }, .maximum_owned = 2 };
    defer engine.deinit();
    var counts: Counts = .{};
    var gate: Gate = .{ .result = .{ .success = 0 } };
    const env: Env = .{ .allocator = std.testing.allocator, .async_engine = engine.executor(), .counts = &counts, .gate = &gate };
    const value: Record.Decoded = .{ .answer = 42 };
    var described = Record.encode(&value);
    var result = fx.run(&env, described);
    described.deinit();
    try std.testing.expect(result == .success and result.success.kind == .object);
    try std.testing.expectEqual(@as(i64, 42), result.success.child.?.number);
    try std.testing.expectEqual(@as(usize, 1), counts.live);
    try std.testing.expectEqual(@as(usize, 1), counts.released);
    result.deinit();
    try std.testing.expectEqual(@as(usize, 0), counts.live);
    try std.testing.expectEqual(@as(usize, 2), counts.released);
}

test "CODEC-02 CODEC-06 LIFE-07 native owning default and encoded object roll back at every execution allocation failure" {
    const Check = struct {
        fn run(allocator: std.mem.Allocator) !void {
            var engine: fx.NativeIoExecutor = .{ .allocator = std.testing.allocator, .io = std.testing.io, .capabilities = .{ .independent = true, .cpu = true, .blocking = true }, .maximum_owned = 2 };
            defer engine.deinit();
            var counts: Counts = .{};
            defer std.debug.assert(counts.live == 0);
            var gate: Gate = .{ .result = .{ .success = 0 } };
            gate.proceed.set(std.testing.io);
            const env: Env = .{ .allocator = allocator, .async_engine = engine.executor(), .counts = &counts, .gate = &gate };
            const input: Wire = .{ .kind = .empty_object };
            var decoded = fx.run(&env, Record.decode(&input));
            defer decoded.deinit();
            if (decoded == .die and decoded.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expect(decoded == .success and decoded.success.answer == 42);
            try std.testing.expectEqual(@as(usize, 0), counts.live);
            var encoded = fx.run(&env, Record.encode(&decoded.success));
            defer encoded.deinit();
            if (encoded == .die and encoded.die.kind == .out_of_memory) return error.OutOfMemory;
            try std.testing.expect(encoded == .success and encoded.success.child.?.number == 42);
        }
    };
    try std.testing.checkAllAllocationFailures(std.testing.allocator, Check.run, .{});
}
