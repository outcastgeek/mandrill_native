//! Neutral metric descriptions and recording. Names, units and observation
//! kinds are definition data; exporters and backend configuration stay outside
//! the core package.
const std = @import("std");
const outcomes = @import("outcome.zig");
const effects = @import("effect.zig");
const time = @import("deadline.zig");

pub const Kind = enum { counter, gauge, latency, queue_depth, retry, commit };
pub const Measurement = union(Kind) {
    counter: u64,
    gauge: i64,
    latency: time.Duration,
    queue_depth: usize,
    retry: u64,
    commit: u64,
};

pub fn Error(comptime ProviderError: type) type {
    return union(enum) {
        const Self = @This();
        pub const effect_owned_payload = outcomes.ownsPayload(ProviderError);
        provider: ProviderError,
        invalid_definition,
        kind_conflict,
        overflow,
        unavailable,

        pub fn deinit(self: *Self) void {
            if (self.* == .provider) outcomes.discardPayload(&self.provider);
            self.* = undefined;
        }

        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .provider => |value| .{ .provider = try outcomes.clonePayload(value) },
                else => self.*,
            };
        }
    };
}

pub fn Recorder(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        pub const Failure = Error(ProviderError);
        pub const RecordEffect = effects.Effect(void, Failure, R);
        userdata: ?*anyopaque,
        vtable: *const VTable,

        pub const VTable = struct {
            record: *const fn (?*anyopaque, *const R, []const u8, Measurement) RecordEffect,
        };

        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }

        pub fn record(comptime name: []const u8, comptime select_recorder: fn (*const R) *const Self, comptime select_measurement: fn (*const R) Measurement) RecordEffect {
            if (name.len == 0 or !std.unicode.utf8ValidateSlice(name)) @compileError("metric names must be nonempty UTF-8");
            return RecordEffect.deferEffect(struct {
                fn start(env: *const R) RecordEffect {
                    const recorder = select_recorder(env);
                    return recorder.vtable.record(recorder.userdata, env, name, select_measurement(env));
                }
            }.start);
        }
    };
}

/// Comptime-paired descriptor: a latency observation cannot be passed to a
/// counter, and no runtime tag comparison is required in the common path.
pub fn Definition(comptime name: []const u8, comptime kind: Kind) type {
    if (name.len == 0 or !std.unicode.utf8ValidateSlice(name)) @compileError("metric names must be nonempty UTF-8");
    return struct {
        pub const stable_name = name;
        pub const measurement_kind = kind;
        pub const Value = switch (kind) {
            .counter, .retry, .commit => u64,
            .gauge => i64,
            .latency => time.Duration,
            .queue_depth => usize,
        };

        pub fn measure(
            comptime R: type,
            comptime ProviderError: type,
            comptime select_recorder: fn (*const R) *const Recorder(R, ProviderError),
            comptime select_value: fn (*const R) Value,
        ) Recorder(R, ProviderError).RecordEffect {
            return Recorder(R, ProviderError).record(name, select_recorder, struct {
                fn project(env: *const R) Measurement {
                    return @unionInit(Measurement, @tagName(kind), select_value(env));
                }
            }.project);
        }
    };
}

pub const LatencyAggregate = struct { observations: u64, total_nanoseconds: u128 };
pub const Snapshot = union(Kind) {
    counter: u64,
    gauge: i64,
    latency: LatencyAggregate,
    queue_depth: usize,
    retry: u64,
    commit: u64,
};

