//! Typed service contracts for clocks, randomness, logs, traces, and console I/O.
const std = @import("std");
const outcomes = @import("outcome.zig");

/// Runtime-selected clock. Its borrowed `userdata` and `vtable` must remain
/// valid while effects using this service are running. `SleepResult` keeps
/// interruption separate from the successful sleep outcome.
pub const Clock = struct {
    userdata: ?*anyopaque,
    vtable: *const VTable,

    /// Terminal result of a clock sleep, including interruption and defect.
    pub const SleepResult = union(enum) { done, interrupted, defect: outcomes.Defect };
    /// Typed function-pointer interface for this capability. Implement every
    /// hook with the exact signature below; keep the table valid while the
    /// capability or pending work can invoke it.
    pub const VTable = struct {
        /// Read the current time.
        /// Parameters (callback order):
        /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
        /// Returns (`std.Io.Timestamp`): the specified typed result.
        now: *const fn (?*anyopaque) std.Io.Timestamp,
        /// Suspend for the requested duration.
        /// Parameters (callback order):
        /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
        /// - `2: std.Io.Duration`: requested delay or time interval.
        /// Returns (`SleepResult`): the specified typed result.
        sleep: *const fn (?*anyopaque, std.Io.Duration) SleepResult,
    };

    /// Read the current time from this clock capability.
    /// Parameters:
    /// - `self: Clock`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`std.Io.Timestamp`): The current timestamp from the selected clock.
    pub fn now(self: Clock) std.Io.Timestamp {
        return self.vtable.now(self.userdata);
    }
    /// Suspend for the requested duration through the selected clock.
    /// Parameters:
    /// - `self: Clock`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// - `delay: std.Io.Duration`: Requested duration before the next scheduled action.
    /// Returns (`SleepResult`): Sleep completion, interruption, or defect.
    pub fn sleep(self: Clock, delay: std.Io.Duration) SleepResult {
        return self.vtable.sleep(self.userdata, delay);
    }

    /// The pointed-to I/O value must outlive the clock. No process-global clock
    /// or allocator is installed by this adapter.
    ///
    /// Parameters:
    /// - `io: *const std.Io`: Caller-selected I/O implementation used for suspension and synchronization.
    /// Returns (`Clock`): A capability adapter referencing the selected implementation; keep its backing state alive while in use.
    pub fn fromIo(io: *const std.Io) Clock {
        return .{ .userdata = @ptrCast(@constCast(io)), .vtable = &.{ .now = nativeNow, .sleep = nativeSleep } };
    }

    fn nativeNow(data: ?*anyopaque) std.Io.Timestamp {
        const io: *const std.Io = @ptrCast(@alignCast(data.?));
        return .now(io.*, .awake);
    }

    fn nativeSleep(data: ?*anyopaque, delay: std.Io.Duration) SleepResult {
        const io: *const std.Io = @ptrCast(@alignCast(data.?));
        io.sleep(delay, .awake) catch return .interrupted;
        return .done;
    }
};

/// Runtime-selected random source. `sample_fn` receives borrowed `userdata`
/// and returns a sample as `f64`; the provider chooses its implementation.
pub const Random = struct {
    userdata: ?*anyopaque,
    /// Parameters: borrowed provider context. Returns a random `f64` sample.
    sample_fn: *const fn (?*anyopaque) f64,
    /// Draw one sample from the caller’s random provider.
    /// Parameters:
    /// - `self: Random`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`f64`): One sample from the selected random provider.
    pub fn sample(self: Random) f64 {
        return self.sample_fn(self.userdata);
    }
    /// Adapt the caller-supplied I/O implementation.
    /// Parameters:
    /// - `io: *const std.Io`: Caller-selected I/O implementation used for suspension and synchronization.
    /// Returns (`Random`): A capability adapter referencing the selected implementation; keep its backing state alive while in use.
    pub fn fromIo(io: *const std.Io) Random {
        return .{ .userdata = @ptrCast(@constCast(io)), .sample_fn = nativeSample };
    }
    fn nativeSample(data: ?*anyopaque) f64 {
        const io: *const std.Io = @ptrCast(@alignCast(data.?));
        var bits: u64 = undefined;
        io.random(std.mem.asBytes(&bits));
        return @as(f64, @floatFromInt(bits)) / @as(f64, @floatFromInt(std.math.maxInt(u64)));
    }
};

/// `LogLevel` cases: `debug`, `info`, `warning`, `err`.
pub const LogLevel = enum { debug, info, warning, err };
/// Borrowed diagnostic key and value passed to logger, tracer, and redactor hooks.
pub const Annotation = struct { key: []const u8, value: []const u8 };
/// Trace span identifier carried across log and tracing callbacks.
pub const SpanId = struct { value: u128 };

