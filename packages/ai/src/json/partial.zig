//! Standard JSON parsing and scanner-based partial values, with the narrowly
//! approved Pi compatibility repair for malformed JSON string characters.
const std = @import("std");
const j = @import("value.zig");
const Value = std.json.Value;
pub const Arguments = struct { arguments: Value, argumentsError: ?[]const u8 = null };
pub fn repairJson(a: std.mem.Allocator, bytes: []const u8) std.mem.Allocator.Error![]const u8 {
    var output = std.Io.Writer.Allocating.init(a);
    defer output.deinit();
    writeRepairedJson(&output.writer, bytes) catch return error.OutOfMemory;
    return output.toOwnedSlice();
}
fn writeRepairedJson(writer: *std.Io.Writer, bytes: []const u8) std.Io.Writer.Error!void {
    var in_string = false;
    var index: usize = 0;
    while (index < bytes.len) : (index += 1) {
        const byte = bytes[index];
        if (!in_string) {
            try writer.writeByte(byte);
            if (byte == '"') in_string = true;
            continue;
        }
        if (byte == '"') {
            try writer.writeByte(byte);
            in_string = false;
            continue;
        }
        if (byte == '\\') {
            if (index + 1 < bytes.len) {
                const next = bytes[index + 1];
                if (next == 'u' and bytes.len - index >= 6) {
                    const digits = bytes[index + 2 ..][0..4];
                    var valid = true;
                    for (digits) |digit| if (!std.ascii.isHex(digit)) {
                        valid = false;
                        break;
                    };
                    if (valid) {
                        try writer.writeAll(bytes[index..][0..6]);
                        index += 5;
                        continue;
                    }
                }
                if (std.mem.indexOfScalar(u8, "\"\\/bfnrt", next) != null) {
                    try writer.writeAll(bytes[index..][0..2]);
                    index += 1;
                    continue;
                }
            }
            try std.json.Stringify.encodeJsonStringChars(bytes[index..][0..1], .{}, writer);
            continue;
        }
        if (byte <= 0x1f) try std.json.Stringify.encodeJsonStringChars(bytes[index..][0..1], .{}, writer) else try writer.writeByte(byte);
    }
}
fn parseWithRepair(a: std.mem.Allocator, bytes: []const u8) !Value {
    return j.parseFromSliceLeaky(Value, a, bytes, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |original| {
        if (original == error.OutOfMemory) return original;
        const repaired = try repairJson(a, bytes);
        if (std.mem.eql(u8, repaired, bytes)) return original;
        return j.parseFromSliceLeaky(Value, a, repaired, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always });
    };
}
pub fn parseJsonWithRepair(a: std.mem.Allocator, bytes: []const u8) !Value {
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    return j.clone(a, try parseWithRepair(temporary.allocator(), bytes));
}
pub fn parseToolCallArguments(a: std.mem.Allocator, source: ?[]const u8) !Arguments {
    const bytes = source orelse return .{ .arguments = j.map(a) };
    if (@import("../utils/unicode.zig").isBlank(bytes) catch false) return .{ .arguments = j.map(a) };
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const parsed = parseWithRepair(temporary.allocator(), bytes) catch |err| {
        if (err == error.OutOfMemory) return err;
        return .{ .arguments = j.map(a), .argumentsError = try std.fmt.allocPrint(a, "{s}\n\nReceived arguments:\n{s}", .{ @errorName(err), bytes }) };
    };
    if (parsed != .object) return .{ .arguments = j.map(a), .argumentsError = try std.fmt.allocPrint(a, "Tool arguments must be a JSON object\n\nReceived arguments:\n{s}", .{bytes}) };
    return .{ .arguments = try j.clone(a, parsed) };
}
const Frame = struct { value: Value, key: ?[]const u8 = null };
fn append(a: std.mem.Allocator, frame: *Frame, value: Value) !void {
    switch (frame.value) {
        .array => |*items| try items.append(value),
        .object => |*obj| {
            const key = frame.key orelse return error.MissingKey;
            try obj.put(a, key, value);
            frame.key = null;
        },
        else => return error.InvalidContainer,
    }
}
/// Observe a numeric token through the standard scanner's streaming states.
/// A lower-case exponent can retain an already-decoded significand when the
/// input ends before exponent digits arrive. No source bytes are removed or
/// repaired; the scanner owns both the mantissa and exponent recognition.
fn unfinishedExponentSignificand(a: std.mem.Allocator, number: []const u8) !?Value {
    var scanner = std.json.Scanner.initStreaming(a);
    defer scanner.deinit();
    var tokens: std.ArrayList(u8) = .empty;
    defer tokens.deinit(a);
    var offset: usize = 0;
    var significand: ?Value = null;
    while (true) {
        const token = scanner.next() catch |err| switch (err) {
            error.BufferUnderrun => {
                if (offset == number.len) return if (scanner.state == .number_post_e or scanner.state == .number_post_e_sign) significand else null;
                // One-byte input buffers expose the scanner's transition into
                // the exponent state without finding or slicing an exponent.
                scanner.feedInput(number[offset..][0..1]);
                offset += 1;
                continue;
            },
            error.OutOfMemory => return err,
            else => return null,
        };
        switch (token) {
            .partial_number => |chunk| {
                // The state alone cannot distinguish e from E. This fragment
                // has already been classified as part of the number by std.
                if (scanner.state == .number_post_e and std.mem.eql(u8, chunk, "e")) {
                    significand = j.parseFromSliceLeaky(Value, a, tokens.items, .{ .allocate = .alloc_always }) catch |err| {
                        if (err == error.OutOfMemory) return err;
                        return null;
                    };
                }
                try tokens.appendSlice(a, chunk);
            },
            else => return null,
        }
    }
}
pub fn parseStreamingJson(a: std.mem.Allocator, bytes: ?[]const u8) !Value {
    const input = bytes orelse return j.map(a);
    if (input.len == 0) return j.map(a);
    // Pi returns a complete JSON parse unchanged. In particular, complete
    // null must not take the partial fallback's nullish-to-object branch.
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    if (parseWithRepair(scratch, input)) |parsed| {
        return j.clone(a, parsed);
    } else |err| {
        if (err == error.OutOfMemory) return err;
    }
    const value = parsePartial(scratch, input) catch |err| {
        if (err == error.OutOfMemory) return err;
        const repaired = try repairJson(scratch, input);
        const fallback = parsePartial(scratch, repaired) catch |fallback_err| {
            if (fallback_err == error.OutOfMemory) return fallback_err;
            return j.map(a);
        };
        return j.clone(a, fallback);
    };
    return j.clone(a, value);
}
fn malformedUnterminatedStringPrefix(a: std.mem.Allocator, input: []const u8) !?[]const u8 {
    var scanner = j.Scanner.initStreaming(a);
    defer scanner.deinit();
    var decoded: std.ArrayList(u8) = .empty;
    defer decoded.deinit(a);
    var offset: usize = 0;
    var last_escape_prefix: ?usize = null;
    while (true) {
        const token = scanner.next() catch |err| switch (err) {
            error.BufferUnderrun => {
                if (offset == input.len) return null;
                if (input[offset] == '\\') {
                    if (scanner.raw.state == .string) try scanner.finishPartialString(a, &decoded);
                    // A backslash inside an escape is not a valid truncation
                    // point: the preceding raw prefix still ends in an escape.
                    last_escape_prefix = if (scanner.raw.state == .string) decoded.items.len else null;
                }
                scanner.feedInput(input[offset..][0..1]);
                offset += 1;
                continue;
            },
            error.OutOfMemory => return err,
            error.SyntaxError => {
                switch (scanner.raw.state) {
                    .string_backslash, .string_backslash_u, .string_backslash_u_1, .string_backslash_u_2 => {},
                    .string_backslash_u_3 => {
                        // A consumed fourth hex digit can fail solely because
                        // std.json rejects a valid lone low surrogate. That is
                        // not a malformed escape, and is outside this repair.
                        if (scanner.cursor == scanner.input.len) return null;
                    },
                    .string => {
                        if (scanner.cursor >= scanner.input.len or scanner.input[scanner.cursor] > 0x1f) return null;
                    },
                    else => return null,
                }
                const prefix = last_escape_prefix orelse return null;
                const remainder = offset - scanner.input.len + scanner.cursor;
                // An unescaped closing quote makes this the closed-string
                // path. Any later raw backslash makes the fallback prefix
                // retain the earlier malformed escape/control character.
                if (std.mem.findAny(u8, input[remainder..], "\\\"") != null) return null;
                const units = try std.unicode.wtf8ToWtf16LeAlloc(a, decoded.items[0..prefix]);
                defer a.free(units);
                return try std.unicode.wtf16LeToWtf8Alloc(a, units);
            },
            else => return null,
        };
        switch (token) {
            .partial_string => |chunk| try decoded.appendSlice(a, chunk),
            .partial_string_escaped_1 => |chunk| try decoded.appendSlice(a, &chunk),
            .partial_string_escaped_2 => |chunk| try decoded.appendSlice(a, &chunk),
            .partial_string_escaped_3 => |chunk| try decoded.appendSlice(a, &chunk),
            .partial_string_escaped_4 => |chunk| try decoded.appendSlice(a, &chunk),
            .end_of_document => return null,
            else => {
                decoded.clearRetainingCapacity();
                last_escape_prefix = null;
            },
        }
    }
}
fn parsePartial(a: std.mem.Allocator, input: []const u8) !Value {
    // Only the fallback uses String.trim. Scanner tokens are copied into
    // caller storage below, so the normalized input need not outlive this call.
    const trimmed = @import("../utils/unicode.zig").trimWhitespace(a, input) catch |err| {
        if (err == error.OutOfMemory) return err;
        return error.MalformedPartialJson;
    };
    defer a.free(trimmed);
    if (trimmed.len == 0) return j.map(a);
    var scanner = j.Scanner.initStreaming(a);
    defer scanner.deinit();
    scanner.feedInput(trimmed);
    var frames: std.ArrayList(Frame) = .empty;
    defer frames.deinit(a);
    var token_bytes: std.ArrayList(u8) = .empty;
    defer token_bytes.deinit(a);
    var root: ?Value = null;
    var partial_kind: enum { none, string, number } = .none;
    var unfinished: ?std.json.TokenType = null;
    scan: while (true) {
        const token = scanner.next() catch |err| switch (err) {
            error.BufferUnderrun => {
                try scanner.finishPartialString(a, &token_bytes);
                unfinished = scanner.peekNextTokenType() catch |peek_err| switch (peek_err) {
                    error.BufferUnderrun => null,
                    else => {
                        if (frames.items.len == 0) return error.MalformedPartialJson;
                        partial_kind = .none;
                        token_bytes.clearRetainingCapacity();
                        break :scan;
                    },
                };
                break;
            },
            error.OutOfMemory => return err,
            error.SyntaxError => {
                if (try malformedUnterminatedStringPrefix(a, trimmed)) |prefix| {
                    defer a.free(prefix);
                    token_bytes.clearRetainingCapacity();
                    try token_bytes.appendSlice(a, prefix);
                    partial_kind = .string;
                    break :scan;
                }
                if (frames.items.len == 0) return error.MalformedPartialJson;
                partial_kind = .none;
                token_bytes.clearRetainingCapacity();
                break :scan;
            },
            else => return error.MalformedPartialJson,
        };
        var value: ?Value = null;
        switch (token) {
            .object_begin => try frames.append(a, .{ .value = j.map(a) }),
            .array_begin => try frames.append(a, .{ .value = j.array(a) }),
            .object_end, .array_end => {
                if (frames.items.len == 0) return error.MalformedPartialJson;
                value = frames.pop().?.value;
            },
            .partial_string, .partial_number => |chunk| {
                partial_kind = if (token == .partial_string) .string else .number;
                try token_bytes.appendSlice(a, chunk);
            },
            .partial_string_escaped_1 => |chunk| {
                partial_kind = .string;
                try token_bytes.appendSlice(a, &chunk);
            },
            .partial_string_escaped_2 => |chunk| {
                partial_kind = .string;
                try token_bytes.appendSlice(a, &chunk);
            },
            .partial_string_escaped_3 => |chunk| {
                partial_kind = .string;
                try token_bytes.appendSlice(a, &chunk);
            },
            .partial_string_escaped_4 => |chunk| {
                partial_kind = .string;
                try token_bytes.appendSlice(a, &chunk);
            },
            .string, .allocated_string => |chunk| {
                try token_bytes.appendSlice(a, chunk);
                const string = try a.dupe(u8, token_bytes.items);
                token_bytes.clearRetainingCapacity();
                partial_kind = .none;
                if (frames.items.len > 0 and frames.items[frames.items.len - 1].value == .object and frames.items[frames.items.len - 1].key == null) frames.items[frames.items.len - 1].key = string else value = j.str(string);
            },
            .number, .allocated_number => |chunk| {
                try token_bytes.appendSlice(a, chunk);
                value = @import("value.zig").parseFromSliceLeaky(Value, a, token_bytes.items, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |err| {
                    if (err == error.OutOfMemory) return err;
                    return error.MalformedPartialJson;
                };
                token_bytes.clearRetainingCapacity();
                partial_kind = .none;
                // A number token can precede an invalid continuation. Retain
                // it only after the standard scanner accepts its boundary.
                // A failed peek after a consumed comma belongs to the next
                // value; a failure in post_value still belongs to this one.
                const boundary: ?std.json.TokenType = scanner.peekNextTokenType() catch |err| switch (err) {
                    error.BufferUnderrun => null,
                    else => blk: {
                        if (scanner.raw.state == .post_value) {
                            if (frames.items.len == 0) return error.MalformedPartialJson;
                            break :scan;
                        }
                        break :blk null;
                    },
                };
                _ = boundary;
            },
            .true => value = .{ .bool = true },
            .false => value = .{ .bool = false },
            .null => value = .null,
            .end_of_document => break,
        }
        if (value) |v| {
            if (frames.items.len > 0) try append(a, &frames.items[frames.items.len - 1], v) else {
                root = v;
                if (v != .integer and v != .float and v != .number_string) break;
            }
        }
    }
    if (unfinished) |kind| switch (kind) {
        .true, .false, .null => {
            const value: Value = switch (kind) {
                .true => .{ .bool = true },
                .false => .{ .bool = false },
                .null => .null,
                else => unreachable,
            };
            if (frames.items.len > 0) try append(a, &frames.items[frames.items.len - 1], value) else root = value;
        },
        // No partial_string token is emitted for an opened empty string.
        // A key remains unassigned below; an empty value is retained.
        .string => partial_kind = .string,
        else => {},
    };
    if (partial_kind != .none) {
        const v: ?Value = if (partial_kind == .string) j.str(try a.dupe(u8, token_bytes.items)) else blk: {
            break :blk @import("value.zig").parseFromSliceLeaky(Value, a, token_bytes.items, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |err| {
                if (err == error.OutOfMemory) return err;
                break :blk try unfinishedExponentSignificand(a, token_bytes.items);
            };
        };
        if (v) |value| {
            if (frames.items.len > 0) {
                const frame = &frames.items[frames.items.len - 1];
                if (frame.value != .object or frame.key != null) try append(a, frame, value);
            } else root = value;
        }
    }
    while (frames.items.len > 0) {
        const value = frames.pop().?.value;
        if (frames.items.len > 0) try append(a, &frames.items[frames.items.len - 1], value) else root = value;
    }
    const result = root orelse return error.MalformedPartialJson;
    return if (result == .null) j.map(a) else result;
}
