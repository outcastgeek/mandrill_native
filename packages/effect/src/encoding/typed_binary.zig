//! Schema-derived binary payloads for explicitly supported Zig value shapes.
//! The comptime type is the representation schema: fields use declaration
//! order; scalars are fixed-width big endian; optionals carry a presence byte;
//! slices carry a u32 count; enum tags use u32; arrays have their type-level
//! fixed count. A
//! caller-supplied stable schema name identifies that exact shape on the wire.
//! Changing a field type/order requires a new name or a caller migration.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const Issue = @import("../schema/schema_issue.zig").Issue;
const frame = @import("schema_binary.zig");

/// Count uses `u32` as its stable representation in this module.
pub const Count = u32;
/// Explicit limits for frame bytes, collection items, and nesting depth.
pub const Limits = struct {
    frame: frame.Limits,
    max_collection_items: usize,
    max_depth: usize,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Limits`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` when frame limits validate and collection/depth bounds are positive.
    pub fn valid(self: Limits) bool {
        return self.frame.valid() and self.max_collection_items > 0 and self.max_depth > 0;
    }
};

fn supported(comptime T: type) bool {
    return switch (@typeInfo(T)) {
        .bool, .void => true,
        .int => |info| info.bits == 8 or info.bits == 16 or info.bits == 32 or
            info.bits == 64 or info.bits == 128,
        .float => |info| info.bits == 16 or info.bits == 32 or info.bits == 64 or info.bits == 128,
        .@"enum" => |info| blk: {
            inline for (info.fields) |field| {
                if (field.value < 0 or field.value > std.math.maxInt(u32)) break :blk false;
            }
            break :blk true;
        },
        .optional => |info| supported(info.child),
        .array => |info| supported(info.child),
        .pointer => |info| info.size == .slice and info.is_const and supported(info.child),
        .@"struct" => |info| blk: {
            inline for (info.fields) |field| if (field.is_comptime or !supported(field.type)) break :blk false;
            break :blk true;
        },
        .@"union" => |info| blk: {
            if (info.tag_type == null) break :blk false;
            inline for (info.fields) |field| if (!supported(field.type)) break :blk false;
            break :blk true;
        },
        else => false,
    };
}

fn appendByte(out: *std.array_list.Managed(u8), byte: u8, max: usize) error{ OutOfMemory, Limit }!void {
    if (out.items.len >= max) return error.Limit;
    try out.append(byte);
}

fn appendUnsigned(out: *std.array_list.Managed(u8), value: anytype, max: usize) error{ OutOfMemory, Limit }!void {
    const U = @TypeOf(value);
    const width = @sizeOf(U);
    const next = std.math.add(usize, out.items.len, width) catch return error.Limit;
    if (next > max) return error.Limit;
    var index: usize = width;
    while (index > 0) {
        index -= 1;
        try out.append(@truncate(value >> @intCast(index * @bitSizeOf(u8))));
    }
}

fn encodeValue(comptime T: type, value: T, out: *std.array_list.Managed(u8), limits: Limits, depth: usize) error{ OutOfMemory, Limit, InvalidValue }!void {
    if (depth >= limits.max_depth) return error.Limit;
    switch (@typeInfo(T)) {
        .void => {},
        .bool => try appendByte(out, if (value) 1 else 0, limits.frame.max_payload_bytes),
        .int => |info| {
            const U = std.meta.Int(.unsigned, info.bits);
            const bits: U = if (info.signedness == .signed) @bitCast(value) else value;
            try appendUnsigned(out, bits, limits.frame.max_payload_bytes);
        },
        .float => |info| {
            if (!std.math.isFinite(value)) return error.InvalidValue;
            const U = std.meta.Int(.unsigned, info.bits);
            try appendUnsigned(out, @as(U, @bitCast(value)), limits.frame.max_payload_bytes);
        },
        .@"enum" => try appendUnsigned(out, @as(u32, @intCast(@intFromEnum(value))), limits.frame.max_payload_bytes),
        .optional => |info| {
            if (value) |present| {
                try appendByte(out, 1, limits.frame.max_payload_bytes);
                try encodeValue(info.child, present, out, limits, depth + 1);
            } else try appendByte(out, 0, limits.frame.max_payload_bytes);
        },
        .array => |info| {
            if (info.len > limits.max_collection_items) return error.Limit;
            for (value) |item| try encodeValue(info.child, item, out, limits, depth + 1);
        },
        .pointer => |info| {
            if (value.len > limits.max_collection_items or value.len > std.math.maxInt(Count)) return error.Limit;
            try appendUnsigned(out, @as(Count, @intCast(value.len)), limits.frame.max_payload_bytes);
            for (value) |item| try encodeValue(info.child, item, out, limits, depth + 1);
        },
        .@"struct" => |info| inline for (info.fields) |field|
            try encodeValue(field.type, @field(value, field.name), out, limits, depth + 1),
        .@"union" => |info| {
            try encodeValue(info.tag_type.?, std.meta.activeTag(value), out, limits, depth + 1);
            switch (value) {
                inline else => |payload| try encodeValue(@TypeOf(payload), payload, out, limits, depth + 1),
            }
        },
        else => unreachable,
    }
}

