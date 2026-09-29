//! Explicit remote-defect payload: one kind discriminator followed by UTF-8
//! diagnostic bytes. The envelope's payload bound applies before decoding.
const std = @import("std");
const outcomes = @import("../../outcome.zig");
const Protocol = @import("../rpc_error.zig").Protocol;
const RemoteDefect = @import("../rpc_error.zig").RemoteDefect;

pub fn encode(env: anytype, defect: outcomes.Defect, max_payload_bytes: usize) outcomes.Exit(outcomes.OwnedSlice(u8), Protocol) {
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
        @compileError("RPC defect wire requires R.allocator");
    if (!std.unicode.utf8ValidateSlice(defect.message)) return .{ .fail = .malformed_envelope };
    const size = std.math.add(usize, @sizeOf(u8), defect.message.len) catch return .{ .fail = .frame_too_large };
    if (size > max_payload_bytes) return .{ .fail = .frame_too_large };
    const bytes = env.allocator.alloc(u8, size) catch return .{ .die = outcomes.Defect.out_of_memory };
    bytes[0] = @intFromEnum(defect.kind);
    std.mem.copyForwards(u8, bytes[@sizeOf(u8)..], defect.message);
    return .{ .success = .{ .items = bytes, .allocator = env.allocator } };
}

pub fn decode(env: anytype, bytes: []const u8) outcomes.Exit(RemoteDefect, Protocol) {
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
        @compileError("RPC defect wire requires R.allocator");
    if (bytes.len < @sizeOf(u8)) return .{ .fail = .malformed_envelope };
    const kind = std.meta.intToEnum(outcomes.Defect.Kind, bytes[0]) catch return .{ .fail = .malformed_envelope };
    const text = bytes[@sizeOf(u8)..];
    if (!std.unicode.utf8ValidateSlice(text)) return .{ .fail = .malformed_envelope };
    const copied = env.allocator.dupe(u8, text) catch return .{ .die = outcomes.Defect.out_of_memory };
    return .{ .success = .{ .kind = kind, .diagnostic = .{ .items = copied, .allocator = env.allocator } } };
}
