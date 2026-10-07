//! std.json-backed bounded UTF-8 emission. Effect retains policy/error conversion
//! and exact-number validation; std owns escaping, scalar spelling and punctuation.
//! State is per invocation. Returned bytes retain R.allocator, not stack scratch.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const Issue = @import("schema_issue.zig").Issue;
const Sink = @import("internal/json_sink.zig").Sink(Writer, Error, Writer.write);

/// Inclusive budgets for one encoded JSON document. All limits must be positive.
pub const Limits = struct {
    /// Maximum emitted UTF-8 bytes, including JSON escapes and punctuation.
    max_output_bytes: usize,
    /// Maximum simultaneous JSON values; a root scalar counts as depth one.
    max_json_depth: usize,
    /// Maximum unescaped string/key bytes or exact numeric token bytes.
    max_value_bytes: usize,
    /// Maximum decimal digits in the integer part after applying a numeric exponent.
    max_integer_digits: usize,
    /// Return whether every caller-selected budget is positive.
    pub fn valid(self: @This()) bool {
        return self.max_output_bytes > 0 and self.max_json_depth > 0 and self.max_value_bytes > 0 and self.max_integer_digits > 0;
    }
};
/// Native adapter failures translated by failure/failureWith into typed schema issues or an OOM defect.
pub const Error = error{
    /// The caller allocator could not retain output.
    OutOfMemory,
    /// Emitted bytes or one lexical value exceeded its inclusive byte budget.
    SizeLimit,
    /// Entering the next value would exceed the nesting budget.
    DepthLimit,
    /// A string or object key is not UTF-8.
    InvalidUnicode,
    /// A token is not a JSON decimal or a native float is nonfinite.
    InvalidNumber,
    /// The expanded integer part exceeds its digit budget or cannot be counted.
    IntegerDigits,
};
/// Invocation-local bounded sink using std.json for syntax and escaping.
/// Output uses the caller’s allocator; deinit or finish transfers/releases its storage.
/// Callers serialize access; this mutable writer does not synchronize concurrent use.
pub const Writer = struct {
    /// Accumulated bytes owned by this writer until finish transfers them.
    output: std.array_list.Managed(u8),
    /// Caller-selected budgets; no global policy is substituted.
    limits: Limits,
    /// Current entered value count; balanced even when writing fails.
    depth: usize = 0,
    /// Borrowable primitive limit detail from the latest bound failure; no allocated text is retained.
    resource_failure: ?@import("schema_issue.zig").Detail = null,
    /// Borrow the caller allocator for output. Validate limits before starting a public encode operation.
    pub fn init(env: anytype, limits: Limits) Writer {
        return .{ .output = .init(env.allocator), .limits = limits };
    }
    /// Release retained output storage; bytes already transferred by finish remain owned by its result.
    pub fn deinit(self: *Writer) void {
        self.output.deinit();
    }
    /// Transfer accumulated bytes into an OwnedSlice using their originating allocator.
    /// On allocation failure the writer still owns its buffer and must be deinitialized.
    pub fn finish(self: *Writer) error{OutOfMemory}!outcomes.OwnedSlice(u8) {
        return .{ .items = try self.output.toOwnedSlice(), .allocator = self.output.allocator };
    }
    /// Append already-encoded bytes within the total output budget. Does not validate raw JSON syntax.
    /// A limit failure records the attempted byte count and leaves already-written bytes owned by this writer.
    pub fn write(self: *Writer, bytes: []const u8) Error!void {
        const length = std.math.add(usize, self.output.items.len, bytes.len) catch std.math.maxInt(usize);
        if (length > self.limits.max_output_bytes) {
            self.resource_failure = .{ .resource = .{ .resource = .output_bytes, .limit = self.limits.max_output_bytes, .actual = length } };
            return error.SizeLimit;
        }
        try self.output.appendSlice(bytes);
    }
    /// Enter one JSON value, including scalars; reject before exceeding the caller’s nesting budget.
    pub fn enter(self: *Writer) Error!void {
        if (self.depth >= self.limits.max_json_depth) {
            self.resource_failure = .{ .resource = .{ .resource = .json_depth, .limit = self.limits.max_json_depth, .actual = self.depth + 1 } };
            return error.DepthLimit;
        }
        self.depth += 1;
    }
    /// Balance a successful enter; use defer so errors also restore invocation-local depth.
    pub fn leave(self: *Writer) void {
        self.depth -= 1;
    }
    /// Write a UTF-8 string token or field name using std escaping.
    /// Checks unescaped byte length; it does not enter a value, so keys do not increase JSON depth.
    pub fn quoted(self: *Writer, bytes: []const u8) Error!void {
        try self.checkString(bytes);
        var sink: Sink = .{ .target = self };
        std.json.Stringify.encodeJsonString(bytes, .{}, &sink.writer) catch return sink.failure.?;
    }

    /// Write a validated exact decimal token unchanged, checking lexical bytes and decimal integer digits.
    /// Does not enter a value; numeric is the whole-value form.
    pub fn number(self: *Writer, text: []const u8) Error!void {
        if (text.len > self.limits.max_value_bytes) {
            self.resource_failure = .{ .resource = .{ .resource = .value_bytes, .limit = self.limits.max_value_bytes, .actual = text.len } };
            return error.SizeLimit;
        }
        const number_value = @import("json_number_codec.zig").Number.parse(text) catch return error.InvalidNumber;
        const digits = number_value.integerDigits() catch {
            self.resource_failure = .{ .integer_digits = .{ .limit = self.limits.max_integer_digits, .required = null } };
            return error.IntegerDigits;
        };
        if (digits > self.limits.max_integer_digits) {
            self.resource_failure = .{ .integer_digits = .{ .limit = self.limits.max_integer_digits, .required = digits } };
            return error.IntegerDigits;
        }
        try self.write(text);
    }
    /// Write one complete string value with UTF-8, value-byte, output and depth checks.
    pub fn string(self: *Writer, bytes: []const u8) Error!void {
        try self.enter();
        defer self.leave();
        try self.quoted(bytes);
    }
    /// Write one complete exact number value without rounding through f64.
    pub fn numeric(self: *Writer, text: []const u8) Error!void {
        try self.enter();
        defer self.leave();
        try self.number(text);
    }
    /// Write one complete boolean value through std.json, enforcing output and depth budgets.
    pub fn boolean(self: *Writer, value: bool) Error!void {
        try self.enter();
        defer self.leave();
        var sink: Sink = .{ .target = self };
        std.json.Stringify.value(value, .{}, &sink.writer) catch return sink.failure.?;
    }
    /// Write a borrowed std.json.Value without copying it.
    /// Traversal enforces caller depth/value/digit budgets; std emits punctuation, escapes and scalar spelling.
    /// Each container has its own shallow std grammar state, avoiding std’s fixed debug-stack depth as a policy.
    pub fn json(self: *Writer, value: *const std.json.Value) Error!void {
        try self.enter();
        defer self.leave();
        var sink: Sink = .{ .target = self };
        // Each std emitter sees only this container. Recursive policy traversal
        // remains here, so std's fixed debug grammar stack adds no depth cap.
        var emitter: std.json.Stringify = .{ .writer = &sink.writer };
        switch (value.*) {
            .null => emitter.write(null) catch return sink.failure.?,
            .bool => |b| emitter.write(b) catch return sink.failure.?,
            .string => |text| try self.quoted(text),
            .number_string => |text| try self.number(text),
            .integer => |integer| try self.scalarNumber(integer),
            .float => |float| {
                if (!std.math.isFinite(float)) return error.InvalidNumber;
                try self.scalarNumber(float);
            },
            .array => |array| {
                emitter.beginArray() catch return sink.failure.?;
                for (array.items) |*item| {
                    emitter.beginWriteRaw() catch return sink.failure.?;
                    try self.json(item);
                    emitter.endWriteRaw();
                }
                emitter.endArray() catch return sink.failure.?;
            },
            .object => |object| {
                emitter.beginObject() catch return sink.failure.?;
                var iterator = object.iterator();
                while (iterator.next()) |item| {
                    try self.checkString(item.key_ptr.*);
                    emitter.objectField(item.key_ptr.*) catch return sink.failure.?;
                    emitter.beginWriteRaw() catch return sink.failure.?;
                    try self.json(item.value_ptr);
                    emitter.endWriteRaw();
                }
                emitter.endObject() catch return sink.failure.?;
            },
        }
    }
    fn scalarNumber(self: *Writer, value: anytype) Error!void {
        // The std formatter determines spelling. Storage derives from worst
        // finite f64 decimal/exponent and i64 bounds, never caller input length.
        var bytes: [std.fmt.count("{d}", .{-std.math.floatMax(f64)}) + std.fmt.count("{e}", .{std.math.floatMin(f64)}) + std.fmt.count("{d}", .{std.math.minInt(i64)})]u8 = undefined;
        var output: std.Io.Writer = .fixed(&bytes);
        std.json.Stringify.value(value, .{}, &output) catch return error.InvalidNumber;
        try self.number(output.buffered());
    }
    fn checkString(self: *Writer, bytes: []const u8) Error!void {
        if (bytes.len > self.limits.max_value_bytes) {
            self.resource_failure = .{ .resource = .{ .resource = .value_bytes, .limit = self.limits.max_value_bytes, .actual = bytes.len } };
            return error.SizeLimit;
        }
        if (!std.unicode.utf8ValidateSlice(bytes)) return error.InvalidUnicode;
    }
};
/// Map a native writer failure to a typed Issue; OutOfMemory uses .die instead of typed validation failure.
pub fn failure(env: anytype, direction: @import("schema_issue.zig").Direction, problem: Error) outcomes.Exit(outcomes.OwnedSlice(u8), Issue) {
    return switch (problem) {
        error.OutOfMemory => .{ .die = outcomes.Defect.out_of_memory },
        error.SizeLimit => .{ .fail = Issue.init(env, direction, .size_limit) },
        error.DepthLimit => .{ .fail = Issue.init(env, direction, .depth_limit) },
        error.InvalidUnicode => .{ .fail = Issue.init(env, direction, .invalid_utf8) },
        error.InvalidNumber => .{ .fail = Issue.init(env, direction, .invalid_number) },
        error.IntegerDigits => .{ .fail = Issue.init(env, direction, .integer_digits) },
    };
}
/// Map a writer failure and preserve its recorded resource/digit detail without transferring output.
pub fn failureWith(env: anytype, direction: @import("schema_issue.zig").Direction, problem: Error, value: *const Writer) outcomes.Exit(outcomes.OwnedSlice(u8), Issue) {
    var result = failure(env, direction, problem);
    if (result == .fail) result.fail.detail = value.resource_failure;
    return result;
}
/// Check actual public direct-writer evidence at comptime; AST shape and a matching method name alone are insufficient.
pub fn capable(comptime child: anytype) bool {
    const C = if (@TypeOf(child) == type) child else @TypeOf(child);
    if (!@hasDecl(C, "writeJson")) return false;
    if (@hasDecl(C, "supports_direct_writer")) return C.supports_direct_writer;
    if (@TypeOf(child) != type and @hasField(C, "writer")) return child.writer != null;
    return false;
}

