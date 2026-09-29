//! Neutral metric descriptions and recording. Names, units and observation
//! kinds are definition data; exporters and backend configuration stay outside
//! the core package.
const std = @import("std");
const outcomes = @import("outcome.zig");
const effects = @import("effect.zig");
const time = @import("deadline.zig");

/// `Kind` cases: `counter`, `gauge`, `latency`, `queue_depth`, `retry`, `commit`.
pub const Kind = enum { counter, gauge, latency, queue_depth, retry, commit };
/// One counter, gauge, or latency observation; the tag selects the numeric payload.
pub const Measurement = union(Kind) {
    counter: u64,
    gauge: i64,
    latency: time.Duration,
    queue_depth: usize,
    retry: u64,
    commit: u64,
};

/// Specialize the typed metric error channel from its compile-time error type.
/// Parameters:
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// Returns (`type`): tagged metric failure union retaining the selected provider error type.
pub fn Error(comptime ProviderError: type) type {
    return union(enum) {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = outcomes.ownsPayload(ProviderError);
        provider: ProviderError,
        invalid_definition,
        kind_conflict,
        overflow,
        unavailable,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            if (self.* == .provider) outcomes.discardPayload(&self.provider);
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            return switch (self.*) {
                .provider => |value| .{ .provider = try outcomes.clonePayload(value) },
                else => self.*,
            };
        }
    };
}

/// Specialize the caller-supplied metric recorder interface and its error channel.
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// - `ProviderError: comptime type`: Failure type reported by the selected provider and retained in the typed error channel.
/// Returns (`type`): provider interface for typed metric recording.
pub fn Recorder(comptime R: type, comptime ProviderError: type) type {
    return struct {
        const Self = @This();
        /// Typed failure channel combining domain and provider errors.
        pub const Failure = Error(ProviderError);
        /// Lazy Effect for record effect; success, error, and environment types are fixed by
        /// `effects.Effect(void, Failure, R)`.
        pub const RecordEffect = effects.Effect(void, Failure, R);
        userdata: ?*anyopaque,
        vtable: *const VTable,

        /// Typed function-pointer interface for this capability. Implement every
        /// hook with the exact signature below; keep the table valid while the
        /// capability or pending work can invoke it.
        pub const VTable = struct {
            /// Record the supplied measurement.
            /// Parameters (callback order):
            /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
            /// - `2: *const R`: borrowed generic environment `R`, including caller-supplied runtime services.
            /// - `3: []const u8`: borrowed input slice; keep it valid through deferred use or copy before retaining it.
            /// - `4: Measurement`: typed domain value consumed by this hook.
            /// Returns (`RecordEffect`): lazy typed Effect; run it with its declared environment to obtain Exit.
            record: *const fn (?*anyopaque, *const R, []const u8, Measurement) RecordEffect,
        };

        /// Wrap a caller-supplied provider implementation.
        /// Parameters:
        /// - `data: ?*anyopaque`: Borrowed provider context passed unchanged to every hook.
        ///   Its owner keeps it alive through any pending effect or operation.
        /// - `vtable: *const VTable`: Borrowed table of exactly typed callbacks; keep it
        ///   alive for every use of this capability, including deferred execution.
        /// Returns (`Self`): A non-owning adapter referencing `data` and `vtable`; no
        ///   provider state is allocated or transferred by this constructor.
        pub fn fromProvider(data: ?*anyopaque, vtable: *const VTable) Self {
            return .{ .userdata = data, .vtable = vtable };
        }

        /// Record the selected metric or durable result.
        /// Parameters:
        /// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition
        ///   time.
        /// - `select_recorder: comptime fn (*const R) *const Self`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_measurement: comptime fn (*const R) Measurement`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// Returns (`RecordEffect`): unit-valued recording operation sent to the selected metric provider.
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
///
/// Parameters:
/// - `name: comptime []const u8`: Compile-time stable, nonempty identity component validated at definition time.
/// - `kind: comptime Kind`: Typed operation or execution kind selecting behavior.
/// Returns (`type`): metric descriptor whose observation type matches its declared metric kind.
pub fn Definition(comptime name: []const u8, comptime kind: Kind) type {
    if (name.len == 0 or !std.unicode.utf8ValidateSlice(name)) @compileError("metric names must be nonempty UTF-8");
    return struct {
        /// Stable metric name selected at definition time.
        pub const stable_name = name;
        /// Metric measurement kind selected at definition time.
        pub const measurement_kind = kind;
        /// Value type determined by the metric kind or selected codec.
        pub const Value = switch (kind) {
            .counter, .retry, .commit => u64,
            .gauge => i64,
            .latency => time.Duration,
            .queue_depth => usize,
        };

        /// Measure an Effect and record its result.
        /// Parameters:
        /// - `R: comptime type`: Caller-defined environment type supplying the dependencies needed by this operation.
        /// - `ProviderError: comptime type`: Provider failure type retained in the typed result.
        /// - `select_recorder: comptime fn (*const R) *const Recorder(R, ProviderError)`: Compile-time typed selector from the current environment; its
        ///   result must remain valid through use.
        /// - `select_value: comptime fn (*const R) Value`: Compile-time typed selector from the current environment; its result
        ///   must remain valid through use.
        /// Returns (`Recorder(R, ProviderError).RecordEffect`): original Effect result after a matching metric observation is recorded.
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

/// Accumulated latency count and duration statistics for a metric identity.
pub const LatencyAggregate = struct { observations: u64, total_nanoseconds: u128 };
/// Current aggregated counter, gauge, or latency value for inspection.
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
///
/// Parameters:
/// - `R: comptime type`: Caller-defined environment type that supplies dependencies and runtime capabilities when work executes.
/// Returns (`type`): in-memory metric aggregation type for local observation.
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

        /// Create an in-memory metric recorder using the caller environment.
        /// Parameters:
        /// - `env: *const R`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// Returns (`Self`): a recorder using the caller-selected allocation and synchronization capabilities.
        pub fn init(env: *const R) Self {
            if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
                @compileError("MemoryMetrics requires caller allocator in R");
            if (R == void or !@hasField(R, "io") or @FieldType(R, "io") != std.Io)
                @compileError("MemoryMetrics requires caller io in R");
            return .{ .allocator = env.allocator, .io = env.io, .values = std.StringHashMap(Entry).init(env.allocator) };
        }

        /// Expose this implementation through its capability interface.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`Provider`): A capability adapter referencing the selected implementation; keep its backing state alive while in use.
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

        /// Capture the current observable state.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// - `name: []const u8`: Stable, nonempty identity component validated at definition time.
        /// Returns (`?Snapshot`): current metric snapshot when present; `null` if this identity has no recorded value.
        pub fn snapshot(self: *Self, name: []const u8) ?Snapshot {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            return if (self.values.get(name)) |entry| entry.value else null;
        }

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
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
