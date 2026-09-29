//! Versioned, data-only Exit representation. Payload codecs supply the bytes
//! for typed successes and failures; this layer preserves terminal tags,
//! defect diagnostics, interruption and ordered cleanup failures. It never
//! persists process pointers, functions, scopes or live runtime objects.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const Issue = @import("schema_issue.zig").Issue;

pub const Bytes = outcomes.OwnedSlice(u8);
pub const Version = enum(u16) { first = 1 };
pub const Limits = struct {
    max_payload_bytes: usize,
    max_cleanup_records: usize,
    max_total_bytes: usize,

    pub fn valid(self: Limits) bool {
        return self.max_total_bytes > @sizeOf(u16) + @sizeOf(u8) + @sizeOf(u32);
    }
};

const PrimaryTag = enum(u8) {
    success = 1,
    failure = 2,
    defect = 3,
    interrupted = 4,
    succeeded_before_cleanup_failure = 5,
};
const CleanupTag = enum(u8) { failure = 1, defect = 2, interrupted = 3 };
const DefectTag = enum(u8) {
    explicit = 1,
    out_of_memory = 2,
    native_failure = 3,
    provider_unavailable = 4,
    invalid_configuration = 5,
};

pub const DefectData = struct {
    pub const effect_owned_payload = true;
    kind: outcomes.Defect.Kind,
    diagnostic: Bytes,

    pub fn deinit(self: *@This()) void {
        self.diagnostic.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{ .kind = self.kind, .diagnostic = try self.diagnostic.clone() };
    }
};

pub const Primary = union(enum) {
    pub const effect_owned_payload = true;
    success: Bytes,
    failure: Bytes,
    defect: DefectData,
    interrupted,
    succeeded_before_cleanup_failure,

    pub fn deinit(self: *@This()) void {
        switch (self.*) {
            .success => self.success.deinit(),
            .failure => self.failure.deinit(),
            .defect => self.defect.deinit(),
            else => {},
        }
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .success => |value| .{ .success = try value.clone() },
            .failure => |value| .{ .failure = try value.clone() },
            .defect => |value| .{ .defect = try value.clone() },
            else => self.*,
        };
    }
};

pub const Cleanup = union(enum) {
    pub const effect_owned_payload = true;
    failure: Bytes,
    defect: DefectData,
    interrupted,

    pub fn deinit(self: *@This()) void {
        switch (self.*) {
            .failure => self.failure.deinit(),
            .defect => self.defect.deinit(),
            .interrupted => {},
        }
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .failure => |value| .{ .failure = try value.clone() },
            .defect => |value| .{ .defect = try value.clone() },
            .interrupted => .interrupted,
        };
    }
};

pub const StoredExit = struct {
    pub const effect_owned_payload = true;
    primary: Primary,
    cleanup: outcomes.OwnedSlice(Cleanup),

    pub fn deinit(self: *@This()) void {
        self.primary.deinit();
        self.cleanup.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        var primary = try self.primary.clone();
        errdefer primary.deinit();
        return .{ .primary = primary, .cleanup = try self.cleanup.clone() };
    }

    pub fn valid(self: *const @This()) bool {
        if (self.primary == .success and self.cleanup.items.len != 0) return false;
        if (self.primary == .succeeded_before_cleanup_failure and self.cleanup.items.len == 0) return false;
        return true;
    }
};

const FormatError = error{ OutOfMemory, Invalid, TooLarge, IncompatibleVersion };

