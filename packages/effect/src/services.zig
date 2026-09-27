const std = @import("std");
const outcomes = @import("outcome.zig");

pub const Clock = struct {
    userdata: ?*anyopaque,
    vtable: *const VTable,

    pub const SleepResult = union(enum) { done, interrupted, defect: outcomes.Defect };
    pub const VTable = struct {
        now: *const fn (?*anyopaque) std.Io.Timestamp,
        sleep: *const fn (?*anyopaque, std.Io.Duration) SleepResult,
    };

    pub fn now(self: Clock) std.Io.Timestamp {
        return self.vtable.now(self.userdata);
    }
    pub fn sleep(self: Clock, delay: std.Io.Duration) SleepResult {
        return self.vtable.sleep(self.userdata, delay);
    }

    /// The pointed-to I/O value must outlive the clock. No process-global clock
    /// or allocator is installed by this adapter.
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

pub const Random = struct {
    userdata: ?*anyopaque,
    sample_fn: *const fn (?*anyopaque) f64,
    pub fn sample(self: Random) f64 {
        return self.sample_fn(self.userdata);
    }
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

pub const LogLevel = enum { debug, info, warning, err };
pub const Annotation = struct { key: []const u8, value: []const u8 };
pub const SpanId = struct { value: u128 };

pub const DiagnosticSink = struct {
    userdata: ?*anyopaque,
    // The consumer supplies retention capacity and overflow behavior. Payload
    // messages are borrowed for this callback; retained text must be copied.
    report_fn: *const fn (?*anyopaque, outcomes.Defect) void,

    pub fn report(self: DiagnosticSink, defect: outcomes.Defect) void {
        self.report_fn(self.userdata, defect);
    }

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

pub const Redactor = struct {
    userdata: ?*anyopaque,
    redact_fn: *const fn (?*anyopaque, Annotation) ?Annotation,

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

    pub fn iterator(self: Annotations) Iterator {
        return .{ .annotations = self };
    }
    pub const Iterator = struct {
        annotations: Annotations,
        index: usize = 0,

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

pub const LogRecord = struct { level: LogLevel, message: []const u8, annotations: Annotations, span: ?SpanId };

pub const Logger = struct {
    userdata: ?*anyopaque,
    minimum: LogLevel,
    redactor: Redactor,
    diagnostics: DiagnosticSink,
    write_fn: *const fn (?*anyopaque, LogRecord) outcomes.Exit(void, outcomes.Defect),

    pub fn write(self: Logger, level: LogLevel, message: []const u8, annotations: []const Annotation, span: ?SpanId) void {
        if (@intFromEnum(level) < @intFromEnum(self.minimum)) return;
        var result = self.write_fn(self.userdata, .{ .level = level, .message = message, .annotations = .{ .values = annotations, .redactor = self.redactor }, .span = span });
        defer result.deinit();
        self.diagnostics.reportExit(result);
    }
};

pub const SpanOutcome = enum { succeeded, failed, defected, interrupted };
pub const SpanStart = struct { parent: ?SpanId, name: []const u8, annotations: Annotations, time: ?std.Io.Timestamp };
pub const SpanFinish = struct { id: SpanId, outcome: SpanOutcome, cleanup_failed: bool, time: ?std.Io.Timestamp };

pub const Tracer = struct {
    userdata: ?*anyopaque,
    vtable: *const VTable,
    redactor: Redactor,
    diagnostics: DiagnosticSink,

    pub const VTable = struct {
        begin: *const fn (?*anyopaque, SpanStart) outcomes.Exit(SpanId, outcomes.Defect),
        end: *const fn (?*anyopaque, SpanFinish) outcomes.Exit(void, outcomes.Defect),
        // Required lifecycle hook: a failed export cannot orphan a native span.
        abandon: *const fn (?*anyopaque, SpanId) outcomes.Exit(void, outcomes.Defect),
    };

    pub fn begin(self: Tracer, parent: ?SpanId, name: []const u8, annotations: []const Annotation, time: ?std.Io.Timestamp) ?SpanId {
        var result = self.vtable.begin(self.userdata, .{ .parent = parent, .name = name, .annotations = .{ .values = annotations, .redactor = self.redactor }, .time = time });
        defer result.deinit();
        self.diagnostics.reportExit(result);
        return if (result == .success) result.success else null;
    }

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

pub const Console = struct {
    userdata: *anyopaque,
    write_fn: *const fn (*anyopaque, []const u8) outcomes.Exit(void, outcomes.Defect),
    error_fn: *const fn (*anyopaque, []const u8) outcomes.Exit(void, outcomes.Defect),
    pub fn write(self: Console, message: []const u8) outcomes.Exit(void, outcomes.Defect) {
        return self.write_fn(self.userdata, message);
    }
    pub fn err(self: Console, message: []const u8) outcomes.Exit(void, outcomes.Defect) {
        return self.error_fn(self.userdata, message);
    }
};
pub const NativeConsole = struct {
    output: *std.Io.Writer,
    errors: *std.Io.Writer,
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
pub fn freeAnnotations(allocator: std.mem.Allocator, values: []Annotation) void {
    for (values) |annotation| {
        allocator.free(annotation.key);
        allocator.free(annotation.value);
    }
    allocator.free(values);
}
