//! Versioned binary envelope for a schema codec's binary representation.
//! Wire layout (v1): version:u8, schema-name-length:u32 big endian,
//! payload-length:u32 big endian, UTF-8 schema name, codec payload. The
//! schema codec owns the payload grammar; this module owns identity, framing
//! and limits shared by event replication and stored outcomes.
const std = @import("std");
const outcomes = @import("../outcome.zig");
const Issue = @import("../schema/schema_issue.zig").Issue;

/// `Version` cases: `v1`.
pub const Version = enum(u8) { v1 = 1 };
/// Length uses `u32` as its stable representation in this module.
pub const Length = u32;
/// Width in bytes of an encoded schema length field.
pub const length_bytes = @sizeOf(Length);
/// Fixed byte width of a schema-binary header.
pub const header_bytes = @sizeOf(Version) + 2 * length_bytes;

/// Explicit bounds on schema-name and encoded payload bytes.
pub const Limits = struct {
    max_schema_name_bytes: usize,
    max_payload_bytes: usize,
    max_frame_bytes: usize,

    /// Check whether this value satisfies its structural invariants.
    /// Parameters:
    /// - `self: Limits`: Receiver passed by value: the struct itself is copied, while any referenced storage keeps its original ownership and lifetime.
    /// Returns (`bool`): `true` when name/payload limits are positive and header plus payload cannot overflow.
    pub fn valid(self: Limits) bool {
        const parts = std.math.add(usize, self.max_schema_name_bytes, self.max_payload_bytes) catch return false;
        const total = std.math.add(usize, header_bytes, parts) catch return false;
        return self.max_schema_name_bytes > 0 and self.max_payload_bytes > 0 and
            self.max_schema_name_bytes <= std.math.maxInt(Length) and
            self.max_payload_bytes <= std.math.maxInt(Length) and
            self.max_frame_bytes >= total;
    }
};

/// Typed view record.
pub const View = struct { version: Version, schema_name: []const u8, payload: []const u8 };

fn writeLength(target: []u8, value: Length) void {
    var remaining = value;
    var index = target.len;
    while (index > 0) {
        index -= 1;
        target[index] = @truncate(remaining);
        remaining >>= @bitSizeOf(u8);
    }
}

fn readLength(source: []const u8) Length {
    var value: Length = 0;
    for (source) |byte| value = (value << @bitSizeOf(u8)) | byte;
    return value;
}

/// Validate a key or encoded value without taking ownership.
/// Parameters:
/// - `frame: []const u8`: Encoded protocol frame to parse, dispatch, or transmit.
/// - `limits: Limits`: Caller-selected, validated behavior or resource bounds; no hidden default is
///   installed.
/// Returns (`outcomes.Result(View, Issue)`): An ok/err Result; owned payloads must be released according to their type contract.
pub fn inspect(frame: []const u8, limits: Limits) outcomes.Result(View, Issue) {
    if (!limits.valid() or frame.len > limits.max_frame_bytes)
        return .{ .err = Issue.pure(.decode, .size_limit) };
    if (frame.len < header_bytes)
        return .{ .err = Issue.pure(.decode, .invalid_value) };
    if (frame[0] != @backingInt(Version.v1))
        return .{ .err = Issue.pure(.decode, .incompatible_version) };
    const version: Version = .v1;
    const schema_length: usize = readLength(frame[@sizeOf(Version)..][0..length_bytes]);
    const payload_length: usize = readLength(frame[@sizeOf(Version) + length_bytes ..][0..length_bytes]);
    if (schema_length == 0 or schema_length > limits.max_schema_name_bytes or
        payload_length > limits.max_payload_bytes)
        return .{ .err = Issue.pure(.decode, .size_limit) };
    const body = std.math.add(usize, schema_length, payload_length) catch
        return .{ .err = Issue.pure(.decode, .size_limit) };
    const total = std.math.add(usize, header_bytes, body) catch
        return .{ .err = Issue.pure(.decode, .size_limit) };
    if (total != frame.len)
        return .{ .err = Issue.pure(.decode, .invalid_value) };
    const name = frame[header_bytes..][0..schema_length];
    if (!std.unicode.utf8ValidateSlice(name))
        return .{ .err = Issue.pure(.decode, .invalid_utf8) };
    return .{ .ok = .{ .version = version, .schema_name = name, .payload = frame[header_bytes + schema_length ..] } };
}