/// Consumer-supplied defect reporting hook. Messages passed to `report_fn`
/// are borrowed; the sink copies text it wants to retain.
pub const DiagnosticSink = struct {
    userdata: ?*anyopaque,
    // The consumer supplies retention capacity and overflow behavior. Payload
    // messages are borrowed for this callback; retained text must be copied.
    /// Parameters: borrowed provider context and typed `Defect`.
    /// Returns no value; retention and overflow are the provider's policy.
    report_fn: *const fn (?*anyopaque, outcomes.Defect) void,

    /// Forward a Defect to the diagnostic sink.
    /// Parameters:
    /// - `self: DiagnosticSink`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// - `defect: outcomes.Defect`: Unexpected failure represented separately from typed errors.
    /// Returns (`void`): no value.
    pub fn report(self: DiagnosticSink, defect: outcomes.Defect) void {
        self.report_fn(self.userdata, defect);
    }

    /// Report defects and cleanup failures from an Exit.
    /// Parameters:
    /// - `self: DiagnosticSink`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// - `exit: anytype`: Statically inferred input checked when this operation is instantiated.
    /// Returns (`void`): no value.
    pub fn reportExit(self: DiagnosticSink, exit: anytype) void {
        switch (exit.primary()) {
            .succeeded => {},
            .failed => |defect| self.report(defect),
            .defected => |defect| self.report(defect),
            .was_interrupted => {},
        }
        if (exit == .finalization_failed) {
            var iterator = exit.finalization_failed.iterator();
            while (iterator.nextFailure()) |failure| switch (failure) {
                .release_failure, .release_defect => |defect| self.report(defect),
                .release_interrupted => {},
            };
        }
    }
};

/// `Redactor` is a callback interface with `userdata: ?*anyopaque`, `redact_fn: *const fn (?*anyopaque,
/// Annotation) ?Annotation`.
pub const Redactor = struct {
    userdata: ?*anyopaque,
    redact_fn: *const fn (?*anyopaque, Annotation) ?Annotation,

    /// Pass-through redactor that leaves annotations unchanged.
    pub const identity: Redactor = .{ .userdata = null, .redact_fn = pass };
    fn pass(_: ?*anyopaque, annotation: Annotation) ?Annotation {
        return annotation;
    }
};

/// A provider iterates redacted annotations. This avoids allocating a second
/// annotation map merely to export it. Returned text is borrowed through export.
pub const Annotations = struct {
    values: []const Annotation,
    redactor: Redactor,

    /// Create an iterator over the retained entries.
    /// Parameters:
    /// - `self: Annotations`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`Iterator`): An iterator borrowing the collection it traverses.
    pub fn iterator(self: Annotations) Iterator {
        return .{ .annotations = self };
    }
    /// Borrowed cursor over annotations; keep its source alive while iterating.
    pub const Iterator = struct {
        annotations: Annotations,
        index: usize = 0,

        /// Advance to the next item or state.
        /// Parameters:
        /// - `self: *Iterator`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`?Annotation`): next borrowed annotation or `null` when the iterator is exhausted.
        pub fn next(self: *Iterator) ?Annotation {
            while (self.index < self.annotations.values.len) {
                const current = self.annotations.values[self.index];
                self.index += 1;
                if (self.annotations.redactor.redact_fn(self.annotations.redactor.userdata, current)) |redacted| return redacted;
            }
            return null;
        }
    };
};

/// Borrowed log payload containing severity, text, redacted annotations, and optional span id.
pub const LogRecord = struct { level: LogLevel, message: []const u8, annotations: Annotations, span: ?SpanId };

/// Provider-backed logger with level filtering, redaction, and defect
/// reporting. The provider context must outlive each `write_fn` call.
pub const Logger = struct {
    userdata: ?*anyopaque,
    minimum: LogLevel,
    redactor: Redactor,
    diagnostics: DiagnosticSink,
    /// Parameters: borrowed provider context and `LogRecord` whose message
    /// and annotation strings are borrowed for this call.
    /// Returns `Exit(void, Defect)`; `Logger.write` reports non-success to its
    /// diagnostic sink and releases any owned exit payload.
    write_fn: *const fn (?*anyopaque, LogRecord) outcomes.Exit(void, outcomes.Defect),

    /// Write through the selected capability.
    /// Parameters:
    /// - `self: Logger`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// - `level: LogLevel`: Diagnostic severity used for this log entry.
    /// - `message: []const u8`: Borrowed input; keep its owner alive until the operation has copied or joined
    ///   it.
    /// - `annotations: []const Annotation`: Metadata attached to the operation or diagnostic.
    /// - `span: ?SpanId`: Optional parent span identity for trace correlation.
    /// Returns (`void`): no value.
    pub fn write(self: Logger, level: LogLevel, message: []const u8, annotations: []const Annotation, span: ?SpanId) void {
        if (@backingInt(level) < @backingInt(self.minimum)) return;
        var result = self.write_fn(self.userdata, .{ .level = level, .message = message, .annotations = .{ .values = annotations, .redactor = self.redactor }, .span = span });
        defer result.deinit();
        self.diagnostics.reportExit(result);
    }
};

