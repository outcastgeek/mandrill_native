//! Stable, namespaced identities shared by persisted and routed work.
//! The wire form is versioned, typed and length delimited. It is never a
//! process hash or an ambiguous concatenation of application strings.
const std = @import("std");
const effects = @import("effect.zig");
const outcomes = @import("outcome.zig");
const runtime = @import("internal/runtime.zig");

pub const Version = enum(u8) { first = 1 };
pub const PartKind = enum(u8) { text, bytes, unsigned };

/// A borrowed description. Each slice and the selected environment must live
/// through the Effect run. Encoding copies everything into an owning buffer.
pub const Part = union(PartKind) {
    text: []const u8,
    bytes: []const u8,
    unsigned: u64,
};

const Key = struct {
    namespace: []const u8,
    parts: []const Part,
};

pub fn TypedKey(comptime Domain: type) type {
    if (!@hasDecl(Domain, "key_namespace")) @compileError("a key domain needs a stable key_namespace");
    const namespace: []const u8 = Domain.key_namespace;
    if (namespace.len == 0) @compileError("a key domain needs a nonempty key_namespace");
    if (!std.unicode.utf8ValidateSlice(namespace)) @compileError("key_namespace must be UTF-8");
    return struct {
        pub const domain = Domain;
        pub const key_namespace = namespace;
        parts: []const Part,

        pub fn encode(comptime R: type, comptime select: fn (*const R) @This()) effects.Effect(outcomes.OwnedSlice(u8), Error, R) {
            const Field = @tagName(runtime.EnvironmentField.allocator);
            if (R == void or !@hasField(R, Field)) @compileError("PrimaryKey.encode requires the caller allocator in R");
            if (@FieldType(R, Field) != std.mem.Allocator) @compileError("R.allocator must be std.mem.Allocator");
            return effects.Effect(outcomes.OwnedSlice(u8), Error, R).fromOutcome(struct {
                fn encodeSelected(env: *const R) outcomes.Exit(outcomes.OwnedSlice(u8), Error) {
                    return encodeBorrowed(env.allocator, .{ .namespace = namespace, .parts = select(env).parts });
                }
            }.encodeSelected);
        }

        pub fn inspect(encoded: []const u8) outcomes.Result(void, Error) {
            const checked = inspectEncoded(encoded);
            return switch (checked) {
                .err => |err| .{ .err = err },
                .ok => |found| if (std.mem.eql(u8, namespace, found)) .{ .ok = {} } else .{ .err = .wrong_namespace },
            };
        }
    };
}

/// Internal and provider code may assemble a short-lived parts array inside
/// one Effect callback. Encoding copies it before that callback returns.
/// The allocator is selected only through the caller's environment.
pub fn encodeDirect(env: anytype, comptime namespace: []const u8, parts: []const Part) outcomes.Exit(outcomes.OwnedSlice(u8), Error) {
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
        @compileError("PrimaryKey.encodeDirect requires std.mem.Allocator in R");
    if (comptime namespace.len == 0 or !std.unicode.utf8ValidateSlice(namespace))
        @compileError("PrimaryKey namespace must be stable nonempty UTF-8");
    return encodeBorrowed(env.allocator, .{ .namespace = namespace, .parts = parts });
}

pub const Error = union(enum) {
    empty_namespace,
    invalid_text,
    wrong_namespace,
    length_overflow,
    incompatible_version: u8,
    malformed,
};

/// Protocol v1: one version byte, an unsigned 64-bit big-endian namespace
/// length, namespace bytes, a 64-bit part count, then each part's kind byte,
/// 64-bit byte length and payload. An unsigned part always has eight big-endian
/// bytes. This is the complete canonical byte layout, not a hash of the key.
fn encodeBorrowed(allocator: std.mem.Allocator, key: Key) outcomes.Exit(outcomes.OwnedSlice(u8), Error) {
    if (key.namespace.len == 0) return .{ .fail = .empty_namespace };
    if (!std.unicode.utf8ValidateSlice(key.namespace)) return .{ .fail = .invalid_text };
    const wire_count: usize = @sizeOf(u64);
    _ = std.math.cast(u64, key.namespace.len) orelse return .{ .fail = .length_overflow };
    _ = std.math.cast(u64, key.parts.len) orelse return .{ .fail = .length_overflow };

    var total: usize = @sizeOf(u8);
    total = std.math.add(usize, total, wire_count) catch return .{ .fail = .length_overflow };
    total = std.math.add(usize, total, key.namespace.len) catch return .{ .fail = .length_overflow };
    total = std.math.add(usize, total, wire_count) catch return .{ .fail = .length_overflow };
    for (key.parts) |part| {
        const payload_len: usize = switch (part) {
            .text => |value| blk: {
                if (!std.unicode.utf8ValidateSlice(value)) return .{ .fail = .invalid_text };
                break :blk value.len;
            },
            .bytes => |value| value.len,
            .unsigned => wire_count,
        };
        _ = std.math.cast(u64, payload_len) orelse return .{ .fail = .length_overflow };
        total = std.math.add(usize, total, @sizeOf(u8) + wire_count) catch return .{ .fail = .length_overflow };
        total = std.math.add(usize, total, payload_len) catch return .{ .fail = .length_overflow };
    }

    const buffer = allocator.alloc(u8, total) catch return .{ .die = outcomes.Defect.out_of_memory };
    var cursor: usize = 0;
    buffer[cursor] = @intFromEnum(Version.first);
    cursor += @sizeOf(u8);
    writeU64(buffer, &cursor, @intCast(key.namespace.len));
    writeBytes(buffer, &cursor, key.namespace);
    writeU64(buffer, &cursor, @intCast(key.parts.len));
    for (key.parts) |part| {
        buffer[cursor] = @intFromEnum(std.meta.activeTag(part));
        cursor += @sizeOf(u8);
        switch (part) {
            .text => |value| {
                writeU64(buffer, &cursor, @intCast(value.len));
                writeBytes(buffer, &cursor, value);
            },
            .bytes => |value| {
                writeU64(buffer, &cursor, @intCast(value.len));
                writeBytes(buffer, &cursor, value);
            },
            .unsigned => |value| {
                writeU64(buffer, &cursor, @intCast(wire_count));
                writeU64(buffer, &cursor, value);
            },
        }
    }
    std.debug.assert(cursor == total);
    return .{ .success = .{ .items = buffer, .allocator = allocator } };
}

