//! Approved ECMAScript string compatibility over Zig 0.17 std.json.Scanner.
//! Only surrogate transitions differ; std.json owns grammar and token parsing.
const std = @import("std");
const Token = std.json.Token;
pub const Scanner = struct {
    raw: std.json.Scanner,
    input: []const u8 = "",
    cursor: usize = 0,
    supplied: usize = 0,
    is_end_of_input: bool = false,
    high: ?u16 = null,
    queuedUnit: ?u21 = null,
    terminal: bool = false,
    fragment: [4]u8 = undefined,
    fragmentLen: usize = 0,
    fragmentCursor: usize = 0,
    unicode: [4]u8 = undefined,
    unicodeLen: usize = 0,
    unicodeNeeded: usize = 0,
    pub const NextError = std.json.Scanner.NextError;
    pub const PeekError = std.json.Scanner.PeekError;
    pub const AllocError = std.json.Scanner.AllocError || error{BufferUnderrun};
    pub const AllocIntoArrayListError = AllocError;
    pub const SkipError = NextError;
    pub fn initStreaming(a: std.mem.Allocator) @This() {
        return .{ .raw = .initStreaming(a) };
    }
    pub fn initCompleteInput(a: std.mem.Allocator, input: []const u8) @This() {
        var self = initStreaming(a);
        self.feedInput(input);
        self.endInput();
        return self;
    }
    pub fn deinit(self: *@This()) void {
        self.raw.deinit();
        self.* = undefined;
    }
    pub fn feedInput(self: *@This(), input: []const u8) void {
        std.debug.assert(self.cursor == self.input.len and !self.is_end_of_input);
        self.input = input;
        self.cursor = 0;
        self.supplied = 0;
    }
    pub fn endInput(self: *@This()) void {
        self.is_end_of_input = true;
    }
    pub fn stackHeight(self: *const @This()) usize {
        return self.raw.stackHeight();
    }
    fn position(self: *@This()) void {
        self.cursor = self.supplied - (self.raw.input.len - self.raw.cursor);
    }
    fn feed(self: *@This()) bool {
        if (self.supplied < self.input.len) {
            self.raw.feedInput(self.input[self.supplied..][0..1]);
            self.supplied += 1;
            return true;
        }
        if (self.is_end_of_input and !self.raw.is_end_of_input) {
            self.raw.endInput();
            return true;
        }
        return false;
    }
    fn encoded(code: u21) Token {
        var bytes: [4]u8 = undefined;
        const count = std.unicode.wtf8Encode(code, &bytes) catch unreachable;
        return switch (count) {
            1 => .{ .partial_string_escaped_1 = bytes[0..1].* },
            2 => .{ .partial_string_escaped_2 = bytes[0..2].* },
            3 => .{ .partial_string_escaped_3 = bytes[0..3].* },
            4 => .{ .partial_string_escaped_4 = bytes },
            else => unreachable,
        };
    }
    fn unit(self: *@This(), code: u21) ?Token {
        if (self.high) |high| {
            self.high = null;
            if (code >= 0xdc00 and code <= 0xdfff) {
                const pair = [_]u16{ high, @intCast(code) };
                return encoded(std.unicode.utf16DecodeSurrogatePair(&pair) catch unreachable);
            }
            self.queuedUnit = code;
            return encoded(high);
        }
        if (code >= 0xd800 and code <= 0xdbff) {
            self.high = @intCast(code);
            return null;
        }
        return encoded(code);
    }
    fn setFragment(self: *@This(), bytes: []const u8) void {
        std.debug.assert(bytes.len <= self.fragment.len);
        @memcpy(self.fragment[0..bytes.len], bytes);
        self.fragmentLen = bytes.len;
        self.fragmentCursor = 0;
    }
    pub fn next(self: *@This()) NextError!Token {
        defer self.position();
        while (true) {
            if (self.queuedUnit) |code| {
                self.queuedUnit = null;
                if (self.unit(code)) |token| return token;
            }
            while (self.fragmentCursor < self.fragmentLen) {
                const byte = self.fragment[self.fragmentCursor];
                self.fragmentCursor += 1;
                if (self.unicodeLen == 0) self.unicodeNeeded = std.unicode.utf8ByteSequenceLength(byte) catch return error.SyntaxError;
                self.unicode[self.unicodeLen] = byte;
                self.unicodeLen += 1;
                if (self.unicodeLen == self.unicodeNeeded) {
                    const code = std.unicode.wtf8Decode(self.unicode[0..self.unicodeLen]) catch return error.SyntaxError;
                    self.unicodeLen = 0;
                    if (self.unit(code)) |token| return token;
                }
            }
            if (self.terminal) {
                if (self.unicodeLen != 0) return error.SyntaxError;
                if (self.high) |high| {
                    self.high = null;
                    return encoded(high);
                }
                self.terminal = false;
                return .{ .string = "" };
            }
            // Embedded JSON strings can contain literal WTF8 surrogate units.
            // Permit only ED A0..BF here; std still validates the last byte.
            if (self.raw.state == .string_utf8_second_to_last_byte_guard_against_surrogate_half and self.raw.cursor < self.raw.input.len and self.raw.input[self.raw.cursor] >= 0xa0 and self.raw.input[self.raw.cursor] <= 0xbf)
                self.raw.state = .string_utf8_second_to_last_byte;
            const before = self.raw.state;
            const before_cursor = self.raw.cursor;
            const token = self.raw.next() catch |err| {
                if (err == error.BufferUnderrun) {
                    // One-byte feeds expose this state before the scanner can
                    // consume any part of a following escape. No replay needed.
                    if (self.raw.state == .string_surrogate_half) {
                        const high = self.raw.utf16_code_units[0];
                        self.raw.state = .string;
                        self.raw.value_start = self.raw.cursor;
                        if (self.unit(high)) |emitted| return emitted;
                    }
                    if (self.feed()) continue;
                    return error.BufferUnderrun;
                }
                // Accept only a fully scanner-decoded standalone low unit.
                // A bad hex digit has not advanced cursor and cannot enter.
                if (err == error.SyntaxError and before == .string_backslash_u_3 and self.raw.state == .string_backslash_u_3 and self.raw.cursor == before_cursor + 1 and std.unicode.utf16IsLowSurrogate(self.raw.utf16_code_units[0])) {
                    const low = self.raw.utf16_code_units[0];
                    self.raw.state = .string;
                    self.raw.value_start = self.raw.cursor;
                    if (self.unit(low)) |emitted| return emitted;
                    continue;
                }
                return err;
            };
            switch (token) {
                .partial_string => |bytes| self.setFragment(bytes),
                .partial_string_escaped_1 => |bytes| self.setFragment(&bytes),
                .partial_string_escaped_2 => |bytes| self.setFragment(&bytes),
                .partial_string_escaped_3 => |bytes| self.setFragment(&bytes),
                .partial_string_escaped_4 => |bytes| self.setFragment(&bytes),
                .string => |bytes| {
                    self.setFragment(bytes);
                    self.terminal = true;
                },
                else => return token,
            }
        }
    }
    /// Called only when a partial-value consumer stops at its final supplied
    /// chunk. A complete escaped high unit belongs to that partial string.
    pub fn finishPartialString(self: *@This(), a: std.mem.Allocator, out: *std.ArrayList(u8)) !void {
        if (self.high) |high| {
            var bytes: [4]u8 = undefined;
            const n = std.unicode.wtf8Encode(high, &bytes) catch unreachable;
            try out.appendSlice(a, bytes[0..n]);
            self.high = null;
        }
        if (self.unicodeLen > 0) try out.appendSlice(a, self.unicode[0..self.unicodeLen]);
    }
    pub fn peekNextTokenType(self: *@This()) PeekError!std.json.TokenType {
        defer self.position();
        if (self.high != null or self.queuedUnit != null or self.terminal or self.fragmentCursor < self.fragmentLen or self.unicodeLen > 0) return .string;
        while (true) return self.raw.peekNextTokenType() catch |err| {
            if (err == error.BufferUnderrun and self.feed()) continue;
            return err;
        };
    }
    fn append(list: *std.array_list.Managed(u8), bytes: []const u8, max: usize) !void {
        if (bytes.len > max or list.items.len > max - bytes.len) return error.ValueTooLong;
        try list.appendSlice(bytes);
    }
    pub fn allocNextIntoArrayListMax(self: *@This(), list: *std.array_list.Managed(u8), when: std.json.AllocWhen, max: usize) AllocIntoArrayListError!?[]const u8 {
        while (true) switch (try self.next()) {
            .partial_number, .partial_string => |bytes| try append(list, bytes, max),
            .partial_string_escaped_1 => |bytes| try append(list, &bytes, max),
            .partial_string_escaped_2 => |bytes| try append(list, &bytes, max),
            .partial_string_escaped_3 => |bytes| try append(list, &bytes, max),
            .partial_string_escaped_4 => |bytes| try append(list, &bytes, max),
            .number, .string => |bytes| {
                if (when == .alloc_if_needed and list.items.len == 0) return bytes;
                try append(list, bytes, max);
                return null;
            },
            else => unreachable,
        };
    }
    pub fn nextAllocMax(self: *@This(), a: std.mem.Allocator, when: std.json.AllocWhen, max: usize) AllocError!Token {
        const kind = try self.peekNextTokenType();
        if (kind != .number and kind != .string) return self.next();
        var list = std.array_list.Managed(u8).init(a);
        errdefer list.deinit();
        if (try self.allocNextIntoArrayListMax(&list, when, max)) |bytes| return if (kind == .string) .{ .string = bytes } else .{ .number = bytes };
        const owned = try list.toOwnedSlice();
        return if (kind == .string) .{ .allocated_string = owned } else .{ .allocated_number = owned };
    }
    pub fn nextAlloc(self: *@This(), a: std.mem.Allocator, when: std.json.AllocWhen) AllocError!Token {
        return self.nextAllocMax(a, when, std.json.default_max_value_len);
    }
    pub fn skipUntilStackHeight(self: *@This(), height: usize) NextError!void {
        while (true) switch (try self.next()) {
            .object_end, .array_end => if (self.stackHeight() == height) return,
            .end_of_document => return error.UnexpectedEndOfInput,
            else => {},
        };
    }
    pub fn skipValue(self: *@This()) NextError!void {
        switch (try self.peekNextTokenType()) {
            .object_begin, .array_begin => return self.skipUntilStackHeight(self.stackHeight()),
            .number, .string => while (true) switch (try self.next()) {
                .number, .string => return,
                else => {},
            },
            else => {
                _ = try self.next();
            },
        }
    }
};
