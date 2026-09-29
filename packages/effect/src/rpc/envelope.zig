//! Bounded, transport-neutral RPC envelopes. Framing adds only the outer
//! length prefix; operation identity, correlation and application metadata
//! remain independent of HTTP or any other transport.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const Protocol = @import("rpc_error.zig").Protocol;

pub const Kind = enum(u8) {
    request,
    response,
    stream_chunk,
    stream_end,
    acknowledge,
    cancel,
    response_failure,
    response_defect,
    response_interrupted,
};
pub const Length = u32;
pub const Count = u32;
pub const Version = u32;
pub const Correlation = u128;
pub const Sequence = u64;
const Bytes = outcomes.OwnedSlice(u8);
const fixed_header_bytes = @sizeOf(Version) + @sizeOf(Kind) + @sizeOf(Correlation) +
    @sizeOf(Length) + @sizeOf(Count) + @sizeOf(Length);
const metadata_header_bytes = @sizeOf(Length) + @sizeOf(Length);

fn carriesSequence(kind: Kind) bool {
    return switch (kind) {
        .stream_chunk, .stream_end, .acknowledge => true,
        else => false,
    };
}

pub const Limits = struct {
    max_envelope_bytes: usize,
    max_operation_bytes: usize,
    max_payload_bytes: usize,
    max_metadata_entries: usize,
    max_metadata_bytes: usize,

    pub fn valid(self: Limits) bool {
        return self.max_envelope_bytes > 0 and self.max_operation_bytes > 0 and
            self.max_operation_bytes <= std.math.maxInt(Length) and
            self.max_payload_bytes <= std.math.maxInt(Length) and
            self.max_metadata_entries <= std.math.maxInt(Count) and
            self.max_metadata_bytes <= std.math.maxInt(Length);
    }
};

pub const Metadata = struct {
    pub const effect_owned_payload = true;
    key: Bytes,
    value: Bytes,

    pub fn deinit(self: *Metadata) void {
        self.key.deinit();
        self.value.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const Metadata) error{OutOfMemory}!Metadata {
        var key = try self.key.clone();
        errdefer key.deinit();
        return .{ .key = key, .value = try self.value.clone() };
    }
};

pub const Envelope = struct {
    pub const effect_owned_payload = true;
    version: Version,
    kind: Kind,
    correlation: Correlation,
    sequence: ?Sequence = null,
    operation: Bytes,
    metadata: []Metadata,
    payload: Bytes,
    allocator: std.mem.Allocator,

    pub fn deinit(self: *Envelope) void {
        self.operation.deinit();
        for (self.metadata) |*entry| entry.deinit();
        self.allocator.free(self.metadata);
        self.payload.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const Envelope) error{OutOfMemory}!Envelope {
        var operation = try self.operation.clone();
        errdefer operation.deinit();
        const metadata = try self.allocator.alloc(Metadata, self.metadata.len);
        var count: usize = 0;
        errdefer {
            for (metadata[0..count]) |*entry| entry.deinit();
            self.allocator.free(metadata);
        }
        for (self.metadata) |*entry| {
            metadata[count] = try entry.clone();
            count += 1;
        }
        return .{
            .version = self.version,
            .kind = self.kind,
            .correlation = self.correlation,
            .sequence = self.sequence,
            .operation = operation,
            .metadata = metadata,
            .payload = try self.payload.clone(),
            .allocator = self.allocator,
        };
    }
};

pub const View = struct {
    version: Version,
    kind: Kind,
    correlation: Correlation,
    sequence: ?Sequence = null,
    operation: []const u8,
    metadata: []const Entry,
    payload: []const u8,

    pub const Entry = struct { key: []const u8, value: []const u8 };
};

fn appendInteger(list: *std.array_list.Managed(u8), comptime T: type, source: T) error{OutOfMemory}!void {
    var bytes: [@sizeOf(T)]u8 = undefined;
    var value = source;
    var index = bytes.len;
    while (index > 0) {
        index -= 1;
        bytes[index] = @truncate(value);
        if (comptime @bitSizeOf(T) > @bitSizeOf(u8)) {
            if (index > 0) value >>= @bitSizeOf(u8);
        }
    }
    try list.appendSlice(&bytes);
}

fn take(input: []const u8, cursor: *usize, count: usize) ?[]const u8 {
    if (count > input.len - cursor.*) return null;
    const value = input[cursor.* .. cursor.* + count];
    cursor.* += count;
    return value;
}