/// Primary span outcome: success, failure, defect, or interruption.
pub const SpanOutcome = enum { succeeded, failed, defected, interrupted };
/// Borrowed input to `Tracer.begin`: parent, span name, annotations, and optional timestamp.
pub const SpanStart = struct { parent: ?SpanId, name: []const u8, annotations: Annotations, time: ?std.Io.Timestamp };
/// Input to `Tracer.end`: span id, primary outcome, cleanup status, and optional timestamp.
pub const SpanFinish = struct { id: SpanId, outcome: SpanOutcome, cleanup_failed: bool, time: ?std.Io.Timestamp };

/// Provider-backed tracing lifecycle. Its vtable must implement begin, end,
/// and abandon so a failed export cannot orphan a native span.
pub const Tracer = struct {
    userdata: ?*anyopaque,
    vtable: *const VTable,
    redactor: Redactor,
    diagnostics: DiagnosticSink,

    /// Typed function-pointer interface for this capability. Implement every
    /// hook with the exact signature below; keep the table valid while the
    /// capability or pending work can invoke it.
    pub const VTable = struct {
        /// Start a span.
        /// Parameters (callback order):
        /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
        /// - `2: SpanStart`: typed tracing lifecycle data.
        /// Returns (`outcomes.Exit(SpanId, outcomes.Defect)`): typed terminal Exit, including defect and interruption outcomes.
        begin: *const fn (?*anyopaque, SpanStart) outcomes.Exit(SpanId, outcomes.Defect),
        /// Finish a span.
        /// Parameters (callback order):
        /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
        /// - `2: SpanFinish`: typed tracing lifecycle data.
        /// Returns (`outcomes.Exit(void, outcomes.Defect)`): typed terminal Exit, including defect and interruption outcomes.
        end: *const fn (?*anyopaque, SpanFinish) outcomes.Exit(void, outcomes.Defect),
        // Required lifecycle hook: a failed export cannot orphan a native span.
        /// Release a span after failed export.
        /// Parameters (callback order):
        /// - `1: ?*anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
        /// - `2: SpanId`: typed tracing lifecycle data.
        /// Returns (`outcomes.Exit(void, outcomes.Defect)`): typed terminal Exit, including defect and interruption outcomes.
        abandon: *const fn (?*anyopaque, SpanId) outcomes.Exit(void, outcomes.Defect),
    };

    /// Start a statically checked builder sequence.
    /// Parameters:
    /// - `self: Tracer`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// - `parent: ?SpanId`: Optional parent span identity or scope.
    /// - `name: []const u8`: Stable, nonempty identity component validated at definition time.
    /// - `annotations: []const Annotation`: Metadata attached to the operation or diagnostic.
    /// - `time: ?std.Io.Timestamp`: Caller-supplied clock instant.
    /// Returns (`?SpanId`): new span handle, or `null` if the tracer declines to start one.
    pub fn begin(self: Tracer, parent: ?SpanId, name: []const u8, annotations: []const Annotation, time: ?std.Io.Timestamp) ?SpanId {
        var result = self.vtable.begin(self.userdata, .{ .parent = parent, .name = name, .annotations = .{ .values = annotations, .redactor = self.redactor }, .time = time });
        defer result.deinit();
        self.diagnostics.reportExit(result);
        return if (result == .success) result.success else null;
    }

    /// Finish the current operation and release its active state.
    /// Parameters:
    /// - `self: Tracer`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// - `id: SpanId`: Stable typed identity for this operation.
    /// - `exit: anytype`: Statically inferred input checked when this operation is instantiated.
    /// - `time: ?std.Io.Timestamp`: Caller-supplied clock instant.
    /// Returns (`void`): no value.
    pub fn end(self: Tracer, id: SpanId, exit: anytype, time: ?std.Io.Timestamp) void {
        const state: SpanOutcome = switch (exit.primary()) {
            .succeeded => .succeeded,
            .failed => .failed,
            .defected => .defected,
            .was_interrupted => .interrupted,
        };
        var result = self.vtable.end(self.userdata, .{ .id = id, .outcome = state, .cleanup_failed = exit == .finalization_failed, .time = time });
        defer result.deinit();
        self.diagnostics.reportExit(result);
        if (result != .success) {
            var abandoned = self.vtable.abandon(self.userdata, id);
            defer abandoned.deinit();
            self.diagnostics.reportExit(abandoned);
        }
    }
};

