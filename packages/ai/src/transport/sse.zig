//! Typed SSE framing over std.Io.Reader. Event payloads stay opaque until the
//! provider passes them to std.json; framing never inspects JSON syntax.
const std = @import("std");
const unicode = @import("../utils/unicode.zig");
pub const Event = struct { event: []const u8 = "", data: []const u8, id: []const u8 = "", retry: ?u64 = null, raw: []const u8 = "" };
/// These adapters use record boundaries rather than the SDK's SSE line decoder.
pub const RecordFraming = enum { lines, codex, pi, google };
/// A streaming TextDecoder does not flush a pending sequence unless decode()
/// is called without stream:true. Google omits that final call. All validity
/// decisions below belong to std.unicode; this only locates a pending suffix.
/// The original bytes are neither repaired nor rewritten.
pub fn googleEofPrefix(bytes: []const u8) []const u8 {
    for (1..@min(bytes.len, 3) + 1) |tail_length| {
        const start = bytes.len - tail_length;
        const sequence_length = std.unicode.utf8ByteSequenceLength(bytes[start]) catch continue;
        if (sequence_length <= tail_length) continue;
        // Ask the standard validator whether this suffix can be completed.
        // UTF8's only restricted continuation range is its first one. Every
        // permitted range includes 80 or BF, so these two bounded completion
        // candidates cover every possible valid prefix. They are scratch
        // validation inputs, never substituted into the record or payload.
        for ([_]u8{ 0x80, 0xbf }) |continuation| {
            var candidate: [4]u8 = @splat(continuation);
            @memcpy(candidate[0..tail_length], bytes[start..]);
            if (std.unicode.utf8ValidateSlice(candidate[0..sequence_length])) return bytes[0..start];
        }
    }
    return bytes;
}
pub const Decoder = struct {
    allocator: std.mem.Allocator,
    reader: *std.Io.Reader,
    /// An explicit logical body-chunk source, used by the pinned Anthropic
    /// parser whose CR/LF behavior depends on each body read's boundary.
    bodyChunks: ?@import("http.zig").Response = null,
    chunkBuffer: [8192]u8 = undefined,
    chunk: []const u8 = &.{},
    chunkPosition: usize = 0,
    bodyChunkObserver: ?struct { context: ?*anyopaque, call: *const fn (?*anyopaque, []const u8) anyerror!void } = null,
    line: std.ArrayList(u8) = .empty,
    data: std.ArrayList(u8) = .empty,
    raw: std.ArrayList(u8) = .empty,
    event: []const u8 = "",
    id: []const u8 = "",
    retry: ?u64 = null,
    owned_event: ?[]u8 = null,
    owned_id: ?[]u8 = null,
    had_data: bool = false,
    dispatched: bool = false,
    skip_lf: bool = false,
    first: bool = true,
    ended: bool = false,
    flush_residual: bool = false,
    firstDataLineOnly: bool = false,
    dispatchEventWithoutData: bool = false,
    utf8PerLine: bool = false,
    stripBomPerLine: bool = false,
    collectRawLines: bool = false,
    requireCompleteEvent: bool = false,
    pending_non_whitespace: bool = false,
    framing: RecordFraming = .lines,
    requireDataColon: bool = false,
    trimDataLineStart: bool = false,
    pub fn init(a: std.mem.Allocator, reader: *std.Io.Reader, flush_residual: bool) @This() {
        return .{ .allocator = a, .reader = reader, .flush_residual = flush_residual };
    }
    pub fn deinit(self: *@This()) void {
        self.line.deinit(self.allocator);
        self.data.deinit(self.allocator);
        self.raw.deinit(self.allocator);
        if (self.owned_event) |v| self.allocator.free(v);
        if (self.owned_id) |v| self.allocator.free(v);
        self.* = undefined;
    }
    const Field = enum { data, event, id, retry };
    fn takeByte(self: *@This()) !u8 {
        const response = self.bodyChunks orelse return self.reader.takeByte();
        while (self.chunkPosition == self.chunk.len) {
            self.chunk = (try response.nextBodyChunk(&self.chunkBuffer)) orelse return error.EndOfStream;
            self.chunkPosition = 0;
            if (self.bodyChunkObserver) |observer| try observer.call(observer.context, self.chunk);
            // consumeLine consumes a CR immediately. Only an LF already in
            // that same logical chunk is swallowed as part of CRLF.
            self.skip_lf = false;
        }
        const byte = self.chunk[self.chunkPosition];
        self.chunkPosition += 1;
        return byte;
    }
    fn processLine(self: *@This()) !bool {
        var decoded: ?[]u8 = null;
        defer if (decoded) |bytes| self.allocator.free(bytes);
        const bytes = if (self.utf8PerLine) value: {
            decoded = try std.fmt.allocPrint(self.allocator, "{f}", .{std.unicode.fmtUtf8(self.line.items)});
            break :value decoded.?;
        } else self.line.items;
        var line = std.Io.Reader.fixed(bytes);
        if (self.first or self.stripBomPerLine) {
            self.first = false;
            if (std.mem.startsWith(u8, bytes, &.{ 0xef, 0xbb, 0xbf })) line.toss(3);
        }
        if (line.bufferedLen() == 0) {
            self.pending_non_whitespace = false;
            const dispatch = self.had_data or (self.dispatchEventWithoutData and self.event.len > 0);
            if (!dispatch) self.event = "";
            return dispatch;
        }
        if (self.collectRawLines) {
            if (self.raw.items.len > 0) try self.raw.appendSlice(self.allocator, "\\n");
            try self.raw.appendSlice(self.allocator, line.buffered());
        }
        if (std.mem.trim(u8, line.buffered(), &std.ascii.whitespace).len > 0) self.pending_non_whitespace = true;
        if (try line.peekByte() == ':') return false;
        const has_colon = std.mem.findScalar(u8, line.buffered(), ':') != null;
        const name = (try line.takeDelimiter(':')) orelse return false;
        const field = std.meta.stringToEnum(Field, name) orelse return false;
        if (field == .data and self.requireDataColon and !has_colon) return false;
        if ((line.peekByte() catch 0) == ' ') line.toss(1);
        const trimmed = if (field == .data and self.trimDataLineStart) try unicode.trimWhitespaceStart(self.allocator, line.buffered()) else null;
        defer if (trimmed) |value| self.allocator.free(value);
        const value = trimmed orelse line.buffered();
        switch (field) {
            .data => {
                if (self.firstDataLineOnly and self.had_data) return false;
                if (self.had_data) try self.data.append(self.allocator, '\n');
                try self.data.appendSlice(self.allocator, value);
                self.had_data = true;
            },
            .event => {
                const copy = try self.allocator.dupe(u8, value);
                if (self.owned_event) |old| self.allocator.free(old);
                self.owned_event = copy;
                self.event = copy;
            },
            .id => {
                if (std.mem.findScalar(u8, value, 0) == null) {
                    const copy = try self.allocator.dupe(u8, value);
                    if (self.owned_id) |old| self.allocator.free(old);
                    self.owned_id = copy;
                    self.id = copy;
                }
            },
            .retry => {
                if (value.len > 0) {
                    const digits = for (value) |byte| {
                        if (!std.ascii.isDigit(byte)) break false;
                    } else true;
                    if (digits) self.retry = std.fmt.parseInt(u64, value, 10) catch self.retry;
                }
            },
        }
        return false;
    }
    /// Result borrows the decoder until the next call. id/retry persist per SSE.
    pub fn next(self: *@This()) !?Event {
        if (self.ended) return null;
        if (self.dispatched) {
            self.data.clearRetainingCapacity();
            self.raw.clearRetainingCapacity();
            self.had_data = false;
            self.dispatched = false;
            self.event = "";
        }
        if (self.framing != .lines) return self.nextRecord();
        while (true) {
            const byte = self.takeByte() catch |err| switch (err) {
                error.EndOfStream => {
                    self.ended = true;
                    if (self.requireCompleteEvent and (self.pending_non_whitespace or std.mem.trim(u8, self.line.items, &std.ascii.whitespace).len > 0)) return error.IncompleteSseEvent;
                    if (self.flush_residual) {
                        if (self.line.items.len > 0) {
                            _ = try self.processLine();
                            self.line.clearRetainingCapacity();
                        }
                        if (self.had_data or (self.dispatchEventWithoutData and self.event.len > 0)) return self.current();
                    }
                    return null;
                },
                else => return err,
            };
            if (self.skip_lf) {
                self.skip_lf = false;
                if (byte == '\n') continue;
            }
            if (byte == '\r' or byte == '\n') {
                self.skip_lf = byte == '\r';
                const dispatch = try self.processLine();
                self.line.clearRetainingCapacity();
                if (dispatch) return self.current();
            } else try self.line.append(self.allocator, byte);
        }
    }
    fn recordBoundary(self: *const @This()) usize {
        if (std.mem.endsWith(u8, self.line.items, "\n\n")) return 2;
        if (self.framing == .google) {
            if (std.mem.endsWith(u8, self.line.items, "\r\r")) return 2;
            if (std.mem.endsWith(u8, self.line.items, "\r\n\r\n")) return 4;
        }
        return 0;
    }
    fn decodedRecord(self: *@This(), input: []const u8) ![]u8 {
        const decoded = try std.fmt.allocPrint(self.allocator, "{f}", .{std.unicode.fmtUtf8(input)});
        errdefer self.allocator.free(decoded);
        // A streaming TextDecoder strips only the initial BOM. Formatting is
        // delegated to std; protocol delimiters cannot split a UTF8 sequence.
        if (self.first) {
            self.first = false;
            if (std.mem.startsWith(u8, decoded, &.{ 0xef, 0xbb, 0xbf })) {
                std.mem.copyForwards(u8, decoded, decoded[3..]);
                return self.allocator.realloc(decoded, decoded.len - 3);
            }
        }
        return decoded;
    }
    fn processRecord(self: *@This()) !bool {
        const decoded = try self.decodedRecord(self.line.items);
        defer self.allocator.free(decoded);
        self.data.clearRetainingCapacity();
        self.had_data = false;
        if (self.framing == .google) {
            const record = try unicode.trimWhitespace(self.allocator, decoded);
            defer self.allocator.free(record);
            if (!std.mem.startsWith(u8, record, "data:")) return false;
            const payload = try unicode.trimWhitespace(self.allocator, record[5..]);
            defer self.allocator.free(payload);
            try self.data.appendSlice(self.allocator, payload);
            self.had_data = true;
            return true;
        }
        var lines = std.mem.splitScalar(u8, decoded, '\n');
        while (lines.next()) |line| {
            if (!std.mem.startsWith(u8, line, "data:")) continue;
            const payload = try unicode.trimWhitespace(self.allocator, line[5..]);
            defer self.allocator.free(payload);
            if (self.had_data) try self.data.append(self.allocator, '\n');
            try self.data.appendSlice(self.allocator, payload);
            self.had_data = true;
            if (self.framing == .pi) break;
        }
        return self.had_data;
    }
    fn nextRecord(self: *@This()) !?Event {
        while (true) {
            const byte = self.takeByte() catch |err| switch (err) {
                error.EndOfStream => {
                    self.ended = true;
                    if (self.framing == .google) {
                        // Withhold only a suffix validated as an unfinished
                        // scalar prefix. Malformed bytes still reach std's
                        // replacement formatter and the nonblank EOF check.
                        const decoded = try self.decodedRecord(googleEofPrefix(self.line.items));
                        defer self.allocator.free(decoded);
                        if (!try unicode.isBlank(decoded)) return error.IncompleteSseEvent;
                        return null;
                    }
                    if (self.flush_residual and self.line.items.len > 0 and try self.processRecord()) return self.current();
                    return null;
                },
                else => return err,
            };
            // Pi normalizes CRLF before looking for LF/LF. Bare CR remains
            // record data; Codex leaves CRLF untouched, as its source does.
            if (self.framing == .pi and byte == '\n' and std.mem.endsWith(u8, self.line.items, "\r")) self.line.items[self.line.items.len - 1] = '\n' else try self.line.append(self.allocator, byte);
            const boundary = self.recordBoundary();
            if (boundary > 0) {
                self.line.items.len -= boundary;
                const dispatch = try self.processRecord();
                self.line.clearRetainingCapacity();
                if (dispatch) return self.current();
            }
        }
    }
    fn current(self: *@This()) Event {
        self.dispatched = true;
        return .{ .event = if (self.event.len == 0) "message" else self.event, .data = self.data.items, .id = self.id, .retry = self.retry, .raw = self.raw.items };
    }
};
/// Typed server/test framing; the caller serializes structured payloads first.
pub fn writeEvent(writer: *std.Io.Writer, event: Event) !void {
    if (std.mem.findAny(u8, event.event, "\r\n") != null or std.mem.findAny(u8, event.id, "\r\n\x00") != null) return error.InvalidSseField;
    if (event.event.len > 0) try writer.print("event: {s}\n", .{event.event});
    if (event.id.len > 0) try writer.print("id: {s}\n", .{event.id});
    if (event.retry) |milliseconds| try writer.print("retry: {d}\n", .{milliseconds});
    var lines = std.Io.Reader.fixed(event.data);
    while (true) {
        const length = std.mem.findAny(u8, lines.buffered(), "\r\n") orelse lines.bufferedLen();
        try writer.writeAll("data: ");
        try writer.writeAll(try lines.take(length));
        try writer.writeByte('\n');
        if (lines.bufferedLen() == 0) break;
        const delimiter = try lines.takeByte();
        if (delimiter == '\r' and (lines.peekByte() catch 0) == '\n') lines.toss(1);
    }
    try writer.writeByte('\n');
}