fn readInteger(input: []const u8, cursor: *usize, comptime T: type) ?T {
    const encoded = take(input, cursor, @sizeOf(T)) orelse return null;
    if (comptime T == u8) return encoded[0];
    var value: T = 0;
    for (encoded) |digit| value = (value << @bitSizeOf(u8)) | digit;
    return value;
}

fn copy(allocator: std.mem.Allocator, source: []const u8) error{OutOfMemory}!Bytes {
    return .{ .items = try allocator.dupe(u8, source), .allocator = allocator };
}

fn validText(source: []const u8) bool {
    return source.len > 0 and std.unicode.utf8ValidateSlice(source);
}

pub fn encode(allocator: std.mem.Allocator, value: View, limits: Limits) outcomes.Exit(Bytes, Protocol) {
    if (!limits.valid() or !validText(value.operation) or
        value.operation.len > limits.max_operation_bytes or value.payload.len > limits.max_payload_bytes or
        value.metadata.len > limits.max_metadata_entries or
        carriesSequence(value.kind) != (value.sequence != null)) return .{ .fail = .malformed_envelope };
    var metadata_bytes: usize = 0;
    var encoded_bytes = std.math.add(usize, fixed_header_bytes, value.operation.len) catch return .{ .fail = .frame_too_large };
    if (value.sequence != null) encoded_bytes = std.math.add(usize, encoded_bytes, @sizeOf(Sequence)) catch
        return .{ .fail = .frame_too_large };
    encoded_bytes = std.math.add(usize, encoded_bytes, value.payload.len) catch return .{ .fail = .frame_too_large };
    for (value.metadata) |entry| {
        if (!validText(entry.key) or entry.key.len > std.math.maxInt(Length) or
            entry.value.len > std.math.maxInt(Length)) return .{ .fail = .malformed_envelope };
        metadata_bytes = std.math.add(usize, metadata_bytes, entry.key.len) catch return .{ .fail = .frame_too_large };
        metadata_bytes = std.math.add(usize, metadata_bytes, entry.value.len) catch return .{ .fail = .frame_too_large };
        if (metadata_bytes > limits.max_metadata_bytes) return .{ .fail = .frame_too_large };
        encoded_bytes = std.math.add(usize, encoded_bytes, metadata_header_bytes) catch return .{ .fail = .frame_too_large };
        encoded_bytes = std.math.add(usize, encoded_bytes, entry.key.len) catch return .{ .fail = .frame_too_large };
        encoded_bytes = std.math.add(usize, encoded_bytes, entry.value.len) catch return .{ .fail = .frame_too_large };
    }
    if (encoded_bytes > limits.max_envelope_bytes) return .{ .fail = .frame_too_large };
    var buffer = std.array_list.Managed(u8).init(allocator);
    defer buffer.deinit();
    buffer.ensureTotalCapacity(encoded_bytes) catch return .{ .die = outcomes.Defect.out_of_memory };
    appendInteger(&buffer, Version, value.version) catch return .{ .die = outcomes.Defect.out_of_memory };
    buffer.append(@intFromEnum(value.kind)) catch return .{ .die = outcomes.Defect.out_of_memory };
    appendInteger(&buffer, Correlation, value.correlation) catch return .{ .die = outcomes.Defect.out_of_memory };
    if (value.sequence) |sequence| appendInteger(&buffer, Sequence, sequence) catch
        return .{ .die = outcomes.Defect.out_of_memory };
    appendInteger(&buffer, Length, @intCast(value.operation.len)) catch return .{ .die = outcomes.Defect.out_of_memory };
    appendInteger(&buffer, Count, @intCast(value.metadata.len)) catch return .{ .die = outcomes.Defect.out_of_memory };
    appendInteger(&buffer, Length, @intCast(value.payload.len)) catch return .{ .die = outcomes.Defect.out_of_memory };
    buffer.appendSlice(value.operation) catch return .{ .die = outcomes.Defect.out_of_memory };
    for (value.metadata) |entry| {
        appendInteger(&buffer, Length, @intCast(entry.key.len)) catch return .{ .die = outcomes.Defect.out_of_memory };
        appendInteger(&buffer, Length, @intCast(entry.value.len)) catch return .{ .die = outcomes.Defect.out_of_memory };
        buffer.appendSlice(entry.key) catch return .{ .die = outcomes.Defect.out_of_memory };
        buffer.appendSlice(entry.value) catch return .{ .die = outcomes.Defect.out_of_memory };
        if (buffer.items.len > limits.max_envelope_bytes) return .{ .fail = .frame_too_large };
    }
    buffer.appendSlice(value.payload) catch return .{ .die = outcomes.Defect.out_of_memory };
    if (buffer.items.len > limits.max_envelope_bytes) return .{ .fail = .frame_too_large };
    const owned = buffer.toOwnedSlice() catch return .{ .die = outcomes.Defect.out_of_memory };
    return .{ .success = .{ .items = owned, .allocator = allocator } };
}

