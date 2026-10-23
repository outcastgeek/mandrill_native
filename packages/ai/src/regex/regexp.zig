const std = @import("std");
extern fn lre_compile(*c_int, [*]u8, c_int, [*:0]const u8, usize, c_int, ?*anyopaque) ?[*]u8;
extern fn lre_exec([*]?[*]u8, [*]const u8, [*]const u8, c_int, c_int, c_int, ?*anyopaque) c_int;
extern fn lre_get_capture_count([*]const u8) c_int;
extern fn lre_get_alloc_count([*]const u8) c_int;
pub const Flags = struct {
    ignoreCase: bool = false,
    multiline: bool = false,
    dotAll: bool = false,
    unicode: bool = false,
    sticky: bool = false,
    unicodeSets: bool = false,
    fn bits(self: @This()) c_int {
        return (@as(c_int, @intFromBool(self.ignoreCase)) << 1) | (@as(c_int, @intFromBool(self.multiline)) << 2) | (@as(c_int, @intFromBool(self.dotAll)) << 3) | (@as(c_int, @intFromBool(self.unicode)) << 4) | (@as(c_int, @intFromBool(self.sticky)) << 5) | (@as(c_int, @intFromBool(self.unicodeSets)) << 8);
    }
};
const Allocation = extern struct { length: usize, reserved: usize = 0 };
const Runtime = struct {
    allocator: std.mem.Allocator,
    io: ?std.Io = null,
    failure: ?anyerror = null,
    stackBase: usize = 0,
    fn enter(self: *@This()) void {
        var marker: u8 = 0;
        self.stackBase = @intFromPtr(&marker);
        self.failure = null;
    }
};
export fn lre_realloc(context: ?*anyopaque, ptr: ?*anyopaque, size: usize) ?*anyopaque {
    const state: *Runtime = @ptrCast(@alignCast(context.?));
    const old: ?*Allocation = if (ptr) |value| @ptrFromInt(@intFromPtr(value) - @sizeOf(Allocation)) else null;
    if (size == 0) {
        if (old) |value| state.allocator.rawFree(@as([*]u8, @ptrCast(value))[0..value.length], .of(u128), @returnAddress());
        return null;
    }
    const total = std.math.add(usize, size, @sizeOf(Allocation)) catch {
        state.failure = error.OutOfMemory;
        return null;
    };
    const memory = state.allocator.rawAlloc(total, .of(u128), @returnAddress()) orelse {
        state.failure = error.OutOfMemory;
        return null;
    };
    const header: *Allocation = @ptrCast(@alignCast(memory));
    header.* = .{ .length = total };
    const result = memory + @sizeOf(Allocation);
    if (old) |value| {
        const count = @min(size, value.length - @sizeOf(Allocation));
        @memcpy(result[0..count], (@as([*]const u8, @ptrCast(value)) + @sizeOf(Allocation))[0..count]);
        state.allocator.rawFree(@as([*]u8, @ptrCast(value))[0..value.length], .of(u128), @returnAddress());
    }
    return result;
}
export fn lre_check_stack_overflow(context: ?*anyopaque, size: usize) c_int {
    const state: *Runtime = @ptrCast(@alignCast(context.?));
    var marker: u8 = 0;
    const current = @intFromPtr(&marker);
    const used = @max(current, state.stackBase) - @min(current, state.stackBase);
    return @intFromBool(size > 256 * 1024 or used > 256 * 1024 - size);
}
export fn lre_check_timeout(context: ?*anyopaque) c_int {
    const state: *Runtime = @ptrCast(@alignCast(context.?));
    if (state.io) |io| io.checkCancel() catch |err| {
        state.failure = err;
        return 1;
    };
    return 0;
}
pub const Span = struct { start: usize, end: usize };
pub const Match = struct {
    allocator: std.mem.Allocator,
    input: []u16,
    captures: []?Span,
    sensitive: bool = false,
    pub fn deinit(self: *@This()) void {
        if (self.sensitive) std.crypto.secureZero(u16, self.input);
        self.allocator.free(self.input);
        self.allocator.free(self.captures);
        self.* = undefined;
    }
    pub fn text(self: @This(), a: std.mem.Allocator, index: usize) !?[]const u8 {
        const span = self.captures[index] orelse return null;
        return try std.unicode.wtf16LeToWtf8Alloc(a, self.input[span.start..span.end]);
    }
};
/// Byte protocol matches borrow input through deinit; capture spans are owned.
/// They use the engine's 8-bit mode without Unicode transcoding or copying.
pub const ByteMatch = struct {
    allocator: std.mem.Allocator,
    input: []const u8,
    captures: []?Span,
    pub fn deinit(self: *@This()) void {
        self.allocator.free(self.captures);
        self.* = undefined;
    }
    pub fn text(self: @This(), a: std.mem.Allocator, index: usize) !?[]const u8 {
        const span = self.captures[index] orelse return null;
        return try a.dupe(u8, self.input[span.start..span.end]);
    }
};
pub const Regex = struct {
    runtime: *Runtime,
    bytecode: [*]u8,
    pub fn init(a: std.mem.Allocator, io: ?std.Io, pattern: []const u8, flags: Flags) !@This() {
        const runtime = try a.create(Runtime);
        errdefer a.destroy(runtime);
        runtime.* = .{ .allocator = a, .io = io };
        runtime.enter();
        const terminated = try a.dupeSentinel(u8, pattern, 0);
        defer a.free(terminated);
        var length: c_int = 0;
        var message: [256]u8 = undefined;
        const code = lre_compile(&length, &message, message.len, terminated, pattern.len, flags.bits(), runtime) orelse return runtime.failure orelse error.InvalidRegularExpression;
        return .{ .runtime = runtime, .bytecode = code };
    }
    pub fn deinit(self: *@This()) void {
        const a = self.runtime.allocator;
        _ = lre_realloc(self.runtime, self.bytecode, 0);
        a.destroy(self.runtime);
        self.* = undefined;
    }
    pub fn exec(self: *@This(), a: std.mem.Allocator, text: []const u8, start: usize) !?Match {
        return self.execOwned(a, text, start, false);
    }
    /// Clear owned input copies on errors/no match, or when the returned match
    /// is released. Does not alter the borrowed source text.
    pub fn execSensitive(self: *@This(), a: std.mem.Allocator, text: []const u8, start: usize) !?Match {
        return self.execOwned(a, text, start, true);
    }
    /// Raw-byte framing only; ordinary JavaScript strings use exec instead.
    pub fn execBytes(self: *@This(), a: std.mem.Allocator, input: []const u8, start: usize) !?ByteMatch {
        if (start > input.len) return null;
        const count = std.math.cast(c_int, input.len) orelse return error.RegexInputTooLong;
        const slots = std.math.cast(usize, lre_get_alloc_count(self.bytecode)) orelse return error.InvalidRegularExpression;
        const captures = try a.alloc(?[*]u8, slots);
        defer a.free(captures);
        self.runtime.enter();
        const result = lre_exec(captures.ptr, self.bytecode, input.ptr, @intCast(start), count, 0, self.runtime);
        if (result < 0) return self.runtime.failure orelse if (result == -1) error.OutOfMemory else error.RegexExecutionFailed;
        if (result == 0) return null;
        const groups = try a.alloc(?Span, @intCast(lre_get_capture_count(self.bytecode)));
        errdefer a.free(groups);
        for (groups, 0..) |*group, index| {
            const begin = captures[index * 2] orelse {
                group.* = null;
                continue;
            };
            const end = captures[index * 2 + 1] orelse {
                group.* = null;
                continue;
            };
            group.* = .{ .start = @intFromPtr(begin) - @intFromPtr(input.ptr), .end = @intFromPtr(end) - @intFromPtr(input.ptr) };
        }
        return .{ .allocator = a, .input = input, .captures = groups };
    }
    fn execOwned(self: *@This(), a: std.mem.Allocator, text: []const u8, start: usize, sensitive: bool) !?Match {
        const input = try std.unicode.wtf8ToWtf16LeAlloc(a, text);
        var transferred = false;
        defer if (!transferred) {
            if (sensitive) std.crypto.secureZero(u16, input);
            a.free(input);
        };
        if (start > input.len) return null;
        const count = std.math.cast(c_int, input.len) orelse return error.RegexInputTooLong;
        const slots = std.math.cast(usize, lre_get_alloc_count(self.bytecode)) orelse return error.InvalidRegularExpression;
        const captures = try a.alloc(?[*]u8, slots);
        defer a.free(captures);
        self.runtime.enter();
        const result = lre_exec(captures.ptr, self.bytecode, @ptrCast(input.ptr), @intCast(start), count, 1, self.runtime);
        if (result < 0) return self.runtime.failure orelse if (result == -1) error.OutOfMemory else error.RegexExecutionFailed;
        if (result == 0) return null;
        const groups = try a.alloc(?Span, @intCast(lre_get_capture_count(self.bytecode)));
        errdefer a.free(groups);
        for (groups, 0..) |*group, index| {
            const begin = captures[index * 2] orelse {
                group.* = null;
                continue;
            };
            const end = captures[index * 2 + 1] orelse {
                group.* = null;
                continue;
            };
            group.* = .{ .start = (@intFromPtr(begin) - @intFromPtr(input.ptr)) / 2, .end = (@intFromPtr(end) - @intFromPtr(input.ptr)) / 2 };
        }
        transferred = true;
        return .{ .allocator = a, .input = input, .captures = groups, .sensitive = sensitive };
    }
};
pub fn matches(a: std.mem.Allocator, io: ?std.Io, pattern: []const u8, text: []const u8, flags: Flags) !bool {
    var regex = try Regex.init(a, io, pattern, flags);
    defer regex.deinit();
    var result = (try regex.exec(a, text, 0)) orelse return false;
    defer result.deinit();
    return true;
}