/// Encode a value into its wire representation.
/// Parameters:
/// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
/// - `schema_name: comptime []const u8`: Compile-time typed codec/schema fixing the corresponding decoded and encoded
///   value types.
/// - `payload: []const u8`: Borrowed input; keep its owner alive until the operation has copied or joined it.
/// - `limits: Limits`: Caller-selected, validated behavior or resource bounds; no hidden default is
///   installed.
/// Returns (`outcomes.Exit(outcomes.OwnedSlice(u8), Issue)`): Carries `outcomes.OwnedSlice(u8)` on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
pub fn encode(env: anytype, comptime schema_name: []const u8, payload: []const u8, limits: Limits) outcomes.Exit(outcomes.OwnedSlice(u8), Issue) {
    comptime {
        if (schema_name.len == 0 or !std.unicode.utf8ValidateSlice(schema_name))
            @compileError("binary codec schema name must be nonempty UTF-8 metadata");
    }
    if (!limits.valid() or schema_name.len > limits.max_schema_name_bytes or
        payload.len > limits.max_payload_bytes)
        return .{ .fail = Issue.init(env, .encode, .size_limit) };
    const body = std.math.add(usize, schema_name.len, payload.len) catch
        return .{ .fail = Issue.init(env, .encode, .size_limit) };
    const total = std.math.add(usize, header_bytes, body) catch
        return .{ .fail = Issue.init(env, .encode, .size_limit) };
    if (total > limits.max_frame_bytes)
        return .{ .fail = Issue.init(env, .encode, .size_limit) };
    const frame = env.allocator.alloc(u8, total) catch return .{ .die = outcomes.Defect.out_of_memory };
    frame[0] = @backingInt(Version.v1);
    writeLength(frame[@sizeOf(Version)..][0..length_bytes], @intCast(schema_name.len));
    writeLength(frame[@sizeOf(Version) + length_bytes ..][0..length_bytes], @intCast(payload.len));
    @memcpy(frame[header_bytes..][0..schema_name.len], schema_name);
    @memcpy(frame[header_bytes + schema_name.len ..], payload);
    return .{ .success = .{ .items = frame, .allocator = env.allocator } };
}

/// Decode a value from its wire representation.
/// Parameters:
/// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
/// - `schema_name: comptime []const u8`: Compile-time typed codec/schema fixing the corresponding decoded and encoded
///   value types.
/// - `frame: []const u8`: Encoded protocol frame to parse, dispatch, or transmit.
/// - `limits: Limits`: Caller-selected, validated behavior or resource bounds; no hidden default is
///   installed.
/// Returns (`outcomes.Exit(outcomes.OwnedSlice(u8), Issue)`): Carries `outcomes.OwnedSlice(u8)` on `.success` or `Issue` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
pub fn decode(env: anytype, comptime schema_name: []const u8, frame: []const u8, limits: Limits) outcomes.Exit(outcomes.OwnedSlice(u8), Issue) {
    const view = switch (inspect(frame, limits)) {
        .ok => |value| value,
        .err => |problem| return .{ .fail = problem },
    };
    if (!std.mem.eql(u8, view.schema_name, schema_name))
        return .{ .fail = Issue.init(env, .decode, .incompatible_version) };
    const payload = env.allocator.dupe(u8, view.payload) catch return .{ .die = outcomes.Defect.out_of_memory };
    return .{ .success = .{ .items = payload, .allocator = env.allocator } };
}

/// Direct adapter retains pure callback execution while framing allocation
/// remains explicit through R. No Effect frames are created by these methods.
pub fn Pure(comptime child: anytype, comptime schema_name: []const u8) type {
    const Identity = schema_name;
    const Child = child;
    const checked = @import("../schema/codec.zig").requirePure(child);
    const C = @TypeOf(checked);
    if (C.Representation != []const u8) @compileError("pure binary adapter requires a borrowed byte representation");
    return struct {
        pub const binary_schema = struct {
            pub const codec = Child;
            pub const name = Identity;
        };
        pub const LimitsType = Limits;
        pub const Decoded = C.Decoded;
        pub const pure_decoding = true;
        pub const pure_encoding = true;
        pub fn decodeDirect(env: anytype, input: []const u8, limits: Limits) outcomes.Exit(C.Decoded, Issue) {
            const viewed = inspect(input, limits);
            if (viewed == .err) return .{ .fail = viewed.err };
            if (!std.mem.eql(u8, viewed.ok.schema_name, schema_name)) return .{ .fail = Issue.init(env, .decode, .incompatible_version) };
            const bytes = viewed.ok.payload;
            return switch (checked.decodeDirect(&bytes)) {
                .ok => |value| .{ .success = value },
                .err => |problem| .{ .fail = problem },
            };
        }
        pub fn encodeDirect(env: anytype, input: *const C.Decoded, limits: Limits) outcomes.Exit(outcomes.OwnedSlice(u8), Issue) {
            const encoded = checked.encodeDirect(input);
            if (encoded == .err) return .{ .fail = encoded.err };
            return encode(env, schema_name, encoded.ok, limits);
        }
        pub fn validateDirect(input: *const C.Decoded) outcomes.Result(void, Issue) {
            return checked.validateDirect(input);
        }
    };
}