const Writer = struct {
    allocator: std.mem.Allocator,
    bytes: std.array_list.Managed(u8),
    limits: Limits,

    fn init(allocator: std.mem.Allocator, limits: Limits) Writer {
        return .{ .allocator = allocator, .bytes = std.array_list.Managed(u8).init(allocator), .limits = limits };
    }

    fn write(self: *Writer, input: []const u8) FormatError!void {
        const next = std.math.add(usize, self.bytes.items.len, input.len) catch return error.TooLarge;
        if (next > self.limits.max_total_bytes) return error.TooLarge;
        try self.bytes.appendSlice(input);
    }

    fn byte(self: *Writer, value: u8) FormatError!void {
        const buffer = [_]u8{value};
        try self.write(&buffer);
    }

    fn unsigned(self: *Writer, comptime T: type, value: T) FormatError!void {
        if (@typeInfo(T) != .int or @typeInfo(T).int.signedness != .unsigned)
            @compileError("wire integers must be unsigned");
        var bytes: [@sizeOf(T)]u8 = undefined;
        var remaining = value;
        var index = bytes.len;
        while (index > 0) {
            index -= 1;
            bytes[index] = @truncate(remaining);
            remaining >>= @bitSizeOf(u8);
        }
        try self.write(&bytes);
    }

    fn payload(self: *Writer, value: []const u8) FormatError!void {
        if (value.len > self.limits.max_payload_bytes or value.len > std.math.maxInt(u32)) return error.TooLarge;
        try self.unsigned(u32, @intCast(value.len));
        try self.write(value);
    }

    fn defect(self: *Writer, value: DefectData) FormatError!void {
        const tag: DefectTag = switch (value.kind) {
            .explicit => .explicit,
            .out_of_memory => .out_of_memory,
            .native_failure => .native_failure,
            .provider_unavailable => .provider_unavailable,
            .invalid_configuration => .invalid_configuration,
        };
        try self.byte(@intFromEnum(tag));
        try self.payload(value.diagnostic.items);
    }

    fn primary(self: *Writer, value: Primary) FormatError!void {
        switch (value) {
            .success => |bytes| {
                try self.byte(@intFromEnum(PrimaryTag.success));
                try self.payload(bytes.items);
            },
            .failure => |bytes| {
                try self.byte(@intFromEnum(PrimaryTag.failure));
                try self.payload(bytes.items);
            },
            .defect => |defect_value| {
                try self.byte(@intFromEnum(PrimaryTag.defect));
                try self.defect(defect_value);
            },
            .interrupted => try self.byte(@intFromEnum(PrimaryTag.interrupted)),
            .succeeded_before_cleanup_failure => try self.byte(@intFromEnum(PrimaryTag.succeeded_before_cleanup_failure)),
        }
    }

    fn cleanup(self: *Writer, value: Cleanup) FormatError!void {
        switch (value) {
            .failure => |bytes| {
                try self.byte(@intFromEnum(CleanupTag.failure));
                try self.payload(bytes.items);
            },
            .defect => |defect_value| {
                try self.byte(@intFromEnum(CleanupTag.defect));
                try self.defect(defect_value);
            },
            .interrupted => try self.byte(@intFromEnum(CleanupTag.interrupted)),
        }
    }
};

const Reader = struct {
    allocator: std.mem.Allocator,
    source: []const u8,
    offset: usize = 0,
    limits: Limits,

    fn take(self: *Reader, count: usize) FormatError![]const u8 {
        const end = std.math.add(usize, self.offset, count) catch return error.Invalid;
        if (end > self.source.len) return error.Invalid;
        const result = self.source[self.offset..end];
        self.offset = end;
        return result;
    }

    fn byte(self: *Reader) FormatError!u8 {
        return (try self.take(1))[0];
    }

    fn unsigned(self: *Reader, comptime T: type) FormatError!T {
        if (@typeInfo(T) != .int or @typeInfo(T).int.signedness != .unsigned)
            @compileError("wire integers must be unsigned");
        const input = try self.take(@sizeOf(T));
        var result: T = 0;
        for (input) |digit| result = (result << @bitSizeOf(u8)) | digit;
        return result;
    }

    fn payload(self: *Reader) FormatError!Bytes {
        const size = try self.unsigned(u32);
        if (size > self.limits.max_payload_bytes) return error.TooLarge;
        const borrowed = try self.take(size);
        return .{ .items = try self.allocator.dupe(u8, borrowed), .allocator = self.allocator };
    }

    fn defect(self: *Reader) FormatError!DefectData {
        const tag = std.enums.fromInt(DefectTag, try self.byte()) orelse return error.Invalid;
        const kind: outcomes.Defect.Kind = switch (tag) {
            .explicit => .explicit,
            .out_of_memory => .out_of_memory,
            .native_failure => .native_failure,
            .provider_unavailable => .provider_unavailable,
            .invalid_configuration => .invalid_configuration,
        };
        return .{ .kind = kind, .diagnostic = try self.payload() };
    }

    fn primary(self: *Reader) FormatError!Primary {
        const tag = std.enums.fromInt(PrimaryTag, try self.byte()) orelse return error.Invalid;
        return switch (tag) {
            .success => .{ .success = try self.payload() },
            .failure => .{ .failure = try self.payload() },
            .defect => .{ .defect = try self.defect() },
            .interrupted => .interrupted,
            .succeeded_before_cleanup_failure => .succeeded_before_cleanup_failure,
        };
    }

    fn cleanup(self: *Reader) FormatError!Cleanup {
        const tag = std.enums.fromInt(CleanupTag, try self.byte()) orelse return error.Invalid;
        return switch (tag) {
            .failure => .{ .failure = try self.payload() },
            .defect => .{ .defect = try self.defect() },
            .interrupted => .interrupted,
        };
    }
};