pub fn decode(allocator: std.mem.Allocator, input: []const u8, limits: Limits) outcomes.Exit(Envelope, Protocol) {
    if (!limits.valid() or input.len > limits.max_envelope_bytes) return .{ .fail = .frame_too_large };
    var cursor: usize = 0;
    const version = readInteger(input, &cursor, Version) orelse return .{ .fail = .malformed_envelope };
    const raw_kind = readInteger(input, &cursor, u8) orelse return .{ .fail = .malformed_envelope };
    const kind = std.meta.intToEnum(Kind, raw_kind) catch return .{ .fail = .malformed_envelope };
    const correlation = readInteger(input, &cursor, Correlation) orelse return .{ .fail = .malformed_envelope };
    const sequence: ?Sequence = if (carriesSequence(kind))
        (readInteger(input, &cursor, Sequence) orelse return .{ .fail = .malformed_envelope })
    else
        null;
    const operation_len = readInteger(input, &cursor, Length) orelse return .{ .fail = .malformed_envelope };
    const metadata_count = readInteger(input, &cursor, Count) orelse return .{ .fail = .malformed_envelope };
    const payload_len = readInteger(input, &cursor, Length) orelse return .{ .fail = .malformed_envelope };
    if (operation_len > limits.max_operation_bytes or metadata_count > limits.max_metadata_entries or
        payload_len > limits.max_payload_bytes) return .{ .fail = .frame_too_large };
    const operation_view = take(input, &cursor, operation_len) orelse return .{ .fail = .malformed_envelope };
    if (!validText(operation_view)) return .{ .fail = .malformed_envelope };
    if (metadata_count > (input.len - cursor) / metadata_header_bytes)
        return .{ .fail = .malformed_envelope };
    var operation = copy(allocator, operation_view) catch return .{ .die = outcomes.Defect.out_of_memory };
    var keep_operation = false;
    defer if (!keep_operation) operation.deinit();
    const metadata = allocator.alloc(Metadata, metadata_count) catch return .{ .die = outcomes.Defect.out_of_memory };
    var copied: usize = 0;
    var keep_metadata = false;
    defer if (!keep_metadata) {
        for (metadata[0..copied]) |*entry| entry.deinit();
        allocator.free(metadata);
    };
    var metadata_bytes: usize = 0;
    while (copied < metadata.len) {
        const key_len = readInteger(input, &cursor, Length) orelse return .{ .fail = .malformed_envelope };
        const value_len = readInteger(input, &cursor, Length) orelse return .{ .fail = .malformed_envelope };
        metadata_bytes = std.math.add(usize, metadata_bytes, key_len) catch return .{ .fail = .frame_too_large };
        metadata_bytes = std.math.add(usize, metadata_bytes, value_len) catch return .{ .fail = .frame_too_large };
        if (metadata_bytes > limits.max_metadata_bytes) return .{ .fail = .frame_too_large };
        const key_view = take(input, &cursor, key_len) orelse return .{ .fail = .malformed_envelope };
        const value_view = take(input, &cursor, value_len) orelse return .{ .fail = .malformed_envelope };
        if (!validText(key_view)) return .{ .fail = .malformed_envelope };
        var key = copy(allocator, key_view) catch return .{ .die = outcomes.Defect.out_of_memory };
        const value = copy(allocator, value_view) catch {
            key.deinit();
            return .{ .die = outcomes.Defect.out_of_memory };
        };
        metadata[copied] = .{ .key = key, .value = value };
        copied += 1;
    }
    const payload_view = take(input, &cursor, payload_len) orelse return .{ .fail = .malformed_envelope };
    if (cursor != input.len) return .{ .fail = .malformed_envelope };
    const payload = copy(allocator, payload_view) catch return .{ .die = outcomes.Defect.out_of_memory };
    keep_operation = true;
    keep_metadata = true;
    return .{ .success = .{
        .version = version,
        .kind = kind,
        .correlation = correlation,
        .sequence = sequence,
        .operation = operation,
        .metadata = metadata,
        .payload = payload,
        .allocator = allocator,
    } };
}