fn writeU64(buffer: []u8, cursor: *usize, value: u64) void {
    const width: usize = @sizeOf(u64);
    for (0..width) |index| {
        const remaining = width - index - 1;
        buffer[cursor.* + index] = @truncate(value >> @intCast(remaining * @bitSizeOf(u8)));
    }
    cursor.* += width;
}

fn writeBytes(buffer: []u8, cursor: *usize, bytes: []const u8) void {
    std.mem.copyForwards(u8, buffer[cursor.*..][0..bytes.len], bytes);
    cursor.* += bytes.len;
}

pub fn equalEncoded(left: []const u8, right: []const u8) bool {
    return std.mem.eql(u8, left, right);
}

/// Validates the entire byte sequence. The returned namespace borrows encoded.
pub fn inspectEncoded(encoded: []const u8) outcomes.Result([]const u8, Error) {
    var cursor: usize = 0;
    const version = readByte(encoded, &cursor) orelse return .{ .err = .malformed };
    if (version != @intFromEnum(Version.first)) return .{ .err = .{ .incompatible_version = version } };
    const namespace_len = readLength(encoded, &cursor) orelse return .{ .err = .malformed };
    const namespace = readBytes(encoded, &cursor, namespace_len) orelse return .{ .err = .malformed };
    if (namespace.len == 0) return .{ .err = .empty_namespace };
    if (!std.unicode.utf8ValidateSlice(namespace)) return .{ .err = .invalid_text };
    const count = readLength(encoded, &cursor) orelse return .{ .err = .malformed };
    // Each part requires at least a kind byte and a complete length prefix.
    if (count > (encoded.len - cursor) / (@sizeOf(u8) + @sizeOf(u64))) return .{ .err = .malformed };
    for (0..count) |_| {
        const tag = readByte(encoded, &cursor) orelse return .{ .err = .malformed };
        const kind: PartKind = switch (tag) {
            @intFromEnum(PartKind.text) => .text,
            @intFromEnum(PartKind.bytes) => .bytes,
            @intFromEnum(PartKind.unsigned) => .unsigned,
            else => return .{ .err = .malformed },
        };
        const len = readLength(encoded, &cursor) orelse return .{ .err = .malformed };
        const payload = readBytes(encoded, &cursor, len) orelse return .{ .err = .malformed };
        switch (kind) {
            .text => if (!std.unicode.utf8ValidateSlice(payload)) return .{ .err = .invalid_text },
            .bytes => {},
            .unsigned => if (payload.len != @sizeOf(u64)) return .{ .err = .malformed },
        }
    }
    if (cursor != encoded.len) return .{ .err = .malformed };
    return .{ .ok = namespace };
}

fn readByte(encoded: []const u8, cursor: *usize) ?u8 {
    if (cursor.* >= encoded.len) return null;
    const result = encoded[cursor.*];
    cursor.* += @sizeOf(u8);
    return result;
}

fn readU64(encoded: []const u8, cursor: *usize) ?u64 {
    const width = @sizeOf(u64);
    if (cursor.* > encoded.len or encoded.len - cursor.* < width) return null;
    var value: u64 = 0;
    for (encoded[cursor.*..][0..width]) |byte| value = (value << @bitSizeOf(u8)) | byte;
    cursor.* += width;
    return value;
}

fn readLength(encoded: []const u8, cursor: *usize) ?usize {
    return std.math.cast(usize, readU64(encoded, cursor) orelse return null);
}

fn readBytes(encoded: []const u8, cursor: *usize, len: usize) ?[]const u8 {
    if (cursor.* > encoded.len or len > encoded.len - cursor.*) return null;
    const bytes = encoded[cursor.*..][0..len];
    cursor.* += len;
    return bytes;
}