fn formatFailure(env: anytype, problem: FormatError, direction: @import("schema_issue.zig").Direction) outcomes.Exit(StoredExit, Issue) {
    return switch (problem) {
        error.OutOfMemory => .{ .die = outcomes.Defect.out_of_memory },
        error.TooLarge => .{ .fail = Issue.init(env, direction, .size_limit) },
        error.IncompatibleVersion => .{ .fail = Issue.init(env, direction, .incompatible_version) },
        error.Invalid => .{ .fail = Issue.init(env, direction, .invalid_value) },
    };
}

pub fn encode(env: anytype, stored: *const StoredExit, limits: Limits) outcomes.Exit(Bytes, Issue) {
    if (!limits.valid() or !stored.valid() or stored.cleanup.items.len > limits.max_cleanup_records or
        stored.cleanup.items.len > std.math.maxInt(u32))
        return .{ .fail = Issue.init(env, .encode, .invalid_value) };
    var writer = Writer.init(env.allocator, limits);
    defer writer.bytes.deinit();
    const failure: ?FormatError = blk: {
        writer.unsigned(u16, @intFromEnum(Version.first)) catch |problem| break :blk problem;
        writer.primary(stored.primary) catch |problem| break :blk problem;
        writer.unsigned(u32, @intCast(stored.cleanup.items.len)) catch |problem| break :blk problem;
        for (stored.cleanup.items) |entry| writer.cleanup(entry) catch |problem| break :blk problem;
        break :blk null;
    };
    if (failure) |problem| return switch (problem) {
        error.OutOfMemory => .{ .die = outcomes.Defect.out_of_memory },
        error.TooLarge => .{ .fail = Issue.init(env, .encode, .size_limit) },
        else => .{ .fail = Issue.init(env, .encode, .invalid_value) },
    };
    const owned = env.allocator.dupe(u8, writer.bytes.items) catch return .{ .die = outcomes.Defect.out_of_memory };
    return .{ .success = .{ .items = owned, .allocator = env.allocator } };
}

pub fn decode(env: anytype, encoded: []const u8, limits: Limits) outcomes.Exit(StoredExit, Issue) {
    if (!limits.valid()) return .{ .fail = Issue.init(env, .decode, .invalid_value) };
    if (encoded.len > limits.max_total_bytes) return .{ .fail = Issue.init(env, .decode, .size_limit) };
    var reader: Reader = .{ .allocator = env.allocator, .source = encoded, .limits = limits };
    const wire_version = reader.unsigned(u16) catch |problem| return formatFailure(env, problem, .decode);
    if (wire_version != @intFromEnum(Version.first)) return formatFailure(env, error.IncompatibleVersion, .decode);
    var primary = reader.primary() catch |problem| return formatFailure(env, problem, .decode);
    var keep_primary = false;
    defer if (!keep_primary) primary.deinit();
    const cleanup_count = reader.unsigned(u32) catch |problem| return formatFailure(env, problem, .decode);
    if (cleanup_count > limits.max_cleanup_records) return formatFailure(env, error.TooLarge, .decode);
    const cleanup = env.allocator.alloc(Cleanup, cleanup_count) catch return formatFailure(env, error.OutOfMemory, .decode);
    var initialized: usize = 0;
    var keep_cleanup = false;
    defer if (!keep_cleanup) {
        for (cleanup[0..initialized]) |*entry| entry.deinit();
        env.allocator.free(cleanup);
    };
    for (cleanup) |*entry| {
        entry.* = reader.cleanup() catch |problem| return formatFailure(env, problem, .decode);
        initialized += 1;
    }
    if (reader.offset != reader.source.len) return formatFailure(env, error.Invalid, .decode);
    const stored: StoredExit = .{ .primary = primary, .cleanup = .{ .items = cleanup, .allocator = env.allocator } };
    if (!stored.valid()) return formatFailure(env, error.Invalid, .decode);
    keep_primary = true;
    keep_cleanup = true;
    return .{ .success = stored };
}
