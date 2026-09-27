//! Lean embedding and consumer-defined value conversion through its C ABI.

const std = @import("std");

pub const Object = opaque {};
pub const ModuleInitializer = *const fn (builtin: u8) callconv(.c) *Object;

const Metadata = packed struct(u32) {
    allocation_size: u16,
    other: u8,
    tag: u8,
};
const Header = extern struct { rc: c_int, metadata: Metadata };
pub extern fn lean_initialize_runtime_module() callconv(.c) void;
extern fn lean_initialize_thread() callconv(.c) void;
extern fn lean_finalize_thread() callconv(.c) void;
pub extern fn lean_io_mark_end_initialization() callconv(.c) void;
pub extern fn lean_dec_ref_cold(object: *Object) callconv(.c) void;
pub extern fn lean_alloc_object(size: usize) callconv(.c) ?*Object;
pub extern fn lean_mk_string_from_bytes(data: [*]const u8, size: usize) callconv(.c) *Object;

var runtime_mutex: std.atomic.Mutex = .unlocked;
var runtime_initialized = false;

pub const Initialization = struct {
    mutex: std.atomic.Mutex = .unlocked,
    initialized: bool = false,

    /// Initializes one consumer-selected Lean module exactly once. Consumers
    /// keep one instance per initializer, so the helper supports any number of
    /// independently composed Lean artifacts.
    pub fn initialize(self: *Initialization, initializer: ModuleInitializer) error{ModuleInitializationFailed}!void {
        lock(&self.mutex);
        defer self.mutex.unlock();
        if (self.initialized) return;
        initializeProcessRuntime();
        const result = initializer(1);
        defer release(result);
        if (!isScalar(result) and objectTag(result) == 1) return error.ModuleInitializationFailed;
        lean_io_mark_end_initialization();
        self.initialized = true;
    }
};

fn initializeProcessRuntime() void {
    lock(&runtime_mutex);
    defer runtime_mutex.unlock();
    if (runtime_initialized) return;
    lean_initialize_runtime_module();
    runtime_initialized = true;
}

fn lock(mutex: *std.atomic.Mutex) void {
    while (!mutex.tryLock()) std.atomic.spinLoopHint();
}

/// Runs Lean work on a foreign worker with the runtime's required TLS scope.
pub fn withWorkerThread(comptime body: anytype, arguments: anytype) @TypeOf(@call(.auto, body, arguments)) {
    initializeWorkerThread();
    defer finalizeWorkerThread();
    return @call(.auto, body, arguments);
}

pub fn initializeWorkerThread() void {
    lean_initialize_thread();
}

pub fn finalizeWorkerThread() void {
    lean_finalize_thread();
}

pub fn release(object: *Object) void {
    if (isScalar(object)) return;
    const header: *Header = @ptrCast(@alignCast(object));
    if (header.rc > 1) header.rc -= 1 else if (header.rc != 0) lean_dec_ref_cold(object);
}

pub fn constructor(tag: u8, fields: []const *Object, scalar_size: u8) *Object {
    const result = allocateObject(@sizeOf(Header) + fields.len * @sizeOf(*Object) + scalar_size, tag, @intCast(fields.len));
    @memcpy(objectFields(result)[0..fields.len], fields);
    return result;
}

pub fn structure(fields: []const *Object) *Object {
    return constructor(0, fields, 0);
}

pub fn field(object: *Object, index: usize) *Object {
    return objectFields(object)[index];
}

pub fn nat(value: usize) *Object {
    return @ptrFromInt((value << 1) | 1);
}

pub fn natValue(value: *Object) usize {
    std.debug.assert(isScalar(value));
    return @intFromPtr(value) >> 1;
}

pub fn list(elements: []const *Object) *Object {
    var result = nat(0);
    var index = elements.len;
    while (index > 0) {
        index -= 1;
        result = constructor(1, &.{ elements[index], result }, 0);
    }
    return result;
}

/// Builds a Lean list directly from arbitrary consumer elements. The consumer
/// callback owns the element-to-Lean mapping; this helper owns only traversal.
pub fn listFromSlice(
    comptime Element: type,
    elements: []const Element,
    context: anytype,
    comptime encode: anytype,
) *Object {
    var result = nat(0);
    var index = elements.len;
    while (index > 0) {
        index -= 1;
        result = constructor(1, &.{ @call(.auto, encode, .{ context, elements[index] }), result }, 0);
    }
    return result;
}

pub const ListIterator = struct {
    current: *Object,

    pub fn next(self: *ListIterator) ?*Object {
        if (isScalar(self.current)) return null;
        const item = field(self.current, 0);
        self.current = field(self.current, 1);
        return item;
    }
};

pub fn iterateList(value: *Object) ListIterator {
    return .{ .current = value };
}

/// Traverses a Lean list while leaving output shape, capacity, ownership and
/// element conversion entirely with the consumer callback.
pub fn forEachList(
    comptime Context: type,
    value: *Object,
    context: *Context,
    comptime visit: fn (*Context, *Object) anyerror!void,
) anyerror!void {
    var iterator = iterateList(value);
    while (iterator.next()) |element| try visit(context, element);
}

pub fn string(value: []const u8) *Object {
    return lean_mk_string_from_bytes(value.ptr, value.len);
}

const StringObject = extern struct {
    header: Header,
    size: usize,
    capacity: usize,
    length: usize,
};

pub fn stringBytes(value: *Object) []const u8 {
    const string_object: *const StringObject = @ptrCast(@alignCast(value));
    const data: [*]const u8 = @ptrFromInt(@intFromPtr(value) + @sizeOf(StringObject));
    return data[0 .. string_object.size - 1];
}

fn objectFields(object: *Object) [*]*Object {
    return @ptrFromInt(@intFromPtr(object) + @sizeOf(Header));
}

fn allocateObject(size: usize, tag: u8, other: u8) *Object {
    const object = lean_alloc_object(size) orelse unreachable;
    const header: *Header = @ptrCast(@alignCast(object));
    const allocation_size = header.metadata.allocation_size;
    header.* = .{
        .rc = 1,
        .metadata = .{ .allocation_size = allocation_size, .other = other, .tag = tag },
    };
    return object;
}

test "Lean object header prefix remains eight bytes" {
    try std.testing.expectEqual(@as(usize, 8), @sizeOf(Header));
}

fn isScalar(object: *Object) bool {
    return @intFromPtr(object) & 1 == 1;
}

fn objectTag(object: *Object) u8 {
    return (@as(*const Header, @ptrCast(@alignCast(object)))).metadata.tag;
}
