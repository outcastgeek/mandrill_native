const std = @import("std");
const j = @import("../json/value.zig");
pub const Failure = struct { err: anyerror, status: ?u16 = null, message: []const u8 };
/// SDK processStreamResponse inspects each streaming-decoded body read before
/// accumulating its SSE records. All decoding/parsing uses standard codecs.
pub const StreamingBody = struct {
    allocator: std.mem.Allocator,
    errorMessage: *?[]const u8,
    pending: [3]u8 = undefined,
    pendingLength: usize = 0,
    first: bool = true,
    pub fn observe(ctx: ?*anyopaque, chunk: []const u8) !void {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        var temporary = std.heap.ArenaAllocator.init(self.allocator);
        defer temporary.deinit();
        const a = temporary.allocator();
        const combined = try std.mem.concat(a, u8, &.{ self.pending[0..self.pendingLength], chunk });
        const prefix = @import("../transport/sse.zig").googleEofPrefix(combined);
        const tail = combined[prefix.len..];
        @memcpy(self.pending[0..tail.len], tail);
        self.pendingLength = tail.len;
        var decoded: []const u8 = try std.fmt.allocPrint(a, "{f}", .{std.unicode.fmtUtf8(prefix)});
        if (prefix.len > 0 and self.first) {
            self.first = false;
            if (std.mem.startsWith(u8, decoded, &.{ 0xef, 0xbb, 0xbf })) decoded = decoded[3..];
        }
        const value = j.parseFromSliceLeaky(std.json.Value, a, decoded, .{ .duplicate_field_behavior = .use_last }) catch |err| {
            if (err == error.OutOfMemory) return err;
            return;
        };
        const raw = j.get(value, "error") orelse return;
        const failure = j.roundTrip(a, raw) catch |err| {
            if (err == error.OutOfMemory) return err;
            return;
        };
        // Null/malformed receivers and non-ApiError conversion failures are
        // swallowed by the SDK's surrounding inspection catch.
        if (failure == .null) return;
        const code = @import("../utils/numbers.zig").coerceValue(a, j.get(failure, "code")) catch |err| {
            if (err == error.OutOfMemory) return err;
            return;
        };
        if (!(code >= 400 and code < 600)) return;
        const status = @import("../utils/diagnostics.zig").formatThrownValue(a, if (j.get(failure, "status")) |status| .{ .value = status } else .undefined_value) catch |err| {
            if (err == error.OutOfMemory) return err;
            return;
        };
        const encoded = j.stringify(a, value, .{}) catch |err| {
            if (err == error.OutOfMemory) return err;
            return;
        };
        self.errorMessage.* = try std.fmt.allocPrint(self.allocator, "got status: {s}. {s}", .{ status, encoded });
        return error.GoogleStreamingApiError;
    }
};
pub const Policy = struct {
    allocator: std.mem.Allocator,
    delays: []const f64,
    statuses: []const u16,
    next: usize = 0,
    counts: std.StringHashMapUnmanaged(usize) = .{},
    main: ?Failure = null,
    mainCount: usize = 0,
    pub fn init(a: std.mem.Allocator, io: std.Io, params: std.json.Value) !?@This() {
        const config = j.get(j.get(j.get(params, "config") orelse .null, "httpOptions") orelse .null, "retryOptions") orelse return null;
        if (config == .null) return null;
        if (config != .object) return error.InvalidGoogleRetryOptions;
        const attempts = try number(config, "attempts", 5);
        if (attempts >= @as(f64, @floatFromInt(std.math.maxInt(usize)))) return error.InvalidGoogleRetryOptions;
        const retries: usize = @intFromFloat(@ceil(@max(1, attempts)) - 1);
        const minimum = @floor(try number(config, "initialDelay", 1) * 1000 + 0.5);
        const maximum = @max(minimum, @floor(try number(config, "maxDelay", 60) * 1000 + 0.5));
        const factor = try number(config, "expBase", 2);
        const randomize = try number(config, "jitter", 1) > 0;
        const delays = try a.alloc(f64, retries);
        for (delays, 0..) |*out, index| {
            var fraction: f64 = 0;
            if (randomize) {
                var bytes: [8]u8 = undefined;
                try io.randomSecure(&bytes);
                fraction = @as(f64, @floatFromInt(std.mem.readInt(u64, &bytes, .little) >> 11)) / 9007199254740992.0;
            }
            const calculated = @min(@floor((1 + fraction) * @max(minimum, 1) * std.math.pow(f64, factor, @floatFromInt(index)) + 0.5), maximum);
            out.* = calculated;
        }
        std.mem.sort(f64, delays, {}, std.sort.asc(f64));
        var statuses: []const u16 = &.{ 408, 429, 500, 502, 503, 504 };
        if (j.get(config, "httpStatusCodes")) |codes| if (codes != .null) {
            if (codes != .array) return error.InvalidGoogleRetryOptions;
            const copied = try a.alloc(u16, codes.array.items.len);
            for (codes.array.items, copied) |code, *out| out.* = std.json.innerParseFromValue(u16, a, code, .{}) catch |err| {
                if (err == error.OutOfMemory) return err;
                return error.InvalidGoogleRetryOptions;
            };
            statuses = copied;
        };
        return .{ .allocator = a, .delays = delays, .statuses = statuses };
    }
    fn number(config: std.json.Value, key: []const u8, default: f64) !f64 {
        const raw = j.get(config, key) orelse return default;
        if (raw == .null) return default;
        const value = j.number(raw) orelse return error.InvalidGoogleRetryOptions;
        if (!std.math.isFinite(value)) return error.InvalidGoogleRetryOptions;
        return value;
    }
    pub fn retryable(self: @This(), status: u16) bool {
        return std.mem.findScalar(u16, self.statuses, status) != null;
    }
    /// Records every rejected attempt; equal-frequency errors select the later
    /// one, matching retry.operation().mainError(). Returns whether to retry.
    pub fn wait(self: *@This(), io: std.Io, failure: Failure) !bool {
        const entry = try self.counts.getOrPut(self.allocator, failure.message);
        if (!entry.found_existing) {
            entry.key_ptr.* = try self.allocator.dupe(u8, failure.message);
            entry.value_ptr.* = 0;
        }
        entry.value_ptr.* += 1;
        if (entry.value_ptr.* >= self.mainCount) {
            self.mainCount = entry.value_ptr.*;
            self.main = failure;
        }
        if (self.next >= self.delays.len) return false;
        const requested = self.delays[self.next];
        self.next += 1;
        // retry sorts its computed delays before Node normalizes timer bounds.
        const delay: i64 = if (!std.math.isFinite(requested) or requested < 1 or requested > std.math.maxInt(i32)) 1 else @intFromFloat(requested);
        try io.sleep(.fromMilliseconds(delay), .awake);
        return true;
    }
};
