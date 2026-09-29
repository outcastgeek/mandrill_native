//! Transport-neutral, length-prefixed RPC framing. The caller chooses both
//! limits; this module never creates a socket, listener or execution runtime.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const Protocol = @import("rpc_error.zig").Protocol;

/// FrameLength uses `u32` as its stable representation in this module.
pub const FrameLength = u32;
/// Fixed byte width of a framed RPC length header.
pub const header_bytes: usize = @sizeOf(FrameLength);

/// Maximum single frame and buffered byte counts for incremental RPC framing.
pub const Limits = struct {
    max_frame_bytes: usize,
    max_buffered_bytes: usize,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Limits`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` when frame size is positive, fits the wire length, and the buffer holds its header.
    pub fn valid(self: Limits) bool {
        const full = std.math.add(usize, self.max_frame_bytes, header_bytes) catch return false;
        return self.max_frame_bytes > 0 and
            self.max_frame_bytes <= std.math.maxInt(FrameLength) and
            self.max_buffered_bytes >= full;
    }
};

/// Framer result: complete owned frame or a request for more bytes.
pub const Read = union(enum) {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    need_more,
    frame: outcomes.OwnedSlice(u8),

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        if (self.* == .frame) self.frame.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const @This()`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!@This()`): copied value on success; `OutOfMemory` if owned storage could not be cloned. The source remains valid.
    pub fn clone(self: *const @This()) error{OutOfMemory}!@This() {
        return switch (self.*) {
            .need_more => .need_more,
            .frame => |value| .{ .frame = try value.clone() },
        };
    }
};

/// Validates one complete provider frame. The incremental Framer remains the
/// path for byte streams that may split or coalesce frames.
///
/// Parameters:
/// - `bytes: []const u8`: Borrowed input; keep its owner alive until the operation has copied or joined it.
/// - `limits: Limits`: Caller-selected, validated behavior or resource bounds; no hidden default is
///   installed.
/// Returns (`outcomes.Result([]const u8, Protocol)`): An ok/err Result; owned payloads must be released according to their type contract.
pub fn inspectOne(bytes: []const u8, limits: Limits) outcomes.Result([]const u8, Protocol) {
    if (!limits.valid()) return .{ .err = .frame_too_large };
    if (bytes.len < header_bytes) return .{ .err = .malformed_envelope };
    var length: FrameLength = 0;
    for (bytes[0..header_bytes]) |byte| length = (length << @bitSizeOf(u8)) | byte;
    const payload_length: usize = length;
    if (payload_length > limits.max_frame_bytes) return .{ .err = .frame_too_large };
    const total = std.math.add(usize, header_bytes, payload_length) catch return .{ .err = .frame_too_large };
    if (total != bytes.len) return .{ .err = .malformed_envelope };
    return .{ .ok = bytes[header_bytes..] };
}

/// Decode one length-framed RPC message.
/// Parameters:
/// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
/// - `bytes: []const u8`: Borrowed input; keep its owner alive until the operation has copied or joined it.
/// - `limits: Limits`: Caller-selected, validated behavior or resource bounds; no hidden default is
///   installed.
/// Returns (`outcomes.Exit(outcomes.OwnedSlice(u8), Protocol)`): Carries `outcomes.OwnedSlice(u8)` on `.success` or `Protocol` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
pub fn decodeOne(env: anytype, bytes: []const u8, limits: Limits) outcomes.Exit(outcomes.OwnedSlice(u8), Protocol) {
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
        @compileError("RPC frame decoding requires R.allocator");
    const payload_view = switch (inspectOne(bytes, limits)) {
        .ok => |value| value,
        .err => |problem| return .{ .fail = problem },
    };
    const payload = env.allocator.dupe(u8, payload_view) catch return .{ .die = outcomes.Defect.out_of_memory };
    return .{ .success = .{ .items = payload, .allocator = env.allocator } };
}