const Cursor = struct {
    bytes: []const u8,
    index: usize = 0,

    fn take(self: *Cursor, count: usize) error{Invalid}![]const u8 {
        const end = std.math.add(usize, self.index, count) catch return error.Invalid;
        if (end > self.bytes.len) return error.Invalid;
        const view = self.bytes[self.index..end];
        self.index = end;
        return view;
    }
};

fn readUnsigned(comptime U: type, cursor: *Cursor) error{Invalid}!U {
    const bytes = try cursor.take(@sizeOf(U));
    var value: U = bytes[0];
    if (comptime @sizeOf(U) > @sizeOf(u8)) {
        for (bytes[1..]) |byte| value = (value << @bitSizeOf(u8)) | byte;
    }
    return value;
}

fn decodeValue(comptime T: type, allocator: std.mem.Allocator, cursor: *Cursor, limits: Limits, depth: usize) error{ Invalid, OutOfMemory }!T {
    if (depth >= limits.max_depth) return error.Invalid;
    return switch (@typeInfo(T)) {
        .void => {},
        .bool => blk: {
            const byte = (try cursor.take(@sizeOf(u8)))[0];
            if (byte > 1) return error.Invalid;
            break :blk byte == 1;
        },
        .int => |info| blk: {
            const U = std.meta.Int(.unsigned, info.bits);
            const bits = try readUnsigned(U, cursor);
            break :blk if (info.signedness == .signed) @as(T, @bitCast(bits)) else @as(T, bits);
        },
        .float => |info| blk: {
            const value: T = @bitCast(try readUnsigned(std.meta.Int(.unsigned, info.bits), cursor));
            if (!std.math.isFinite(value)) return error.Invalid;
            break :blk value;
        },
        .@"enum" => |info| blk: {
            const tag = try readUnsigned(u32, cursor);
            inline for (info.fields) |field| {
                if (tag == field.value) break :blk @enumFromInt(tag);
            }
            return error.Invalid;
        },
        .optional => |info| blk: {
            const marker = (try cursor.take(@sizeOf(u8)))[0];
            if (marker == 0) break :blk null;
            if (marker != 1) return error.Invalid;
            break :blk try decodeValue(info.child, allocator, cursor, limits, depth + 1);
        },
        .array => |info| blk: {
            if (info.len > limits.max_collection_items) return error.Invalid;
            var result: T = undefined;
            for (&result) |*slot| slot.* = try decodeValue(info.child, allocator, cursor, limits, depth + 1);
            break :blk result;
        },
        .pointer => |info| blk: {
            const count: usize = try readUnsigned(Count, cursor);
            if (count > limits.max_collection_items) return error.Invalid;
            const items = try allocator.alloc(info.child, count);
            for (items) |*slot| slot.* = try decodeValue(info.child, allocator, cursor, limits, depth + 1);
            break :blk items;
        },
        .@"struct" => |info| blk: {
            var result: T = undefined;
            inline for (info.fields) |field| @field(result, field.name) =
                try decodeValue(field.type, allocator, cursor, limits, depth + 1);
            break :blk result;
        },
        .@"union" => |info| blk: {
            const tag = try decodeValue(info.tag_type.?, allocator, cursor, limits, depth + 1);
            inline for (info.fields) |field| {
                if (tag == @field(info.tag_type.?, field.name)) {
                    const payload = try decodeValue(field.type, allocator, cursor, limits, depth + 1);
                    break :blk @unionInit(T, field.name, payload);
                }
            }
            return error.Invalid;
        },
        else => unreachable,
    };
}

