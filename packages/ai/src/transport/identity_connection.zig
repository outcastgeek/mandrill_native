//! An acquired standard HTTP connection carrying target TLS over a transport
//! or proxy tunnel, with an optional caller-supplied TLS identity. HTTP codecs remain
//! in use. The same caller Io backs direct sockets and proxy tunnels.
const std = @import("std");
const Tls = @import("tls_client.zig");
const Identity = @import("tls_identity.zig").Identity;
const Reader = std.Io.Reader;
const Writer = std.Io.Writer;
pub const Connection = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    arena: std.heap.ArenaAllocator,
    client: std.http.Client,
    connection: ?*std.http.Client.Connection = null,
    owns_connection: bool = false,
    rejected_request: ?std.http.Client.Request = null,
    rejected_response: ?std.http.Client.Response = null,
    identity: Identity = undefined,
    inner: Tls = undefined,
    standard_inner: std.crypto.tls.Client = undefined,
    has_identity: bool = false,
    ready: bool = false,
    bridged: bool = false,
    outer_tls: ?std.crypto.tls.Client = null,
    base_tls: ?*std.crypto.tls.Client = null,
    plain_reader: ?std.Io.net.Stream.Reader = null,
    plain_writer: ?std.Io.net.Stream.Writer = null,
    tunnel_writer: Writer = undefined,
    read_failure: ?anyerror = null,
    write_failure: ?anyerror = null,
    pub fn open(parent: std.mem.Allocator, io: std.Io, settings: *const std.http.Client, target: std.Uri, certificate: []const u8, private_key: []const u8, proxy: ?std.Uri) !*@This() {
        return openWithIdentity(parent, io, settings, target, certificate, private_key, proxy);
    }
    pub fn openTunnel(parent: std.mem.Allocator, io: std.Io, settings: *const std.http.Client, target: std.Uri, proxy: std.Uri) !*@This() {
        return openWithIdentity(parent, io, settings, target, null, null, proxy);
    }
    fn openWithIdentity(parent: std.mem.Allocator, io: std.Io, settings: *const std.http.Client, target: std.Uri, certificate: ?[]const u8, private_key: ?[]const u8, proxy: ?std.Uri) !*@This() {
        const self = try parent.create(@This());
        self.* = .{ .allocator = parent, .io = io, .arena = .init(parent), .client = .{ .allocator = parent, .io = io } };
        errdefer self.destroy();
        const a = self.arena.allocator();
        self.client.allocator = a;
        self.client.read_buffer_size = @max(settings.read_buffer_size, Tls.min_buffer_len);
        self.client.write_buffer_size = settings.write_buffer_size;
        self.client.tls_buffer_size = @max(settings.tls_buffer_size, Tls.min_buffer_len);
        self.has_identity = certificate != null;
        if (certificate) |chain| self.identity = try Identity.load(a, io, chain, private_key orelse return error.MissingPrivateKey);
        const now = std.Io.Clock.real.now(io);
        try self.client.ca_bundle.rescan(a, io, now);
        self.client.now = now;
        if (proxy) |url| try @import("proxy.zig").configureHttpProxy(&self.client, a, url);
        var host_buffer: [std.Io.net.HostName.max_len]u8 = undefined;
        const host = try std.Io.net.HostName.fromUri(target, &host_buffer);
        // Acquire the transport/tunnel first. Its TLS layer, if present, is
        // the HTTPS proxy's identity; the target's TLS layer is added below.
        const connection = if (self.client.http_proxy) |configured|
            try self.client.connectTcp(configured.host, configured.port, configured.protocol)
        else
            try self.client.connectTcp(host, target.port orelse 443, .plain);
        self.connection = connection;
        self.owns_connection = true;
        // Use standard HTTP framing for CONNECT, then add a distinct target
        // TLS layer. The proxy TLS handshake authenticates the proxy host.
        // A rejected tunnel never falls back to sending the target request.
        if (self.client.http_proxy) |configured| {
            var headers: std.ArrayList(std.http.Header) = .empty;
            try headers.append(a, .{ .name = "proxy-connection", .value = "close" });
            if (configured.authorization) |value| try headers.append(a, .{ .name = "proxy-authorization", .value = value });
            var connect_request = try self.client.request(.CONNECT, .{ .scheme = "http", .host = target.host, .port = target.port orelse 443 }, .{
                .connection = connection,
                .redirect_behavior = .unhandled,
                .extra_headers = headers.items,
                .handle_continue = false,
                .headers = .{ .authorization = .omit, .user_agent = .omit, .connection = .omit, .accept_encoding = .omit },
            });
            defer {
                if (connect_request.connection != null) {
                    connection.closing = true;
                    connect_request.reader.state = .ready;
                    self.owns_connection = false;
                    self.connection = null;
                }
                connect_request.deinit();
            }
            connect_request.sendBodiless() catch |err| return self.physicalError(err);
            const response = connect_request.receiveHead(&.{}) catch |err| return self.physicalError(err);
            if (response.head.status != .ok) {
                // The pinned agent closes the physical proxy socket and
                // replays only bytes buffered with the rejection head. Keep
                // that EOF boundary through a standard fixed reader, without
                // waiting for more body bytes or writing the target request.
                const reader = connection.reader();
                const buffered = try a.dupe(u8, reader.buffered());
                connection.stream_reader.stream.shutdown(io, .both) catch {};
                reader.* = .fixed(buffered);
                self.rejected_request = connect_request;
                self.rejected_response = response;
                self.rejected_response.?.request = &self.rejected_request.?;
                connect_request.connection = null;
                return self;
            }
            connect_request.connection = null;
        }
        const input: *Reader = if (connection.protocol == .tls) tls_input: {
            const base: *std.crypto.tls.Client = @fieldParentPtr("reader", connection.reader());
            self.base_tls = base;
            self.outer_tls = base.*;
            break :tls_input &self.outer_tls.?.reader;
        } else plain_input: {
            self.plain_reader = connection.stream_reader;
            self.plain_writer = connection.stream_writer;
            break :plain_input &self.plain_reader.?.interface;
        };
        // Give the inner TLS layer its required record-sized output buffer.
        // This writer also flushes every enclosing transport layer, including
        // the physical socket beneath an HTTPS proxy's TLS writer.
        self.tunnel_writer = .{ .buffer = try a.alloc(u8, self.client.tls_buffer_size), .vtable = &.{ .drain = tunnelDrain, .flush = tunnelFlush } };
        const output = &self.tunnel_writer;
        const read_buffer = try a.alloc(u8, self.client.tls_buffer_size + self.client.read_buffer_size);
        const write_buffer = try a.alloc(u8, self.client.tls_buffer_size);
        var entropy: [Tls.Options.entropy_len]u8 = undefined;
        try io.randomSecure(&entropy);
        if (self.has_identity) self.inner = Tls.init(input, output, .{
            .identity = &self.identity,
            .host = .{ .explicit = host.bytes },
            .ca = .{ .bundle = .{ .gpa = a, .io = io, .lock = &self.client.ca_bundle_lock, .bundle = &self.client.ca_bundle } },
            .read_buffer = read_buffer,
            .write_buffer = write_buffer,
            .entropy = &entropy,
            .realtime_now = now,
            .allow_truncation_attacks = true,
        }) catch |err| return self.physicalError(err) else self.standard_inner = std.crypto.tls.Client.init(input, output, .{
            .host = .{ .explicit = host.bytes },
            .ca = .{ .bundle = .{ .gpa = a, .io = io, .lock = &self.client.ca_bundle_lock, .bundle = &self.client.ca_bundle } },
            .read_buffer = read_buffer,
            .write_buffer = write_buffer,
            .entropy = &entropy,
            .realtime_now = now,
            .allow_truncation_attacks = true,
        }) catch |err| return self.physicalError(err);
        self.ready = true;
        const incoming = try a.alloc(u8, self.client.read_buffer_size);
        const outgoing = try a.alloc(u8, self.client.write_buffer_size);
        if (self.base_tls) |base| {
            base.reader = .{ .buffer = incoming, .seek = 0, .end = 0, .vtable = &.{ .stream = Bridge(true).stream, .readVec = Bridge(true).readVec } };
            base.writer = .{ .buffer = outgoing, .vtable = &.{ .drain = Bridge(true).drain, .flush = Bridge(true).flush } };
        } else {
            connection.stream_reader.interface = .{ .buffer = incoming, .seek = 0, .end = 0, .vtable = &.{ .stream = Bridge(false).stream, .readVec = Bridge(false).readVec } };
            connection.stream_writer.interface = .{ .buffer = outgoing, .vtable = &.{ .drain = Bridge(false).drain, .flush = Bridge(false).flush } };
        }
        self.bridged = true;
        return self;
    }
    fn innerReader(self: *@This()) *Reader {
        return if (self.has_identity) &self.inner.reader else &self.standard_inner.reader;
    }
    fn innerWriter(self: *@This()) *Writer {
        return if (self.has_identity) &self.inner.writer else &self.standard_inner.writer;
    }
    fn innerReadError(self: *@This()) ?anyerror {
        return if (self.has_identity) self.inner.read_err else self.standard_inner.read_err;
    }
    fn innerWriteError(self: *@This()) ?anyerror {
        return if (self.has_identity) self.inner.write_err else null;
    }
    /// Called only after a standard Request has accepted the acquired socket.
    /// That Request releases the pool entry before this carrier is destroyed.
    pub fn handoff(self: *@This()) void {
        self.owns_connection = false;
    }
    pub const Rejection = struct { request: std.http.Client.Request, response: std.http.Client.Response };
    pub fn takeRejection(self: *@This()) ?Rejection {
        const request = self.rejected_request orelse return null;
        const response = self.rejected_response.?;
        self.rejected_request = null;
        self.rejected_response = null;
        self.owns_connection = false;
        return .{ .request = request, .response = response };
    }
    fn tunnelOutput(self: *@This()) *Writer {
        return if (self.outer_tls) |*outer| &outer.writer else &self.plain_writer.?.interface;
    }
    fn tunnelDrain(writer: *Writer, data: []const []const u8, splat: usize) Writer.Error!usize {
        const self: *@This() = @alignCast(@fieldParentPtr("tunnel_writer", writer));
        const output = self.tunnelOutput();
        try output.writeAll(writer.buffered());
        writer.end = 0;
        var count: usize = 0;
        for (data[0 .. data.len - 1]) |bytes| {
            try output.writeAll(bytes);
            count += bytes.len;
        }
        const pattern = data[data.len - 1];
        for (0..splat) |_| try output.writeAll(pattern);
        return count + pattern.len * splat;
    }
    fn tunnelFlush(writer: *Writer) Writer.Error!void {
        const self: *@This() = @alignCast(@fieldParentPtr("tunnel_writer", writer));
        const output = self.tunnelOutput();
        try output.writeAll(writer.buffered());
        writer.end = 0;
        try output.flush();
        if (self.outer_tls != null) try self.connection.?.stream_writer.interface.flush();
    }
    fn Bridge(comptime encrypted_proxy: bool) type {
        return struct {
            fn readerOwner(reader: *Reader) *Connection {
                const physical: *std.Io.net.Stream.Reader = if (encrypted_proxy) physical: {
                    const client: *std.crypto.tls.Client = @fieldParentPtr("reader", reader);
                    break :physical @fieldParentPtr("interface", client.input);
                } else @fieldParentPtr("interface", reader);
                const connection: *std.http.Client.Connection = @fieldParentPtr("stream_reader", physical);
                return @fieldParentPtr("client", connection.client);
            }
            fn writerOwner(writer: *Writer) *Connection {
                const connection: *std.http.Client.Connection = if (encrypted_proxy) physical: {
                    const client: *std.crypto.tls.Client = @fieldParentPtr("writer", writer);
                    const reader: *std.Io.net.Stream.Reader = @fieldParentPtr("interface", client.input);
                    break :physical @fieldParentPtr("stream_reader", reader);
                } else physical: {
                    const physical_writer: *std.Io.net.Stream.Writer = @fieldParentPtr("interface", writer);
                    break :physical @fieldParentPtr("stream_writer", physical_writer);
                };
                return @fieldParentPtr("client", connection.client);
            }
            fn stream(reader: *Reader, writer: *Writer, limit: std.Io.Limit) Reader.StreamError!usize {
                const self = readerOwner(reader);
                return self.innerReader().stream(writer, limit) catch |err| {
                    if (err == error.ReadFailed) self.recordReadFailure();
                    return err;
                };
            }
            fn readVec(reader: *Reader, data: [][]u8) Reader.Error!usize {
                const self = readerOwner(reader);
                return self.innerReader().readVec(data) catch |err| {
                    if (err == error.ReadFailed) self.recordReadFailure();
                    return err;
                };
            }
            fn drain(writer: *Writer, data: []const []const u8, splat: usize) Writer.Error!usize {
                const self = writerOwner(writer);
                self.innerWriter().writeAll(writer.buffered()) catch |err| {
                    self.recordWriteFailure();
                    return err;
                };
                writer.end = 0;
                var count: usize = 0;
                for (data[0 .. data.len - 1]) |bytes| {
                    self.innerWriter().writeAll(bytes) catch |err| {
                        self.recordWriteFailure();
                        return err;
                    };
                    count += bytes.len;
                }
                const pattern = data[data.len - 1];
                for (0..splat) |_| self.innerWriter().writeAll(pattern) catch |err| {
                    self.recordWriteFailure();
                    return err;
                };
                return count + pattern.len * splat;
            }
            fn flush(writer: *Writer) Writer.Error!void {
                const self = writerOwner(writer);
                self.innerWriter().writeAll(writer.buffered()) catch |err| {
                    self.recordWriteFailure();
                    return err;
                };
                writer.end = 0;
                self.innerWriter().flush() catch |err| {
                    self.recordWriteFailure();
                    return err;
                };
                // std TLS flush emits records into its output writer; it
                // does not flush that enclosing transport. Drain the tunnel
                // writer and its outer TLS/plain and physical socket layers.
                self.tunnel_writer.flush() catch |err| {
                    self.recordWriteFailure();
                    return err;
                };
            }
        };
    }
    fn physicalError(self: *@This(), err: anyerror) anyerror {
        if (err == error.ReadFailed) {
            if (self.outer_tls) |outer| if (outer.read_err) |failure| return failure;
            if (self.plain_reader) |reader| if (reader.err) |failure| return failure;
            if (self.connection) |connection| {
                if (!self.bridged and connection.protocol == .tls) {
                    const client: *std.crypto.tls.Client = @fieldParentPtr("reader", connection.reader());
                    if (client.read_err) |failure| return failure;
                }
                if (connection.stream_reader.err) |failure| return failure;
            }
        }
        if (err == error.WriteFailed) {
            if (self.ready) if (self.innerWriteError()) |failure| return failure;
            if (self.plain_writer) |writer| if (writer.err) |failure| return failure;
            if (self.connection) |connection| if (connection.stream_writer.err) |failure| return failure;
        }
        return err;
    }
    fn recordReadFailure(self: *@This()) void {
        self.read_failure = if (self.innerReadError()) |failure| switch (failure) {
            error.TlsWriteFailed => self.physicalError(error.WriteFailed),
            else => failure,
        } else self.physicalError(error.ReadFailed);
        // std.http.Connection's plain/proxy error carrier is an OS error set.
        // Preserve the real TLS error above for the transport error callback.
        if (self.connection) |connection| connection.stream_reader.err = error.Unexpected;
    }
    fn recordWriteFailure(self: *@This()) void {
        self.write_failure = self.physicalError(error.WriteFailed);
        if (self.connection) |connection| connection.stream_writer.err = error.Unexpected;
    }
    pub fn ioError(self: *@This(), err: anyerror) anyerror {
        if (err == error.ReadFailed) return self.read_failure orelse self.physicalError(err);
        if (err == error.WriteFailed) return self.write_failure orelse self.physicalError(err);
        return err;
    }
    /// Restore the acquired connection before Request.deinit returns it to its
    /// standard pool. After that the stored connection pointer is not used.
    pub fn prepareRelease(self: *@This()) void {
        if (!self.bridged) return;
        if (self.base_tls) |base| base.* = self.outer_tls.? else if (self.connection) |connection| {
            connection.stream_reader = self.plain_reader.?;
            connection.stream_writer = self.plain_writer.?;
        }
        self.bridged = false;
    }
    pub fn destroy(self: *@This()) void {
        const a = self.allocator;
        const io = self.io;
        const protection = io.swapCancelProtection(.blocked);
        defer _ = io.swapCancelProtection(protection);
        self.prepareRelease();
        if (self.rejected_request) |*request| {
            if (request.connection) |connection| connection.closing = true;
            request.reader.state = .ready;
            request.deinit();
            self.rejected_request = null;
            self.owns_connection = false;
            self.connection = null;
        }
        if (self.owns_connection) if (self.connection) |connection| {
            connection.closing = true;
            self.client.connection_pool.release(connection, io);
            self.connection = null;
            self.owns_connection = false;
        };
        self.client.deinit();
        if (self.ready) {
            if (self.has_identity) std.crypto.secureZero(u8, std.mem.asBytes(&self.inner)) else std.crypto.secureZero(u8, std.mem.asBytes(&self.standard_inner));
        }
        if (self.outer_tls) |*outer| std.crypto.secureZero(u8, std.mem.asBytes(outer));
        self.arena.deinit();
        a.destroy(self);
    }
};
