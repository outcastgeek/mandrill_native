//! Canonical RFC 4648 base64 with padding. Encoded text and decoded bytes are
//! independently owned by the allocator selected through each direction's R.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const effects = @import("../effect.zig");
const Issue = @import("schema_issue.zig").Issue;
const Codec = @import("codec.zig").Codec;

const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const input_group_bytes: usize = 3;
const output_group_bytes: usize = 4;
const sextet_bits: u4 = 6;

pub const Base64Text = struct {
    pub const effect_owned_payload = true;
    bytes: outcomes.OwnedSlice(u8),

    pub fn slice(self: *const @This()) []const u8 {
        return self.bytes.items;
    }

    pub fn deinit(self: *@This()) void {
        self.bytes.deinit();
        self.* = undefined;
    }

    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return .{ .bytes = try self.bytes.clone() };
    }
};

fn valueOf(character: u8) ?u8 {
    for (alphabet, 0..) |candidate, index| {
        if (candidate == character) return @intCast(index);
    }
    return null;
}

fn encode(allocator: std.mem.Allocator, input: []const u8) outcomes.Exit(Base64Text, Issue) {
    const rounded = std.math.add(usize, input.len, input_group_bytes - 1) catch return .{ .fail = Issue.pure(.encode, .size_limit) };
    const groups = rounded / input_group_bytes;
    const output_size = std.math.mul(usize, groups, output_group_bytes) catch return .{ .fail = Issue.pure(.encode, .size_limit) };
    const output = allocator.alloc(u8, output_size) catch return .{ .die = outcomes.Defect.out_of_memory };
    var source: usize = 0;
    var target: usize = 0;
    while (source < input.len) {
        const remaining = input.len - source;
        const first = input[source];
        const second: u8 = if (remaining > 1) input[source + 1] else 0;
        const third: u8 = if (remaining > 2) input[source + 2] else 0;
        output[target] = alphabet[first >> 2];
        output[target + 1] = alphabet[((first & 0b11) << 4) | (second >> 4)];
        output[target + 2] = if (remaining > 1) alphabet[((second & 0b1111) << 2) | (third >> 6)] else '=';
        output[target + 3] = if (remaining > 2) alphabet[third & 0b111111] else '=';
        source += @min(remaining, input_group_bytes);
        target += output_group_bytes;
    }
    return .{ .success = .{ .bytes = .{ .items = output, .allocator = allocator } } };
}

fn decode(allocator: std.mem.Allocator, input: []const u8) outcomes.Exit(outcomes.OwnedSlice(u8), Issue) {
    if (input.len % output_group_bytes != 0) return .{ .fail = Issue.pure(.decode, .invalid_value) };
    const groups = input.len / output_group_bytes;
    const padded = if (input.len == 0) @as(usize, 0) else @as(usize, @intFromBool(input[input.len - 1] == '=')) +
        @as(usize, @intFromBool(input.len > 1 and input[input.len - 2] == '='));
    const full_size = std.math.mul(usize, groups, input_group_bytes) catch return .{ .fail = Issue.pure(.decode, .size_limit) };
    if (padded > full_size) return .{ .fail = Issue.pure(.decode, .invalid_value) };
    const output_size = full_size - padded;
    const output = allocator.alloc(u8, output_size) catch return .{ .die = outcomes.Defect.out_of_memory };
    var keep = false;
    defer if (!keep) allocator.free(output);
    var target: usize = 0;
    for (0..groups) |group| {
        const offset = group * output_group_bytes;
        const last = group + 1 == groups;
        const a = valueOf(input[offset]) orelse return .{ .fail = Issue.pure(.decode, .invalid_value) };
        const b = valueOf(input[offset + 1]) orelse return .{ .fail = Issue.pure(.decode, .invalid_value) };
        const c: u8 = if (input[offset + 2] == '=' and last) 0 else valueOf(input[offset + 2]) orelse return .{ .fail = Issue.pure(.decode, .invalid_value) };
        const d: u8 = if (input[offset + 3] == '=' and last) 0 else valueOf(input[offset + 3]) orelse return .{ .fail = Issue.pure(.decode, .invalid_value) };
        if (input[offset + 2] == '=' and input[offset + 3] != '=') return .{ .fail = Issue.pure(.decode, .invalid_value) };
        if (last and padded == 2 and (b & 0b1111) != 0) return .{ .fail = Issue.pure(.decode, .invalid_value) };
        if (last and padded == 1 and (c & 0b11) != 0) return .{ .fail = Issue.pure(.decode, .invalid_value) };
        output[target] = @truncate((@as(u16, a) << sextet_bits) >> 4 | (@as(u16, b) >> 4));
        target += 1;
        if (target < output_size) {
            output[target] = @truncate((@as(u16, b) << 4) | (@as(u16, c) >> 2));
            target += 1;
        }
        if (target < output_size) {
            output[target] = @truncate((@as(u16, c) << sextet_bits) | @as(u16, d));
            target += 1;
        }
    }
    keep = true;
    return .{ .success = .{ .items = output, .allocator = allocator } };
}

pub fn codec(comptime DecodeR: type, comptime EncodeR: type) Codec(outcomes.OwnedSlice(u8), Base64Text, DecodeR, EncodeR) {
    const Bytes = outcomes.OwnedSlice(u8);
    return Codec(Bytes, Base64Text, DecodeR, EncodeR).init(struct {
        fn run(encoded: *const Base64Text) effects.Effect(Bytes, Issue, DecodeR) {
            return effects.Effect(Bytes, Issue, DecodeR).fromBorrowed(Base64Text, encoded, struct {
                fn read(source: *const Base64Text, env: *const DecodeR) outcomes.Exit(Bytes, Issue) {
                    return decode(env.allocator, source.slice());
                }
            }.read);
        }
    }.run, struct {
        fn run(value: *const Bytes) effects.Effect(Base64Text, Issue, EncodeR) {
            return effects.Effect(Base64Text, Issue, EncodeR).fromBorrowed(Bytes, value, struct {
                fn write(source: *const Bytes, env: *const EncodeR) outcomes.Exit(Base64Text, Issue) {
                    return encode(env.allocator, source.items);
                }
            }.write);
        }
    }.run);
}
