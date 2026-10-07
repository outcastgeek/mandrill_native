//! Explicit remote-defect payload: one kind discriminator followed by UTF-8
//! diagnostic bytes. The envelope's payload bound applies before decoding.
const std = @import("std");
const outcomes = @import("../../outcome.zig");
const Protocol = @import("../rpc_error.zig").Protocol;
const RemoteDefect = @import("../rpc_error.zig").RemoteDefect;

/// Encode a value into its wire representation.
/// Parameters:
/// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
/// - `defect: outcomes.Defect`: Unexpected failure represented separately from typed errors.
/// - `max_payload_bytes: usize`: Maximum permitted encoded payload size.
/// Returns (`outcomes.Exit(outcomes.OwnedSlice(u8), Protocol)`): Carries `outcomes.OwnedSlice(u8)` on `.success` or `Protocol` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
pub fn encode(env: anytype, defect: outcomes.Defect, max_payload_bytes: usize) outcomes.Exit(outcomes.OwnedSlice(u8), Protocol) {
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
        @compileError("RPC defect wire requires R.allocator");
    if (!std.unicode.utf8ValidateSlice(defect.message)) return .{ .fail = .malformed_envelope };
    const size = std.math.add(usize, @sizeOf(u8), defect.message.len) catch return .{ .fail = .frame_too_large };
    if (size > max_payload_bytes) return .{ .fail = .frame_too_large };
    const bytes = env.allocator.alloc(u8, size) catch return .{ .die = outcomes.Defect.out_of_memory };
    bytes[0] = @backingInt(defect.kind);
    std.mem.copyForwards(u8, bytes[@sizeOf(u8)..], defect.message);
    return .{ .success = .{ .items = bytes, .allocator = env.allocator } };
}

/// Decode a value from its wire representation.
/// Parameters:
/// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
/// - `bytes: []const u8`: Borrowed input; keep its owner alive until the operation has copied or joined it.
/// Returns (`outcomes.Exit(RemoteDefect, Protocol)`): Carries `RemoteDefect` on `.success` or `Protocol` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
pub fn decode(env: anytype, bytes: []const u8) outcomes.Exit(RemoteDefect, Protocol) {
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
        @compileError("RPC defect wire requires R.allocator");
    if (bytes.len < @sizeOf(u8)) return .{ .fail = .malformed_envelope };
    const kind_info = @typeInfo(outcomes.Defect.Kind).@"enum";
    var known_kind = false;
    inline for (kind_info.field_values) |candidate| {
        if (candidate == bytes[0]) known_kind = true;
    }
    if (!known_kind) return .{ .fail = .malformed_envelope };
    const kind: outcomes.Defect.Kind = @fromBackingInt(@as(kind_info.tag_type, @intCast(bytes[0])));
    const text = bytes[@sizeOf(u8)..];
    if (!std.unicode.utf8ValidateSlice(text)) return .{ .fail = .malformed_envelope };
    const copied = env.allocator.dupe(u8, text) catch return .{ .die = outcomes.Defect.out_of_memory };
    return .{ .success = .{ .kind = kind, .diagnostic = .{ .items = copied, .allocator = env.allocator } } };
}
