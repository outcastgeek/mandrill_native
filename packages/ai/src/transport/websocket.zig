const std = @import("std");
pub const Opcode = enum(u4) { continuation = 0, text = 1, binary = 2, close = 8, ping = 9, pong = 10 };
pub const Frame = struct { final: bool, opcode: Opcode, payload: []const u8 };
pub const Close = struct { code: u16, reason: []const u8 };
fn validateClose(payload: []const u8) !void {
    if (payload.len == 1) return error.InvalidWebSocketClose;
    if (payload.len < 2) return;
    const code = std.mem.readInt(u16, payload[0..2], .big);
    const valid = (code >= 1000 and code <= 1014 and code != 1004 and code != 1005 and code != 1006) or (code >= 3000 and code <= 4999);
    if (!valid) return error.InvalidWebSocketCloseCode;
    if (!std.unicode.utf8ValidateSlice(payload[2..])) return error.InvalidWebSocketUtf8;
}
pub const Codec = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    reader: *std.Io.Reader,
    writer: *std.Io.Writer,
    fragments: std.ArrayList(u8) = .empty,
    message: std.ArrayList(u8) = .empty,
    fragment_opcode: ?Opcode = null,
    closed: ?Close = null,
    closeSent: bool = false,
    maxMessageBytes: std.Io.Limit = .unlimited,
    writeMutex: ?*std.Io.Mutex = null,
    pub fn deinit(self: *@This()) void {
        self.fragments.deinit(self.allocator);
        self.message.deinit(self.allocator);
        self.* = undefined;
    }
    pub fn readFrame(self: *@This()) !Frame {
        const first = try self.reader.takeByte();
        const second = try self.reader.takeByte();
        if (first & 0x70 != 0 or second & 0x80 != 0) return error.InvalidWebSocketFrame;
        const opcode = std.enums.fromInt(Opcode, @as(u4, @truncate(first))) orelse return error.InvalidWebSocketOpcode;
        const final = first & 0x80 != 0;
        var length: u64 = second & 0x7f;
        if (length == 126) {
            length = try self.reader.takeInt(u16, .big);
            if (length < 126) return error.NonCanonicalWebSocketLength;
        } else if (length == 127) {
            length = try self.reader.takeInt(u64, .big);
            if (length < 65536 or length >> 63 != 0) return error.InvalidWebSocketLength;
        }
        if (@backingInt(opcode) >= 8 and (!final or length > 125)) return error.InvalidWebSocketControlFrame;
        if (length > std.math.maxInt(usize)) return error.WebSocketMessageTooLong;
        if (@backingInt(opcode) < 8) {
            const total = std.math.add(usize, if (opcode == .continuation) self.fragments.items.len else 0, @intCast(length)) catch return error.WebSocketMessageTooLong;
            if (self.maxMessageBytes.toInt()) |limit| if (total > limit) return error.WebSocketMessageTooLong;
        }
        self.message.clearRetainingCapacity();
        try self.message.resize(self.allocator, @intCast(length));
        try self.reader.readSliceAll(self.message.items);
        if (opcode == .close) try validateClose(self.message.items);
        return .{ .final = final, .opcode = opcode, .payload = self.message.items };
    }
    pub fn send(self: *@This(), opcode: Opcode, payload: []const u8) !void {
        if (self.writeMutex) |mutex| try mutex.lock(self.io);
        defer if (self.writeMutex) |mutex| mutex.unlock(self.io);
        if (self.closeSent) {
            if (opcode == .close) return;
            return error.WebSocketClosed;
        }
        if (@backingInt(opcode) >= 8 and payload.len > 125) return error.InvalidWebSocketControlFrame;
        if (opcode == .close) try validateClose(payload);
        if (opcode == .text and !std.unicode.utf8ValidateSlice(payload)) return error.InvalidWebSocketUtf8;
        var mask: [4]u8 = undefined;
        try self.io.randomSecure(&mask);
        if (opcode == .close) self.closeSent = true;
        try self.writer.writeByte(0x80 | @as(u8, @backingInt(opcode)));
        if (payload.len < 126) try self.writer.writeByte(0x80 | @as(u8, @intCast(payload.len))) else if (payload.len <= 65535) {
            try self.writer.writeByte(0x80 | 126);
            try self.writer.writeInt(u16, @intCast(payload.len), .big);
        } else {
            try self.writer.writeByte(0x80 | 127);
            try self.writer.writeInt(u64, payload.len, .big);
        }
        try self.writer.writeAll(&mask);
        var masked: [4096]u8 = undefined;
        var offset: usize = 0;
        while (offset < payload.len) {
            const n = @min(masked.len, payload.len - offset);
            for (masked[0..n], payload[offset..][0..n], 0..) |*out, byte, i| out.* = byte ^ mask[(offset + i) % 4];
            try self.writer.writeAll(masked[0..n]);
            offset += n;
        }
        try self.writer.flush();
    }
    /// Returns complete data messages, replying to ping frames along the way.
    pub fn next(self: *@This()) !?Frame {
        while (true) {
            const frame = try self.readFrame();
            switch (frame.opcode) {
                .ping => {
                    try self.send(.pong, frame.payload);
                    continue;
                },
                .pong => continue,
                .close => {
                    self.closed = if (frame.payload.len >= 2) .{ .code = std.mem.readInt(u16, frame.payload[0..2], .big), .reason = frame.payload[2..] } else .{ .code = 1005, .reason = "" };
                    self.send(.close, frame.payload) catch |err| {
                        if (err == error.Canceled) return err;
                    };
                    return null;
                },
                .continuation => {
                    const opcode = self.fragment_opcode orelse return error.UnexpectedWebSocketContinuation;
                    try self.fragments.appendSlice(self.allocator, frame.payload);
                    if (frame.final) {
                        self.fragment_opcode = null;
                        if (opcode == .text and !std.unicode.utf8ValidateSlice(self.fragments.items)) return error.InvalidWebSocketUtf8;
                        return .{ .final = true, .opcode = opcode, .payload = self.fragments.items };
                    }
                },
                .text, .binary => {
                    if (self.fragment_opcode != null) return error.InterleavedWebSocketMessages;
                    if (frame.final) {
                        if (frame.opcode == .text and !std.unicode.utf8ValidateSlice(frame.payload)) return error.InvalidWebSocketUtf8;
                        return frame;
                    }
                    self.fragment_opcode = frame.opcode;
                    self.fragments.clearRetainingCapacity();
                    try self.fragments.appendSlice(self.allocator, frame.payload);
                },
            }
        }
    }
};
pub fn acceptKey(a: std.mem.Allocator, key: []const u8) ![]const u8 {
    var sha = std.crypto.hash.Sha1.init(.{});
    sha.update(key);
    sha.update("258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    var digest: [20]u8 = undefined;
    sha.final(&digest);
    const out = try a.alloc(u8, std.base64.standard.Encoder.calcSize(digest.len));
    _ = std.base64.standard.Encoder.encode(out, &digest);
    return out;
}
/// An upgraded connection owned by its transport. HTTP/TLS parsing remains in
/// std.http.Client; the codec consumes only WebSocket frames.
pub const Connection = struct {
    reader: *std.Io.Reader,
    writer: *std.Io.Writer,
    owner: *anyopaque,
    status: u16 = 101,
    headers: @import("../types.zig").ProviderHeaders = &.{},
    releaseFn: *const fn (*anyopaque) void,
    shutdownFn: ?*const fn (*anyopaque) void = null,
    ioErrorFn: ?*const fn (*anyopaque, anyerror) anyerror = null,
    pub fn shutdown(self: *@This()) void {
        if (self.shutdownFn) |call| call(self.owner);
    }
    pub fn ioError(self: @This(), err: anyerror) anyerror {
        return if (self.ioErrorFn) |call| call(self.owner, err) else err;
    }
    pub fn deinit(self: *@This()) void {
        self.releaseFn(self.owner);
        self.* = undefined;
    }
};

/// One joined reader owns framing for the entire socket lifetime. It answers
/// control frames even between request leases; consumers never cancel a read
/// halfway through a frame or TLS record to acquire a cached connection.
pub const Session = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    connection: Connection,
    codec: Codec,
    mutex: std.Io.Mutex = .init,
    writeMutex: std.Io.Mutex = .init,
    changed: std.Io.Condition = .init,
    queue: std.ArrayList(Frame) = .empty,
    active: bool = true,
    ended: bool = false,
    stopped: bool = false,
    failure: ?anyerror = null,
    closed: ?Close = null,
    pump: ?std.Io.Future(void) = null,
    pub fn open(a: std.mem.Allocator, io: std.Io, connection: Connection) !*Session {
        return openWithLimit(a, io, connection, .unlimited);
    }
    pub fn openWithLimit(a: std.mem.Allocator, io: std.Io, connection: Connection, max_message_bytes: std.Io.Limit) !*Session {
        const self = try a.create(Session);
        errdefer a.destroy(self);
        self.* = .{ .allocator = a, .io = io, .connection = connection, .codec = .{ .allocator = a, .io = io, .reader = connection.reader, .writer = connection.writer, .maxMessageBytes = max_message_bytes } };
        self.codec.writeMutex = &self.writeMutex;
        self.pump = try io.concurrent(read, .{self});
        return self;
    }
    fn read(self: *Session) void {
        var failure: ?anyerror = null;
        while (true) {
            const frame = (self.codec.next() catch |err| {
                failure = self.connection.ioError(err);
                break;
            }) orelse break;
            self.mutex.lock(self.io) catch |err| {
                failure = err;
                break;
            };
            defer self.mutex.unlock(self.io);
            if (self.ended) break;
            if (!self.active) continue;
            const payload = self.allocator.dupe(u8, frame.payload) catch {
                failure = error.OutOfMemory;
                break;
            };
            self.queue.append(self.allocator, .{ .final = true, .opcode = frame.opcode, .payload = payload }) catch {
                self.allocator.free(payload);
                failure = error.OutOfMemory;
                break;
            };
            self.changed.broadcast(self.io);
        }
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        if (self.codec.closed) |info| {
            if (self.allocator.dupe(u8, info.reason)) |reason| {
                if (self.closed) |old| self.allocator.free(old.reason);
                self.closed = .{ .code = info.code, .reason = reason };
            } else |_| {
                failure = error.OutOfMemory;
            }
        }
        if (!self.ended) {
            self.failure = failure;
            self.ended = true;
        }
        self.changed.broadcast(self.io);
        if (failure) |err| if (err == error.Canceled) self.io.recancel();
    }
    fn clearQueue(self: *Session) void {
        for (self.queue.items) |frame| self.allocator.free(frame.payload);
        self.queue.clearRetainingCapacity();
    }
    pub fn setActive(self: *Session, active: bool) void {
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        self.active = active;
        if (!active) self.clearQueue();
    }
    pub fn isOpen(self: *Session) bool {
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        return !self.ended;
    }
    pub fn send(self: *Session, payload: []const u8) !void {
        return self.codec.send(.text, payload);
    }
    /// Returned payload belongs to `a`; queued buffers are released as consumed.
    pub fn next(self: *Session, a: std.mem.Allocator) !?Frame {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        while (self.queue.items.len == 0 and !self.ended) try self.changed.wait(self.io, &self.mutex);
        if (self.queue.items.len > 0) {
            const frame = self.queue.items[0];
            const payload = try a.dupe(u8, frame.payload);
            _ = self.queue.orderedRemove(0);
            self.allocator.free(frame.payload);
            return .{ .final = true, .opcode = frame.opcode, .payload = payload };
        }
        if (self.failure) |err| return err;
        return null;
    }
    pub fn closeInfo(self: *Session, a: std.mem.Allocator) !?Close {
        try self.mutex.lock(self.io);
        defer self.mutex.unlock(self.io);
        return if (self.closed) |info| .{ .code = info.code, .reason = try a.dupe(u8, info.reason) } else null;
    }
    fn writeClose(self: *Session, reason: []const u8) !void {
        const protection = self.io.swapCancelProtection(.unblocked);
        defer _ = self.io.swapCancelProtection(protection);
        if (reason.len > 123) return error.InvalidWebSocketClose;
        var payload: [125]u8 = undefined;
        var writer = std.Io.Writer.fixed(&payload);
        try writer.writeInt(u16, 1000, .big);
        try writer.writeAll(reason);
        try self.codec.send(.close, payload[0..writer.end]);
    }
    fn closeTimeout(io: std.Io) std.Io.Cancelable!void {
        const protection = io.swapCancelProtection(.unblocked);
        defer _ = io.swapCancelProtection(protection);
        try io.sleep(.fromMilliseconds(1000), .awake);
    }
    /// Best-effort normal Close, with a bounded write and no peer-handshake
    /// wait. Forced shutdown remains available through stop(). Both workers
    /// join before any caller may release the connection or reason storage.
    pub fn close(self: *Session, reason: []const u8) void {
        const protection = self.io.swapCancelProtection(.blocked);
        defer _ = self.io.swapCancelProtection(protection);
        self.mutex.lockUncancelable(self.io);
        if (self.stopped) {
            self.mutex.unlock(self.io);
            return;
        }
        if (self.closed == null) if (self.allocator.dupe(u8, reason)) |copy| {
            self.closed = .{ .code = 1000, .reason = copy };
        } else |_| {};
        self.mutex.unlock(self.io);
        const Outcome = union(enum) { sent: anyerror!void, timeout: std.Io.Cancelable!void };
        var storage: [2]Outcome = undefined;
        var select = std.Io.Select(Outcome).init(self.io, &storage);
        defer self.stop();
        defer select.cancelDiscard();
        select.concurrent(.sent, writeClose, .{ self, reason }) catch return;
        select.concurrent(.timeout, closeTimeout, .{self.io}) catch {
            self.connection.shutdown();
            return;
        };
        const outcome = select.await() catch {
            self.connection.shutdown();
            return;
        };
        switch (outcome) {
            .sent => |result| result catch {},
            .timeout => |result| {
                result catch {};
                self.connection.shutdown();
            },
        }
    }
    /// Stop/join the reader before the connection owner releases its storage.
    /// The connection itself remains owned by the transport/cache caller.
    pub fn stop(self: *Session) void {
        self.mutex.lockUncancelable(self.io);
        if (self.stopped) {
            self.mutex.unlock(self.io);
            return;
        }
        self.stopped = true;
        self.ended = true;
        self.changed.broadcast(self.io);
        self.mutex.unlock(self.io);
        self.connection.shutdown();
        if (self.pump) |*task| {
            task.cancel(self.io);
            self.pump = null;
        }
    }
    pub fn deinit(self: *Session) void {
        self.stop();
        self.clearQueue();
        self.queue.deinit(self.allocator);
        if (self.closed) |info| self.allocator.free(info.reason);
        self.codec.deinit();
        const a = self.allocator;
        a.destroy(self);
    }
};