/// Encode one framed record using the selected codec.
/// Parameters:
/// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
/// - `payload: []const u8`: Borrowed input; keep its owner alive until the operation has copied or joined it.
/// - `limits: Limits`: Caller-selected, validated behavior or resource bounds; no hidden default is
///   installed.
/// Returns (`outcomes.Exit(outcomes.OwnedSlice(u8), Protocol)`): Carries `outcomes.OwnedSlice(u8)` on `.success` or `Protocol` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
pub fn encodeOne(env: anytype, payload: []const u8, limits: Limits) outcomes.Exit(outcomes.OwnedSlice(u8), Protocol) {
    const R = @typeInfo(@TypeOf(env)).pointer.child;
    if (!@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
        @compileError("RPC frame encoding requires R.allocator");
    if (!limits.valid() or payload.len > limits.max_frame_bytes) return .{ .fail = .frame_too_large };
    const total = std.math.add(usize, header_bytes, payload.len) catch return .{ .fail = .frame_too_large };
    const frame = env.allocator.alloc(u8, total) catch return .{ .die = outcomes.Defect.out_of_memory };
    var remaining: FrameLength = @intCast(payload.len);
    var index = header_bytes;
    while (index > 0) {
        index -= 1;
        frame[index] = @truncate(remaining);
        remaining >>= @bitSizeOf(u8);
    }
    std.mem.copyForwards(u8, frame[header_bytes..], payload);
    return .{ .success = .{ .items = frame, .allocator = env.allocator } };
}

/// Incremental frame parser retaining buffered bytes until an entire length-prefixed frame is available.
pub const Framer = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    allocator: std.mem.Allocator,
    limits: Limits,
    buffer: std.array_list.Managed(u8),

    /// Create a bounded frame parser using the caller allocator and configured limits.
    /// Parameters:
    /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
    /// - `limits: Limits`: Caller-selected, validated behavior or resource bounds; no hidden default is
    ///   installed.
    /// Returns (`outcomes.Exit(Framer, Protocol)`): Carries `Framer` on `.success` or `Protocol` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
    pub fn init(env: anytype, limits: Limits) outcomes.Exit(Framer, Protocol) {
        const R = @typeInfo(@TypeOf(env)).pointer.child;
        if (R == void or !@hasField(R, "allocator") or @FieldType(R, "allocator") != std.mem.Allocator)
            @compileError("RPC framing requires the caller allocator in R");
        if (!limits.valid()) return .{ .fail = .frame_too_large };
        return .{ .success = .{
            .allocator = env.allocator,
            .limits = limits,
            .buffer = std.array_list.Managed(u8).init(env.allocator),
        } };
    }

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *Framer`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *Framer) void {
        self.buffer.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const Framer`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!Framer`): A `Framer` on success; `OutOfMemory` if required allocation fails.
    pub fn clone(self: *const Framer) error{OutOfMemory}!Framer {
        var copy: Framer = .{
            .allocator = self.allocator,
            .limits = self.limits,
            .buffer = std.array_list.Managed(u8).init(self.allocator),
        };
        try copy.buffer.appendSlice(self.buffer.items);
        return copy;
    }

    /// Encode a value into its wire representation.
    /// Parameters:
    /// - `self: *const Framer`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// - `payload: []const u8`: Borrowed input; keep its owner alive until the operation has copied or joined
    ///   it.
    /// Returns (`outcomes.Exit(outcomes.OwnedSlice(u8), Protocol)`): Carries `outcomes.OwnedSlice(u8)` on `.success` or `Protocol` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
    pub fn encode(self: *const Framer, payload: []const u8) outcomes.Exit(outcomes.OwnedSlice(u8), Protocol) {
        if (payload.len > self.limits.max_frame_bytes) return .{ .fail = .frame_too_large };
        const total = std.math.add(usize, header_bytes, payload.len) catch return .{ .fail = .frame_too_large };
        const result = self.allocator.alloc(u8, total) catch return .{ .die = outcomes.Defect.out_of_memory };
        var remaining: FrameLength = @intCast(payload.len);
        var index = header_bytes;
        while (index > 0) {
            index -= 1;
            result[index] = @truncate(remaining);
            remaining >>= @bitSizeOf(u8);
        }
        std.mem.copyForwards(u8, result[header_bytes..], payload);
        return .{ .success = .{ .items = result, .allocator = self.allocator } };
    }

    /// Each input chunk is copied so it may be released immediately. Call
    /// `next` until it returns need_more before feeding more transport bytes.
    ///
    /// Parameters:
    /// - `self: *Framer`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `chunk: []const u8`: Borrowed input; keep its owner alive until the operation has copied or joined
    ///   it.
    /// Returns (`outcomes.Exit(void, Protocol)`): Carries no value on `.success` or `Protocol` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
    pub fn feed(self: *Framer, chunk: []const u8) outcomes.Exit(void, Protocol) {
        const length = std.math.add(usize, self.buffer.items.len, chunk.len) catch return .{ .fail = .frame_too_large };
        if (length > self.limits.max_buffered_bytes) return .{ .fail = .frame_too_large };
        self.buffer.appendSlice(chunk) catch return .{ .die = outcomes.Defect.out_of_memory };
        return .{ .success = {} };
    }

    /// Advance to the next item or state.
    /// Parameters:
    /// - `self: *Framer`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`outcomes.Exit(Read, Protocol)`): Carries `Read` on `.success` or `Protocol` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
    pub fn next(self: *Framer) outcomes.Exit(Read, Protocol) {
        if (self.buffer.items.len < header_bytes) return .{ .success = .need_more };
        var length: FrameLength = 0;
        for (self.buffer.items[0..header_bytes]) |byte| {
            length = (length << @bitSizeOf(u8)) | byte;
        }
        const payload_length: usize = length;
        if (payload_length > self.limits.max_frame_bytes) return .{ .fail = .frame_too_large };
        const total = std.math.add(usize, header_bytes, payload_length) catch return .{ .fail = .frame_too_large };
        if (self.buffer.items.len < total) return .{ .success = .need_more };
        const frame = self.allocator.dupe(u8, self.buffer.items[header_bytes..total]) catch return .{ .die = outcomes.Defect.out_of_memory };
        const leftover = self.buffer.items.len - total;
        std.mem.copyForwards(u8, self.buffer.items[0..leftover], self.buffer.items[total..]);
        self.buffer.shrinkRetainingCapacity(leftover);
        return .{ .success = .{ .frame = .{ .items = frame, .allocator = self.allocator } } };
    }
};