/// All variable-size fields borrow this arena, which is released with the
/// decoded value. Move the owner as a unit; do not retain fields after deinit.
///
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// Returns (`type`): decoded value owner retaining the arena for variable-size fields.
pub fn Decoded(comptime T: type) type {
    return struct {
        const Self = @This();
        /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit`
        /// and must be released after use.
        pub const effect_owned_payload = true;
        value: T,
        arena: *std.heap.ArenaAllocator,
        allocator: std.mem.Allocator,

        /// Release resources owned by this value.
        /// Parameters:
        /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
        /// Returns (`void`): no value.
        pub fn deinit(self: *Self) void {
            self.arena.deinit();
            self.allocator.destroy(self.arena);
            self.* = undefined;
        }

        /// Duplicate this value; marked owning payloads are cloned, while ordinary
        /// borrowed fields retain their original lifetime.
        /// Parameters:
        /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
        /// Returns (`error{OutOfMemory}!Self`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
        pub fn clone(self: *const Self) error{OutOfMemory}!Self {
            const target = try self.allocator.create(std.heap.ArenaAllocator);
            target.* = std.heap.ArenaAllocator.init(self.allocator);
            errdefer {
                target.deinit();
                self.allocator.destroy(target);
            }
            return .{ .value = try cloneValue(T, self.value, target.allocator()), .arena = target, .allocator = self.allocator };
        }
    };
}

fn cloneValue(comptime T: type, value: T, allocator: std.mem.Allocator) error{OutOfMemory}!T {
    return switch (@typeInfo(T)) {
        .void, .bool, .int, .float, .@"enum" => value,
        .optional => |info| if (value) |item| try cloneValue(info.child, item, allocator) else null,
        .array => |info| blk: {
            var result: T = undefined;
            for (value, 0..) |item, index| result[index] = try cloneValue(info.child, item, allocator);
            break :blk result;
        },
        .pointer => |info| blk: {
            const items = try allocator.alloc(info.child, value.len);
            for (value, items) |item, *slot| slot.* = try cloneValue(info.child, item, allocator);
            break :blk items;
        },
        .@"struct" => |info| blk: {
            var result: T = undefined;
            inline for (info.fields) |field| @field(result, field.name) =
                try cloneValue(field.type, @field(value, field.name), allocator);
            break :blk result;
        },
        .@"union" => switch (value) {
            inline else => |payload, tag| @unionInit(T, @tagName(tag), try cloneValue(@TypeOf(payload), payload, allocator)),
        },
        else => unreachable,
    };
}

/// Define a typed codec with its compile-time value and representation types.
/// Parameters:
/// - `T: comptime type`: Value type stored or transported by the returned component.
/// - `schema_name: comptime []const u8`: Compile-time typed codec/schema fixing the corresponding decoded and encoded
///   value types.
/// Returns (`type`): binary codec derived from the compile-time Zig value shape and explicit limits.
pub fn Codec(comptime T: type, comptime schema_name: []const u8) type {
    comptime {
        if (!supported(T)) @compileError("binary schema supports only scalar, enum, optional, array, immutable slice, struct and tagged-union values");
        if (schema_name.len == 0 or !std.unicode.utf8ValidateSlice(schema_name))
            @compileError("binary schema name must be nonempty UTF-8");
    }
    return struct {
        /// Encode a value into its wire representation.
        /// Parameters:
        /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `value: *const T`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// - `limits: Limits`: Caller-selected, validated behavior or resource bounds; no hidden default
        ///   is installed.
        /// Returns (`outcomes.Exit(outcomes.OwnedSlice(u8), Issue)`): Carries `outcomes.OwnedSlice(u8)` on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn encode(env: anytype, value: *const T, limits: Limits) outcomes.Exit(outcomes.OwnedSlice(u8), Issue) {
            if (!limits.valid()) return .{ .fail = Issue.init(env, .definition, .size_limit) };
            var payload = std.array_list.Managed(u8).init(env.allocator);
            defer payload.deinit();
            encodeValue(T, value.*, &payload, limits, 0) catch |problem| return switch (problem) {
                error.OutOfMemory => .{ .die = outcomes.Defect.out_of_memory },
                error.Limit => .{ .fail = Issue.init(env, .encode, .size_limit) },
                error.InvalidValue => .{ .fail = Issue.init(env, .encode, .invalid_number) },
            };
            return frame.encode(env, schema_name, payload.items, limits.frame);
        }

        /// Decode a value from its wire representation.
        /// Parameters:
        /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected
        ///   capabilities.
        /// - `bytes: []const u8`: Borrowed input; keep its owner alive until the operation has copied or
        ///   joined it.
        /// - `limits: Limits`: Caller-selected, validated behavior or resource bounds; no hidden default
        ///   is installed.
        /// Returns (`outcomes.Exit(Decoded(T), Issue)`): Carries `Decoded(T)` on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
        pub fn decode(env: anytype, bytes: []const u8, limits: Limits) outcomes.Exit(Decoded(T), Issue) {
            if (!limits.valid()) return .{ .fail = Issue.init(env, .definition, .size_limit) };
            const view = switch (frame.inspect(bytes, limits.frame)) {
                .ok => |value| value,
                .err => |problem| return .{ .fail = problem },
            };
            if (!std.mem.eql(u8, view.schema_name, schema_name))
                return .{ .fail = Issue.init(env, .decode, .incompatible_version) };
            const arena = env.allocator.create(std.heap.ArenaAllocator) catch return .{ .die = outcomes.Defect.out_of_memory };
            arena.* = std.heap.ArenaAllocator.init(env.allocator);
            var keep = false;
            defer if (!keep) {
                arena.deinit();
                env.allocator.destroy(arena);
            };
            var cursor: Cursor = .{ .bytes = view.payload };
            const decoded = decodeValue(T, arena.allocator(), &cursor, limits, 0) catch |problem| return switch (problem) {
                error.OutOfMemory => .{ .die = outcomes.Defect.out_of_memory },
                error.Invalid => .{ .fail = Issue.init(env, .decode, .invalid_value) },
            };
            if (cursor.index != view.payload.len) return .{ .fail = Issue.init(env, .decode, .invalid_value) };
            keep = true;
            return .{ .success = .{ .value = decoded, .arena = arena, .allocator = env.allocator } };
        }
    };
}
