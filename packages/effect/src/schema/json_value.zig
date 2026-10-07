//! Owning JSON document used by alternatives and representation-preserving codecs.
const std = @import("std");
const outcomes = @import("../outcome.zig");
/// Independent deep-copied JSON tree. Its arena owns all keys, tokens, strings and containers; deinit invalidates views.
pub const Owned = struct {
    /// Exit/Result cleanup owns this value; release it once or clone it for an independent observation.
    pub const effect_owned_payload = true;
    /// Root borrowed through this owner’s arena lifetime.
    value: std.json.Value,
    /// Owns every descendant allocation, including keys and lexical number bytes.
    arena: *std.heap.ArenaAllocator,
    /// Originating allocator for destroying the arena itself.
    allocator: std.mem.Allocator,
    /// Deep-copy a borrowed DOM using env.allocator for the arena and its storage; release the returned owner once.
    pub fn copy(env: anytype, input: *const std.json.Value) error{OutOfMemory}!Owned {
        const arena = try env.allocator.create(std.heap.ArenaAllocator);
        arena.* = .init(env.allocator);
        errdefer {
            arena.deinit();
            env.allocator.destroy(arena);
        }
        return .{ .value = try copyValue(arena.allocator(), input.*), .arena = arena, .allocator = env.allocator };
    }
    /// Destroy the document arena and its allocation with the originating allocator.
    pub fn deinit(self: *Owned) void {
        self.arena.deinit();
        self.allocator.destroy(self.arena);
        self.* = undefined;
    }
    /// Deep-copy this document into a new independent arena; the source survives allocation failure.
    pub fn clone(self: *const Owned) error{OutOfMemory}!Owned {
        const Env = struct { allocator: std.mem.Allocator };
        return copy(&Env{ .allocator = self.allocator }, &self.value);
    }
};
/// Deep-copy JSON keys, strings/tokens and containers into an existing arena/scratch allocator.
/// This returns no owner; the caller owns that allocation scope and releases partial allocations on failure.
pub fn copyValue(allocator: std.mem.Allocator, input: std.json.Value) error{OutOfMemory}!std.json.Value {
    return switch (input) {
        .string => |text| .{ .string = try allocator.dupe(u8, text) },
        .number_string => |text| .{ .number_string = try allocator.dupe(u8, text) },
        .array => |array| blk: {
            var result = std.json.Array.init(allocator);
            for (array.items) |item| try result.append(try copyValue(allocator, item));
            break :blk .{ .array = result };
        },
        .object => |object| blk: {
            var result: std.json.ObjectMap = .{};
            var iterator = object.iterator();
            while (iterator.next()) |item| try result.put(allocator, try allocator.dupe(u8, item.key_ptr.*), try copyValue(allocator, item.value_ptr.*));
            break :blk .{ .object = result };
        },
        else => input,
    };
}
/// Mathematical JSON equality; exact lexical numbers compare by value and
/// nonfinite native float input is never considered equal.
pub fn equal(left: *const std.json.Value, right: *const std.json.Value) bool {
    const number = @import("json_number_codec.zig");
    var left_buffer: [std.fmt.count("{d}", .{std.math.minInt(i64)}) + std.fmt.count("{e}", .{-std.math.floatMax(f64)}) + std.fmt.count("{e}", .{std.math.floatMin(f64)})]u8 = undefined;
    var right_buffer: [std.fmt.count("{d}", .{std.math.minInt(i64)}) + std.fmt.count("{e}", .{-std.math.floatMax(f64)}) + std.fmt.count("{e}", .{std.math.floatMin(f64)})]u8 = undefined;
    const left_number: ?[]const u8 = switch (left.*) {
        .number_string => |text| text,
        .integer => |value| std.fmt.bufPrint(&left_buffer, "{d}", .{value}) catch unreachable,
        .float => |value| if (std.math.isFinite(value)) std.fmt.bufPrint(&left_buffer, "{e}", .{value}) catch null else null,
        else => null,
    };
    const right_number: ?[]const u8 = switch (right.*) {
        .number_string => |text| text,
        .integer => |value| std.fmt.bufPrint(&right_buffer, "{d}", .{value}) catch unreachable,
        .float => |value| if (std.math.isFinite(value)) std.fmt.bufPrint(&right_buffer, "{e}", .{value}) catch null else null,
        else => null,
    };
    if (left_number != null and right_number != null) {
        const a = number.Number.parse(left_number.?) catch return false;
        const b = number.Number.parse(right_number.?) catch return false;
        return a.order(b) == .eq;
    }
    if (std.meta.activeTag(left.*) != std.meta.activeTag(right.*)) return false;
    return switch (left.*) {
        .null => true,
        .bool => |value| value == right.bool,
        .string => |value| std.mem.eql(u8, value, right.string),
        .float => |value| std.math.isFinite(value) and std.math.isFinite(right.float) and value == right.float,
        .integer, .number_string => false,
        .array => |value| blk: {
            if (value.items.len != right.array.items.len) break :blk false;
            for (value.items, right.array.items) |*a, *b| if (!equal(a, b)) break :blk false;
            break :blk true;
        },
        .object => |value| blk: {
            if (value.count() != right.object.count()) break :blk false;
            var iterator = value.iterator();
            while (iterator.next()) |item| {
                const other = right.object.get(item.key_ptr.*) orelse break :blk false;
                if (!equal(item.value_ptr, &other)) break :blk false;
            }
            break :blk true;
        },
    };
}