/// Require direct-writer evidence and return its lazy Effect adapter. Reject ineligible codecs at comptime.
pub fn requireWriter(comptime child: anytype) type {
    const C = if (@TypeOf(child) == type) child else @TypeOf(child);
    if (!capable(child)) @compileError("codec has no direct JSON writer evidence");
    if (@TypeOf(child) != type and @hasField(C, "writer") and child.writer == null) @compileError("codec has no direct JSON writer evidence");
    return struct {
        /// Borrow value until this lazy operation joins; select bounds from the current R on each run.
        /// R supplies allocator and child services. Success owns its bytes; all child Exit failure/cleanup channels survive.
        pub fn writeWith(comptime R: type, value: *const C.Decoded, comptime select: fn (*const R) Limits) @import("../effect.zig").Effect(outcomes.OwnedSlice(u8), Issue, R) {
            return @import("../effect.zig").Effect(outcomes.OwnedSlice(u8), Issue, R).fromBorrowed(C.Decoded, value, struct {
                fn run(input: *const C.Decoded, env: *const R) outcomes.Exit(outcomes.OwnedSlice(u8), Issue) {
                    const limits = select(env);
                    if (!limits.valid()) return .{ .fail = Issue.init(env, .encode, .invalid_value) };
                    var writer = Writer.init(env, limits);
                    defer writer.deinit();
                    if (@TypeOf(child) == type) {
                        const checked = C.writeJson(env, input, &writer);
                        if (checked != .success) return outcomes.castFailure(outcomes.OwnedSlice(u8), checked);
                    } else {
                        const checked = child.writeJson(input, &writer);
                        if (checked != .success) return outcomes.castFailure(outcomes.OwnedSlice(u8), checked);
                    }
                    return .{ .success = writer.finish() catch return .{ .die = outcomes.Defect.out_of_memory } };
                }
            }.run);
        }
    };
}
