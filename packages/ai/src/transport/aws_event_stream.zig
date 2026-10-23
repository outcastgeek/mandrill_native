//! Typed AWS event-stream framing over standard readers, integer codecs and CRC.
//! Payload bytes remain opaque until the adapter decodes them with std.json.
const std = @import("std");
const Crc = std.hash.crc.@"CRC-32/ISO-HDLC";
pub const HeaderValue = union(enum) { boolean: bool, byte: i8, short: i16, integer: i32, long: i64, bytes: []const u8, string: []const u8, timestamp: i64, uuid: [16]u8 };
pub const Header = struct { name: []const u8, value: HeaderValue };
pub const MessageKind = enum { event, exception, @"error" };
pub const Message = struct {
    headers: []const Header,
    payload: []const u8,
    pub fn header(self: @This(), name: []const u8) ?HeaderValue {
        var found: ?HeaderValue = null;
        for (self.headers) |h| if (std.mem.eql(u8, h.name, name)) {
            found = h.value;
        };
        return found;
    }
    pub fn stringHeader(self: @This(), name: []const u8) !?[]const u8 {
        const value = self.header(name) orelse return null;
        return if (value == .string) value.string else error.InvalidEventStreamStringHeader;
    }
    pub fn kind(self: @This()) !MessageKind {
        return std.meta.stringToEnum(MessageKind, (try self.stringHeader(":message-type")) orelse return error.MissingEventStreamMessageKind) orelse error.InvalidEventStreamMessageKind;
    }
};
pub const Decoder = struct {
    allocator: std.mem.Allocator,
    reader: *std.Io.Reader,
    arena: std.heap.ArenaAllocator,
    pub fn init(a: std.mem.Allocator, reader: *std.Io.Reader) @This() {
        return .{ .allocator = a, .reader = reader, .arena = .init(a) };
    }
    pub fn deinit(self: *@This()) void {
        self.arena.deinit();
        self.* = undefined;
    }
    /// Message bytes borrow this decoder until its next call or deinit.
    /// Only EOF before a new prelude is clean; a truncated frame propagates an
    /// error. readAllocAll reads the exact advertised length, not through EOF.
    pub fn next(self: *@This()) !?Message {
        _ = self.arena.reset(.retain_capacity);
        const a = self.arena.allocator();
        _ = self.reader.peekByte() catch |err| switch (err) {
            error.EndOfStream => return null,
            else => return err,
        };
        var prelude: [12]u8 = undefined;
        try self.reader.readSliceAll(&prelude);
        const total = std.mem.readInt(u32, prelude[0..4], .big);
        const header_length = std.mem.readInt(u32, prelude[4..8], .big);
        if (total < 16 or header_length > total - 16) return error.InvalidEventStreamLength;
        if (Crc.hash(prelude[0..8]) != std.mem.readInt(u32, prelude[8..12], .big)) return error.EventStreamPreludeChecksum;
        const rest = try self.reader.readAllocAll(a, total - 12);
        var crc = Crc.init();
        crc.update(&prelude);
        crc.update(rest[0 .. rest.len - 4]);
        const checksum = std.mem.readInt(u32, rest[rest.len - 4 ..][0..4], .big);
        if (crc.final() != checksum) return error.EventStreamMessageChecksum;
        var headers_reader = std.Io.Reader.fixed(rest[0..header_length]);
        var headers: std.ArrayList(Header) = .empty;
        while (headers_reader.bufferedLen() > 0) {
            // Smithy's header decoder permits an empty name. Keep it in the
            // typed header list as well; reserved routing names are separate.
            const name_length = try headers_reader.takeByte();
            const raw_name = try headers_reader.take(name_length);
            const name = try std.fmt.allocPrint(a, "{f}", .{std.unicode.fmtUtf8(raw_name)});
            const kind = try headers_reader.takeByte();
            const value: HeaderValue = switch (kind) {
                0 => .{ .boolean = true },
                1 => .{ .boolean = false },
                2 => .{ .byte = try headers_reader.takeByteSigned() },
                3 => .{ .short = try headers_reader.takeInt(i16, .big) },
                4 => .{ .integer = try headers_reader.takeInt(i32, .big) },
                5 => .{ .long = try headers_reader.takeInt(i64, .big) },
                6 => .{ .bytes = try headers_reader.take(try headers_reader.takeInt(u16, .big)) },
                7 => .{ .string = try std.fmt.allocPrint(a, "{f}", .{std.unicode.fmtUtf8(try headers_reader.take(try headers_reader.takeInt(u16, .big)))}) },
                8 => .{ .timestamp = try headers_reader.takeInt(i64, .big) },
                9 => .{ .uuid = (try headers_reader.takeArray(16)).* },
                else => return error.InvalidEventStreamHeaderType,
            };
            try headers.append(a, .{ .name = name, .value = value });
        }
        return .{ .headers = try headers.toOwnedSlice(a), .payload = rest[header_length .. rest.len - 4] };
    }
};