/// Caller-supplied console capability. Both hooks take borrowed message bytes
/// and return typed `Exit` values rather than throwing Zig errors.
pub const Console = struct {
    userdata: *anyopaque,
    /// Parameters: borrowed provider context and borrowed output bytes.
    /// Returns the typed terminal outcome for normal output.
    write_fn: *const fn (*anyopaque, []const u8) outcomes.Exit(void, outcomes.Defect),
    /// Parameters: borrowed provider context and borrowed error-output bytes.
    /// Returns the typed terminal outcome for error output.
    error_fn: *const fn (*anyopaque, []const u8) outcomes.Exit(void, outcomes.Defect),
    /// Write through the selected capability.
    /// Parameters:
    /// - `self: Console`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// - `message: []const u8`: Borrowed input; keep its owner alive until the operation has copied or joined
    ///   it.
    /// Returns (`outcomes.Exit(void, outcomes.Defect)`): Carries no value on `.success` or `outcomes.Defect` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
    pub fn write(self: Console, message: []const u8) outcomes.Exit(void, outcomes.Defect) {
        return self.write_fn(self.userdata, message);
    }
    /// Write a message through the error-output channel.
    /// Parameters:
    /// - `self: Console`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// - `message: []const u8`: Borrowed input; keep its owner alive until the operation has copied or joined
    ///   it.
    /// Returns (`outcomes.Exit(void, outcomes.Defect)`): Carries no value on `.success` or `outcomes.Defect` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
    pub fn err(self: Console, message: []const u8) outcomes.Exit(void, outcomes.Defect) {
        return self.error_fn(self.userdata, message);
    }
};
/// Console capability backed by caller-owned output and error writers.
pub const NativeConsole = struct {
    output: *std.Io.Writer,
    errors: *std.Io.Writer,
    /// Expose this implementation through its service interface.
    /// Parameters:
    /// - `self: *NativeConsole`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`Console`): A capability adapter referencing the selected implementation; keep its backing state alive while in use.
    pub fn service(self: *NativeConsole) Console {
        return .{ .userdata = self, .write_fn = write, .error_fn = err };
    }
    fn line(writer: *std.Io.Writer, message: []const u8) outcomes.Exit(void, outcomes.Defect) {
        writer.writeAll(message) catch |error_value| return .{ .fail = .{ .kind = .native_failure, .message = @errorName(error_value) } };
        writer.writeByte('\n') catch |error_value| return .{ .fail = .{ .kind = .native_failure, .message = @errorName(error_value) } };
        writer.flush() catch |error_value| return .{ .fail = .{ .kind = .native_failure, .message = @errorName(error_value) } };
        return .{ .success = {} };
    }
    fn write(data: *anyopaque, message: []const u8) outcomes.Exit(void, outcomes.Defect) {
        const self: *NativeConsole = @ptrCast(@alignCast(data));
        return line(self.output, message);
    }
    fn err(data: *anyopaque, message: []const u8) outcomes.Exit(void, outcomes.Defect) {
        const self: *NativeConsole = @ptrCast(@alignCast(data));
        return line(self.errors, message);
    }
};

/// Snapshot lexical labels only at an escaping ownership boundary. The strings,
/// as well as the array, must survive their originating environment frame.
///
/// Parameters:
/// - `allocator: std.mem.Allocator`: Caller-owned allocator; allocations and retained values stay in this allocation
///   domain.
/// - `values: []const Annotation`: Input collection; its elements are processed in the documented order.
/// Returns (`error{OutOfMemory}![]Annotation`): A `[]Annotation` on success; `OutOfMemory` if required allocation fails.
pub fn cloneAnnotations(allocator: std.mem.Allocator, values: []const Annotation) error{OutOfMemory}![]Annotation {
    const copied = try allocator.alloc(Annotation, values.len);
    errdefer allocator.free(copied);
    var completed: usize = 0;
    errdefer for (copied[0..completed]) |annotation| {
        allocator.free(annotation.key);
        allocator.free(annotation.value);
    };
    for (values, copied) |value, *output| {
        const key = try allocator.dupe(u8, value.key);
        const text = allocator.dupe(u8, value.value) catch |error_value| {
            allocator.free(key);
            return error_value;
        };
        output.* = .{ .key = key, .value = text };
        completed += 1;
    }
    return copied;
}
/// Release copied annotation storage.
/// Parameters:
/// - `allocator: std.mem.Allocator`: Caller-owned allocator; allocations and retained values stay in this allocation
///   domain.
/// - `values: []Annotation`: Input collection; its elements are processed in the documented order.
/// Returns (`void`): no value.
pub fn freeAnnotations(allocator: std.mem.Allocator, values: []Annotation) void {
    for (values) |annotation| {
        allocator.free(annotation.key);
        allocator.free(annotation.value);
    }
    allocator.free(values);
}