/// Volatile aggregation for local observability and provider conformance.
/// Recorder operations are atomic per name; a mismatched kind is an error.
pub fn MemoryMetrics(comptime R: type) type {
    return struct {
        const Self = @This();
        const Provider = Recorder(R, void);
        const Failure = Error(void);
        const Entry = struct { name: []u8, value: Snapshot };
        const Capture = struct { owner: *Self, name: []const u8, measurement: Measurement };

        allocator: std.mem.Allocator,
        io: std.Io,
        mutex: std.Io.Mutex = .init,
        values: std.StringHashMap(Entry),

        pub fn init(env: *const R) Self {
            if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
                @compileError("MemoryMetrics requires caller allocator in R");
            if (R == void or !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
                @compileError("MemoryMetrics requires caller io in R");
            return .{ .allocator = env.allocator, .io = env.io, .values = std.StringHashMap(Entry).init(env.allocator) };
        }

        pub fn capability(self: *Self) Provider {
            return Provider.fromProvider(self, &.{ .record = recordEffect });
        }

        fn recordEffect(data: ?*anyopaque, env: *const R, name: []const u8, measurement: Measurement) Provider.RecordEffect {
            const owner: *Self = @ptrCast(@alignCast(data.?));
            return Provider.RecordEffect.fromOwned(env, Capture{ .owner = owner, .name = name, .measurement = measurement }, struct {
                fn run(capture: *const Capture, _: *const R) outcomes.Exit(void, Failure) {
                    return capture.owner.recordNow(capture.name, capture.measurement);
                }
            }.run);
        }

        fn initial(measurement: Measurement) Snapshot {
            return switch (measurement) {
                .counter => |value| .{ .counter = value },
                .gauge => |value| .{ .gauge = value },
                .latency => |value| .{ .latency = .{ .observations = 1, .total_nanoseconds = value.nanoseconds } },
                .queue_depth => |value| .{ .queue_depth = value },
                .retry => |value| .{ .retry = value },
                .commit => |value| .{ .commit = value },
            };
        }

        fn updated(current: Snapshot, measurement: Measurement) outcomes.Result(Snapshot, Failure) {
            if (std.meta.activeTag(current) != std.meta.activeTag(measurement)) return .{ .err = .kind_conflict };
            return switch (measurement) {
                .counter => |value| .{ .ok = .{ .counter = std.math.add(u64, current.counter, value) catch return .{ .err = .overflow } } },
                .gauge => |value| .{ .ok = .{ .gauge = value } },
                .latency => |value| .{ .ok = .{ .latency = .{
                    .observations = std.math.add(u64, current.latency.observations, 1) catch return .{ .err = .overflow },
                    .total_nanoseconds = std.math.add(u128, current.latency.total_nanoseconds, value.nanoseconds) catch return .{ .err = .overflow },
                } } },
                .queue_depth => |value| .{ .ok = .{ .queue_depth = value } },
                .retry => |value| .{ .ok = .{ .retry = std.math.add(u64, current.retry, value) catch return .{ .err = .overflow } } },
                .commit => |value| .{ .ok = .{ .commit = std.math.add(u64, current.commit, value) catch return .{ .err = .overflow } } },
            };
        }

        fn recordNow(self: *Self, name: []const u8, measurement: Measurement) outcomes.Exit(void, Failure) {
            if (name.len == 0 or !std.unicode.utf8ValidateSlice(name)) return .{ .fail = .invalid_definition };
            const owned_name = self.allocator.dupe(u8, name) catch return .{ .die = outcomes.Defect.out_of_memory };
            var retained = false;
            defer if (!retained) self.allocator.free(owned_name);
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.values.getPtr(name)) |entry| {
                const next = updated(entry.value, measurement);
                return switch (next) {
                    .ok => |value| blk: {
                        entry.value = value;
                        break :blk .{ .success = {} };
                    },
                    .err => |problem| .{ .fail = problem },
                };
            }
            self.values.ensureUnusedCapacity(1) catch return .{ .die = outcomes.Defect.out_of_memory };
            self.values.putAssumeCapacity(owned_name, .{ .name = owned_name, .value = initial(measurement) });
            retained = true;
            return .{ .success = {} };
        }

        pub fn snapshot(self: *Self, name: []const u8) ?Snapshot {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            return if (self.values.get(name)) |entry| entry.value else null;
        }

        pub fn deinit(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            var iterator = self.values.iterator();
            while (iterator.next()) |entry| self.allocator.free(entry.value_ptr.name);
            self.values.deinit();
            self.mutex.unlock(self.io);
            self.* = undefined;
        }
    };
}