/// Rebase borrowed decoded string views when cloning their document owner.
/// Marked owning values use their own clone contract; external service views
/// retain the same explicit caller lifetime.
pub fn cloneDecoded(value: anytype, source: *const std.json.Value, target: *const std.json.Value) error{OutOfMemory}!@TypeOf(value) {
    const T = @TypeOf(value);
    if (comptime outcomes.ownsPayload(T)) return outcomes.clonePayload(value);
    return switch (@typeInfo(T)) {
        .pointer => |info| if (info.size == .slice and info.child == u8 and info.attrs.@"const") rebaseString(value, source, target) else value,
        .optional => if (value) |present| try cloneDecoded(present, source, target) else null,
        .array => blk: {
            var result: T = undefined;
            for (value, 0..) |item, index| result[index] = try cloneDecoded(item, source, target);
            break :blk result;
        },
        .@"struct" => |shape| blk: {
            var result = value;
            inline for (shape.field_names) |name| @field(result, name) = try cloneDecoded(@field(value, name), source, target);
            break :blk result;
        },
        .@"union" => switch (value) {
            inline else => |item, tag| @unionInit(T, @tagName(tag), try cloneDecoded(item, source, target)),
        },
        else => value,
    };
}
fn rebaseString(value: []const u8, source: *const std.json.Value, target: *const std.json.Value) []const u8 {
    if (source.* == .string or source.* == .number_string) {
        const text = if (source.* == .string) source.string else source.number_string;
        const other = if (target.* == .string) target.string else target.number_string;
        const start = @intFromPtr(text.ptr);
        const address = @intFromPtr(value.ptr);
        if (address >= start and address - start <= text.len and value.len <= text.len - (address - start)) return other[address - start ..][0..value.len];
    } else if (source.* == .array) {
        for (source.array.items, target.array.items) |*left, *right| {
            const candidate = rebaseString(value, left, right);
            if (candidate.ptr != value.ptr) return candidate;
        }
    } else if (source.* == .object) {
        var iterator = source.object.iterator();
        while (iterator.next()) |entry| if (target.object.getPtr(entry.key_ptr.*)) |other| {
            const candidate = rebaseString(value, entry.value_ptr, other);
            if (candidate.ptr != value.ptr) return candidate;
        };
    }
    return value;
}
