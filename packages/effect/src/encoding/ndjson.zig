//! Bounded, incremental NDJSON records. Each returned record owns its bytes;
//! the transport may release a fed chunk immediately. The JSON value itself
//! is decoded by the caller's typed Schema codec.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const Issue = @import("../schema/schema_issue.zig").Issue;

/// Record delimiter byte for NDJSON framing.
pub const newline: u8 = '\n';
/// Maximum record and buffered byte counts for incremental NDJSON decoding.
pub const Limits = struct {
    max_record_bytes: usize,
    max_buffered_bytes: usize,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Limits`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` when record limit is positive and buffered capacity can hold one full record.
    pub fn valid(self: Limits) bool {
        return self.max_record_bytes > 0 and self.max_buffered_bytes >= self.max_record_bytes;
    }
};

/// Decoder result: `record` owns one complete line, while `need_more` asks
/// the caller to feed another chunk. End of input is handled separately.
pub const Read = union(enum) {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    need_more,
    record: outcomes.OwnedSlice(u8),

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *@This()`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *@This()) void {
        if (self.* == .record) self.record.deinit();
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
            .record => |value| .{ .record = try value.clone() },
        };
    }
};

fn validJson(allocator: std.mem.Allocator, record: []const u8) bool {
    var parsed = std.json.parseFromSlice(std.json.Value, allocator, record, .{ .allocate = .alloc_always }) catch return false;
    parsed.deinit();
    return true;
}

/// Encode one framed record using the selected codec.
/// Parameters:
/// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
/// - `record: []const u8`: Encoded record bytes supplied by the caller.
/// - `limits: Limits`: Caller-selected, validated behavior or resource bounds; no hidden default is
///   installed.
/// Returns (`outcomes.Exit(outcomes.OwnedSlice(u8), Issue)`): Carries `outcomes.OwnedSlice(u8)` on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
pub fn encodeOne(env: anytype, record: []const u8, limits: Limits) outcomes.Exit(outcomes.OwnedSlice(u8), Issue) {
    if (!limits.valid() or record.len > limits.max_record_bytes)
        return .{ .fail = Issue.init(env, .encode, .size_limit) };
    if (!validJson(env.allocator, record))
        return .{ .fail = Issue.init(env, .encode, .invalid_value) };
    const length = std.math.add(usize, record.len, @sizeOf(u8)) catch
        return .{ .fail = Issue.init(env, .encode, .size_limit) };
    const bytes = env.allocator.alloc(u8, length) catch return .{ .die = outcomes.Defect.out_of_memory };
    @memcpy(bytes[0..record.len], record);
    bytes[record.len] = newline;
    return .{ .success = .{ .items = bytes, .allocator = env.allocator } };
}

/// Incremental NDJSON reader retaining buffered bytes until a complete record is returned.
pub const Decoder = struct {
    /// Ownership marker read by `Exit`/`Result` cleanup helpers; this payload implements `deinit` and
    /// must be released after use.
    pub const effect_owned_payload = true;
    allocator: std.mem.Allocator,
    limits: Limits,
    buffer: std.array_list.Managed(u8),

    /// Create a bounded incremental NDJSON decoder.
    /// Parameters:
    /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
    /// - `limits: Limits`: Caller-selected, validated behavior or resource bounds; no hidden default is
    ///   installed.
    /// Returns (`outcomes.Exit(Decoder, Issue)`): Carries `Decoder` on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
    pub fn init(env: anytype, limits: Limits) outcomes.Exit(Decoder, Issue) {
        if (!limits.valid()) return .{ .fail = Issue.init(env, .definition, .size_limit) };
        return .{ .success = .{ .allocator = env.allocator, .limits = limits, .buffer = std.array_list.Managed(u8).init(env.allocator) } };
    }

    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *Decoder`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *Decoder) void {
        self.buffer.deinit();
        self.* = undefined;
    }

    /// Duplicate this value; marked owning payloads are cloned, while ordinary
    /// borrowed fields retain their original lifetime.
    /// Parameters:
    /// - `self: *const Decoder`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`error{OutOfMemory}!Decoder`): A `Decoder` on success; `OutOfMemory` if required allocation fails.
    pub fn clone(self: *const Decoder) error{OutOfMemory}!Decoder {
        var copied: Decoder = .{ .allocator = self.allocator, .limits = self.limits, .buffer = std.array_list.Managed(u8).init(self.allocator) };
        errdefer copied.deinit();
        try copied.buffer.appendSlice(self.buffer.items);
        return copied;
    }

    /// Feed more bytes into the incremental decoder.
    /// Parameters:
    /// - `self: *Decoder`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `chunk: []const u8`: Borrowed input; keep its owner alive until the operation has copied or joined
    ///   it.
    /// Returns (`outcomes.Exit(void, Issue)`): Carries no value on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
    pub fn feed(self: *Decoder, chunk: []const u8) outcomes.Exit(void, Issue) {
        const total = std.math.add(usize, self.buffer.items.len, chunk.len) catch
            return .{ .fail = Issue.pure(.decode, .size_limit) };
        if (total > self.limits.max_buffered_bytes)
            return .{ .fail = Issue.pure(.decode, .size_limit) };
        self.buffer.appendSlice(chunk) catch return .{ .die = outcomes.Defect.out_of_memory };
        return .{ .success = {} };
    }

    /// Advance to the next item or state.
    /// Parameters:
    /// - `self: *Decoder`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`outcomes.Exit(Read, Issue)`): Carries `Read` on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
    pub fn next(self: *Decoder) outcomes.Exit(Read, Issue) {
        const end = std.mem.indexOfScalar(u8, self.buffer.items, newline) orelse {
            if (self.buffer.items.len > self.limits.max_record_bytes)
                return .{ .fail = Issue.pure(.decode, .size_limit) };
            return .{ .success = .need_more };
        };
        if (end > self.limits.max_record_bytes)
            return .{ .fail = Issue.pure(.decode, .size_limit) };
        const record = self.buffer.items[0..end];
        if (!validJson(self.allocator, record))
            return .{ .fail = Issue.pure(.decode, .invalid_value) };
        const owned = self.allocator.dupe(u8, record) catch return .{ .die = outcomes.Defect.out_of_memory };
        const consumed = end + @sizeOf(u8);
        const remaining = self.buffer.items.len - consumed;
        std.mem.copyForwards(u8, self.buffer.items[0..remaining], self.buffer.items[consumed..]);
        self.buffer.shrinkRetainingCapacity(remaining);
        return .{ .success = .{ .record = .{ .items = owned, .allocator = self.allocator } } };
    }

    /// A final unterminated record is accepted only after the caller declares
    /// end of input. Empty tails are not emitted as records.
    ///
    /// Parameters:
    /// - `self: *Decoder`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`outcomes.Exit(Read, Issue)`): Carries `Read` on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
    pub fn finish(self: *Decoder) outcomes.Exit(Read, Issue) {
        if (self.buffer.items.len == 0) return .{ .success = .need_more };
        if (self.buffer.items.len > self.limits.max_record_bytes)
            return .{ .fail = Issue.pure(.decode, .size_limit) };
        if (!validJson(self.allocator, self.buffer.items))
            return .{ .fail = Issue.pure(.decode, .invalid_value) };
        const owned = self.allocator.dupe(u8, self.buffer.items) catch return .{ .die = outcomes.Defect.out_of_memory };
        self.buffer.clearRetainingCapacity();
        return .{ .success = .{ .record = .{ .items = owned, .allocator = self.allocator } } };
    }
};
