const std = @import("std");

pub const error_sentinel: usize = std.math.maxInt(usize);

pub const StringView = extern struct {
    data: ?[*]const u8,
    length: usize,

    pub fn fromSlice(value: []const u8) StringView {
        return initStringView(StringView, value);
    }

    pub fn slice(self: StringView) ![]const u8 {
        if (self.length == 0) return "";
        const data = self.data orelse return error.NullData;
        return data[0..self.length];
    }
};

pub fn BorrowedSlice(comptime T: type) type {
    return extern struct {
        data: ?[*]const T,
        length: usize,

        const Self = @This();

        pub fn fromSlice(value: []const T) Self {
            return .{ .data = if (value.len == 0) null else value.ptr, .length = value.len };
        }

        pub fn slice(self: Self) ![]const T {
            if (self.length == 0) return &.{};
            const data = self.data orelse return error.NullData;
            return data[0..self.length];
        }
    };
}

pub fn MutableSlice(comptime T: type) type {
    return extern struct {
        data: ?[*]T,
        length: usize,
        capacity: usize,

        pub fn slice(self: @This()) ![]T {
            if (self.length > self.capacity) return error.LengthExceedsCapacity;
            if (self.capacity == 0) return &.{};
            const data = self.data orelse return error.NullData;
            return data[0..self.length];
        }

        pub fn capacitySlice(self: @This()) ![]T {
            if (self.capacity == 0) return &.{};
            const data = self.data orelse return error.NullData;
            return data[0..self.capacity];
        }
    };
}

pub fn borrow(comptime T: type, data: ?[*]const T, length: usize) ![]const T {
    return (BorrowedSlice(T){ .data = data, .length = length }).slice();
}

pub fn borrowC(comptime T: type, data: [*c]const T, length: usize) ![]const T {
    return borrow(T, if (data == null) null else @ptrCast(data), length);
}

pub fn borrowView(comptime T: type, view: anytype) ![]const T {
    return borrowC(T, view.data, view.length);
}

pub fn validBorrowed(value: anytype) bool {
    return value.length == 0 or value.data != null;
}

pub fn validMutable(value: anytype) bool {
    return value.capacity == 0 or value.data != null;
}

pub fn copyBytes(output: anytype, value: []const u8) !void {
    if (!validMutable(output.*)) return error.NullData;
    if (value.len > output.capacity) return error.InsufficientCapacity;
    if (value.len != 0) @memcpy(output.data[0..value.len], value);
    output.length = value.len;
}

pub fn initView(comptime View: type, value: anytype) View {
    return .{ .data = if (value.len == 0) null else value.ptr, .length = value.len };
}

pub fn initMutableView(comptime View: type, value: anytype) View {
    return .{ .data = if (value.len == 0) null else value.ptr, .length = 0, .capacity = value.len };
}

pub fn pointerOrNull(comptime T: type, values: []const T) ?[*]const T {
    return if (values.len == 0) null else values.ptr;
}

pub fn mutablePointerOrNull(comptime T: type, values: []T) ?[*]T {
    return if (values.len == 0) null else values.ptr;
}

pub fn usedBytes(value: anytype) ![]const u8 {
    if (value.length > value.capacity) return error.LengthExceedsCapacity;
    if (value.length == 0) return &.{};
    const data = value.data orelse return error.NullData;
    return data[0..value.length];
}

pub fn initStringView(comptime View: type, value: []const u8) View {
    return .{ .data = if (value.len == 0) null else value.ptr, .length = value.len };
}

pub fn compatible(actual_version: u32, expected_version: u32, actual_size: u32, expected_size: usize) bool {
    return actual_version == expected_version and actual_size >= expected_size;
}

pub fn resultValue(comptime Result: type, status: u32, value: anytype) Result {
    return .{ .status = status, .reserved = 0, .value = value };
}

pub fn emptyResult(comptime Result: type, comptime Value: type, status: u32) Result {
    return resultValue(Result, status, std.mem.zeroes(Value));
}

pub fn initAbiValue(comptime T: type, version: u32) T {
    var value = std.mem.zeroes(T);
    value.abi_version = version;
    value.struct_size = @sizeOf(T);
    return value;
}

test "generic borrowed and mutable views preserve arbitrary element types" {
    const Item = extern struct { id: u32, weight: f64 };
    var values = [_]Item{ .{ .id = 1, .weight = 1.5 }, .{ .id = 2, .weight = 2.5 } };
    const borrowed = BorrowedSlice(Item).fromSlice(&values);
    try std.testing.expectEqual(@as(u32, 2), (try borrowed.slice())[1].id);
    const mutable = MutableSlice(Item){ .data = &values, .length = values.len, .capacity = values.len };
    (try mutable.slice())[0].id = 9;
    try std.testing.expectEqual(@as(u32, 9), values[0].id);
}

test "ABI values and typed results initialize generically" {
    const Value = extern struct { abi_version: u32, struct_size: u32, count: u32 };
    const Result = extern struct { status: u32, reserved: u32, value: Value };
    const wrapped = resultValue(Result, 3, initAbiValue(Value, 7));
    try std.testing.expectEqual(@as(u32, 3), wrapped.status);
    try std.testing.expectEqual(@as(u32, 7), wrapped.value.abi_version);
}
