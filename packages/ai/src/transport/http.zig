const std = @import("std");
const t = @import("../types.zig");
/// Certificate and PKCS8 key buffers borrow their owner through response release
/// (including send cancellation/join). Transport implementations must explicitly
/// support this identity; never silently send the request without it.
pub const ClientCertificate = struct { certificate: []const u8, privateKey: []const u8 };
/// HTTP header and Node request-target fields carry checked eight-bit code
/// units. Field-specific syntax validation is separate. Unicode decoding stays
/// in std.unicode; std.Io.Writer emits the protocol units.
pub fn httpFieldBytes(a: std.mem.Allocator, text: []const u8) ![]u8 {
    const units = try std.unicode.wtf8ToWtf16LeAlloc(a, text);
    defer a.free(units);
    const output = try a.alloc(u8, units.len);
    errdefer a.free(output);
    var writer = std.Io.Writer.fixed(output);
    for (units) |encoded| {
        const unit = std.mem.littleToNative(u16, encoded);
        if (unit > 255) return error.InvalidHeaderByteString;
        writer.writeInt(u8, @intCast(unit), .little) catch unreachable;
    }
    return output;
}
fn headerString(a: std.mem.Allocator, bytes: []const u8) ![]u8 {
    const units = try a.alloc(u16, bytes.len);
    defer a.free(units);
    for (bytes, units) |byte, *unit| unit.* = std.mem.nativeToLittle(u16, byte);
    return std.unicode.utf16LeToUtf8Alloc(a, units);
}
/// A single-use request reader, borrowed through send and response release.
/// No operation clones or eagerly consumes it at an injected host boundary.
/// length selects content-length framing; otherwise the default HTTP transport
/// streams chunked data. The owner joins sends before releasing this reader.
pub const RequestBodyStream = struct {
    reader: *std.Io.Reader,
    length: ?u64 = null,
    context: ?*anyopaque = null,
    readErrorFn: ?*const fn (?*anyopaque) ?anyerror = null,
    fn readError(self: @This(), err: anyerror) anyerror {
        if (err == error.ReadFailed) if (self.readErrorFn) |call| if (call(self.context)) |failure| return failure;
        return err;
    }
};
pub const Request = struct {
    method: std.http.Method = .GET,
    uri: std.Uri,
    headers: t.ProviderHeaders = &.{},
    body: ?[]const u8 = null,
    bodyStream: ?RequestBodyStream = null,
    timeoutMs: ?f64 = null,
    proxy: ?std.Uri = null,
    proxyResolved: bool = false,
    clientCertificate: ?ClientCertificate = null,
    /// Optional deadline for native std.http request/connection acquisition,
    /// separate from response headers and body. Injected transports implement
    /// this phase themselves. std TLS acquisition includes its handshake.
    connectionTimeoutMs: ?f64 = null,
    /// Preserve typed components in the default transport's owned URI copy.
    /// Native adapters use this for standard-writer-composed request targets
    /// whose literal delimiters belong to the encoded path component.
    preserveUriComponents: bool = false,
    /// Preserve extension methods at injected transport boundaries. The default
    /// std.http backend explicitly rejects names absent from its Method enum.
    customMethod: ?[]const u8 = null,
    followRedirects: bool = true,
    decompressResponse: bool = true,
    pub fn methodText(self: @This()) []const u8 {
        const name = self.customMethod orelse return @tagName(self.method);
        for ([_]std.http.Method{ .DELETE, .GET, .HEAD, .OPTIONS, .POST, .PUT }) |known| if (std.ascii.eqlIgnoreCase(name, @tagName(known))) return @tagName(known);
        return name;
    }
};
/// A host transport may pass arbitrary methods, byte bodies and readers through.
/// Its response owner must remain alive until release is called exactly once.
pub const Response = struct {
    status: u16,
    statusText: []const u8 = "",
    headers: t.ProviderHeaders,
    body: *std.Io.Reader,
    owner: *anyopaque,
    /// False represents Fetch's absent body; an empty reader alone still
    /// represents a present body that reaches EOF without any bytes.
    bodyPresent: bool = true,
    /// Optional logical body-read chunks, borrowed until the next read or
    /// response release. A host can preserve its original stream boundaries.
    /// This and body are alternate views with one consumer, not two streams.
    nextChunkFn: ?*const fn (*anyopaque, []u8) anyerror!?[]const u8 = null,
    releaseFn: *const fn (*anyopaque) void,
    /// Abort releases the owner without draining an unfinished response.
    /// Hosts whose release already closes the stream can omit this callback.
    abortFn: ?*const fn (*anyopaque) void = null,
    readErrorFn: ?*const fn (*anyopaque) ?anyerror = null,
    pub fn deinit(self: *@This()) void {
        self.releaseFn(self.owner);
        self.* = undefined;
    }
    pub fn abort(self: *@This()) void {
        (self.abortFn orelse self.releaseFn)(self.owner);
        self.* = undefined;
    }
    pub fn readError(self: @This(), err: anyerror) anyerror {
        if (err == error.ReadFailed) if (self.readErrorFn) |call| if (call(self.owner)) |underlying| return underlying;
        return err;
    }
    /// Byte-only transports use native read operations as their logical chunk
    /// policy. No Reader refill is advertised as an original Fetch chunk.
    pub fn nextBodyChunk(self: @This(), buffer: []u8) !?[]const u8 {
        if (self.nextChunkFn) |call| return call(self.owner, buffer) catch |err| return self.readError(err);
        if (buffer.len == 0) return error.EmptyBodyChunkBuffer;
        const buffered = self.body.buffered();
        if (buffered.len > 0) {
            const count = @min(buffered.len, buffer.len);
            @memcpy(buffer[0..count], buffered[0..count]);
            self.body.toss(count);
            return buffer[0..count];
        }
        var vectors = [_][]u8{buffer};
        const count = self.body.readVec(&vectors) catch |err| switch (err) {
            error.EndOfStream => return null,
            else => return self.readError(err),
        };
        return buffer[0..count];
    }
    pub fn readAll(self: @This(), a: std.mem.Allocator, limit: std.Io.Limit) ![]u8 {
        return self.body.allocRemaining(a, limit) catch |err| return self.readError(err);
    }
    /// Keeps the response owner borrowed until the body task has joined.
    pub fn readAllWithTimeout(self: @This(), a: std.mem.Allocator, io: std.Io, limit: std.Io.Limit, timeout_ms: ?f64) ![]u8 {
        const timeout = timeout_ms orelse return self.readAll(a, limit);
        const Outcome = union(enum) { body: anyerror![]u8, timeout: std.Io.Cancelable!void };
        var buffer: [2]Outcome = undefined;
        var select = std.Io.Select(Outcome).init(io, &buffer);
        defer while (select.cancel()) |outcome| switch (outcome) {
            .body => |result| {
                if (result) |bytes| a.free(bytes) else |_| {}
            },
            .timeout => {},
        };
        try select.concurrent(.body, readAll, .{ self, a, limit });
        try select.concurrent(.timeout, Transport.waitTimeout, .{ io, timeout });
        return switch (try select.await()) {
            .body => |result| result,
            .timeout => |result| {
                try result;
                return error.RequestTimeout;
            },
        };
    }
    /// Restart the inactivity deadline after each reader operation that makes
    /// progress. The response remains borrowed until each selected read joins.
    pub fn readAllWithIdleTimeout(self: @This(), a: std.mem.Allocator, io: std.Io, timeout_ms: f64) ![]u8 {
        var output = std.Io.Writer.Allocating.init(a);
        defer output.deinit();
        var chunk: [8192]u8 = undefined;
        while (true) {
            const Outcome = union(enum) { read: anyerror!usize, timeout: std.Io.Cancelable!void };
            var buffer: [2]Outcome = undefined;
            var select = std.Io.Select(Outcome).init(io, &buffer);
            defer while (select.cancel()) |_| {};
            try select.concurrent(.read, readSome, .{ self, chunk[0..] });
            try select.concurrent(.timeout, Transport.waitTimeout, .{ io, timeout_ms });
            const count = switch (try select.await()) {
                .read => |result| result catch |err| {
                    if (err == error.EndOfStream) break;
                    return self.readError(err);
                },
                .timeout => |result| {
                    try result;
                    return error.RequestTimeout;
                },
            };
            try output.writer.writeAll(chunk[0..count]);
        }
        return output.toOwnedSlice();
    }
    fn readSome(self: @This(), buffer: []u8) !usize {
        while (true) {
            var vectors = [_][]u8{buffer};
            const count = try self.body.readVec(&vectors);
            if (count > 0) return count;
        }
    }
};
/// The response owner and the caller-allocated body have independent lifetimes.
/// Release `response` after copying its headers; the caller owns `bytes`.
pub const BufferedResponse = struct {
    response: Response,
    bytes: []u8,
    bodyReadError: ?anyerror = null,
    pub fn releaseResponse(self: *@This()) void {
        if (self.bodyReadError != null) self.response.abort() else self.response.deinit();
    }
    pub fn deinit(self: *@This(), a: std.mem.Allocator) void {
        self.releaseResponse();
        a.free(self.bytes);
        self.* = undefined;
    }
};
pub const Transport = struct {
    context: *anyopaque,
    sendFn: *const fn (*anyopaque, std.mem.Allocator, std.Io, Request) anyerror!Response,
    supportsClientCertificates: bool = false,
    openWebSocketFn: ?*const fn (*anyopaque, std.mem.Allocator, std.Io, std.Uri, t.ProviderHeaders, ?std.Uri) anyerror!@import("websocket.zig").Connection = null,
    sessions: ?*@import("websocket_sessions.zig").Cache = null,
    labelTokenCache: ?*@import("../utils/label_token_cache.zig").Cache = null,
    anthropicFederationCache: ?*@import("../auth/anthropic_federation.zig").Cache = null,
    pub fn openWebSocket(self: @This(), a: std.mem.Allocator, io: std.Io, uri: std.Uri, headers: t.ProviderHeaders, timeout: ?f64, proxy: ?std.Uri) !@import("websocket.zig").Connection {
        const call = self.openWebSocketFn orelse return error.WebSocketUnavailable;
        if (timeout == null or !(timeout.? > 0)) return call(self.context, a, io, uri, headers, proxy);
        const Outcome = union(enum) { connection: anyerror!@import("websocket.zig").Connection, timeout: std.Io.Cancelable!void };
        var buffer: [2]Outcome = undefined;
        var select = std.Io.Select(Outcome).init(io, &buffer);
        defer while (select.cancel()) |outcome| switch (outcome) {
            .connection => |result| {
                if (result) |connection| {
                    var owned = connection;
                    owned.deinit();
                } else |_| {}
            },
            .timeout => {},
        };
        try select.concurrent(.connection, openWebSocketUntimed, .{ self, a, io, uri, headers, proxy });
        try select.concurrent(.timeout, waitTimeout, .{ io, timeout.? });
        return switch (try select.await()) {
            .connection => |result| result,
            .timeout => |result| {
                try result;
                return error.WebSocketConnectTimeout;
            },
        };
    }
    fn sendUntimed(self: @This(), a: std.mem.Allocator, io: std.Io, request: Request) anyerror!Response {
        return self.sendFn(self.context, a, io, request);
    }
    fn openWebSocketUntimed(self: @This(), a: std.mem.Allocator, io: std.Io, uri: std.Uri, headers: t.ProviderHeaders, proxy: ?std.Uri) anyerror!@import("websocket.zig").Connection {
        return self.openWebSocketFn.?(self.context, a, io, uri, headers, proxy);
    }
    pub fn send(self: @This(), a: std.mem.Allocator, io: std.Io, request: Request) !Response {
        try io.checkCancel();
        if (request.body != null and request.bodyStream != null) return error.AmbiguousRequestBody;
        if (request.clientCertificate != null and !self.supportsClientCertificates) return error.MtlsTransportUnavailable;
        const timeout = request.timeoutMs orelse return self.sendFn(self.context, a, io, request);
        const Outcome = union(enum) { response: anyerror!Response, timeout: std.Io.Cancelable!void };
        var buffer: [2]Outcome = undefined;
        var select = std.Io.Select(Outcome).init(io, &buffer);
        defer while (select.cancel()) |outcome| switch (outcome) {
            .response => |result| {
                if (result) |response| {
                    var value = response;
                    value.deinit();
                } else |_| {}
            },
            .timeout => {},
        };
        try select.concurrent(.response, sendUntimed, .{ self, a, io, request });
        try select.concurrent(.timeout, waitTimeout, .{ io, timeout });
        return switch (try select.await()) {
            .response => |result| result,
            .timeout => |result| {
                try result;
                return error.RequestTimeout;
            },
        };
    }
    /// Applies one deadline to sending the request and consuming its body.
    pub fn sendReadAll(self: @This(), a: std.mem.Allocator, io: std.Io, request: Request, limit: std.Io.Limit) !BufferedResponse {
        return self.sendReadAllWithOptions(a, io, request, limit, .{});
    }
    pub const ReadAllOptions = struct { ignoreHttpErrorBodyReadFailure: bool = false };
    /// Selected Fetch consumers deliberately ignore an unreadable HTTP error
    /// body. Keep its headers alive and abort that response when released.
    pub fn sendReadAllWithOptions(self: @This(), a: std.mem.Allocator, io: std.Io, request: Request, limit: std.Io.Limit, options: ReadAllOptions) !BufferedResponse {
        try io.checkCancel();
        if (request.body != null and request.bodyStream != null) return error.AmbiguousRequestBody;
        if (request.clientCertificate != null and !self.supportsClientCertificates) return error.MtlsTransportUnavailable;
        const timeout = request.timeoutMs orelse return self.readAllAttempt(a, io, request, limit, options);
        const Outcome = union(enum) { response: anyerror!BufferedResponse, timeout: std.Io.Cancelable!void };
        var buffer: [2]Outcome = undefined;
        var select = std.Io.Select(Outcome).init(io, &buffer);
        defer while (select.cancel()) |outcome| switch (outcome) {
            .response => |result| {
                if (result) |response| {
                    var owned = response;
                    owned.deinit(a);
                } else |_| {}
            },
            .timeout => {},
        };
        var untimed = request;
        untimed.timeoutMs = null;
        try select.concurrent(.response, readAllAttempt, .{ self, a, io, untimed, limit, options });
        try select.concurrent(.timeout, waitTimeout, .{ io, timeout });
        return switch (try select.await()) {
            .response => |result| result,
            .timeout => |result| {
                try result;
                return error.RequestTimeout;
            },
        };
    }
    fn readAllAttempt(self: @This(), a: std.mem.Allocator, io: std.Io, request: Request, limit: std.Io.Limit, options: ReadAllOptions) !BufferedResponse {
        var response = try self.sendFn(self.context, a, io, request);
        errdefer response.abort();
        const bytes = response.readAll(a, limit) catch |err| {
            if (!options.ignoreHttpErrorBodyReadFailure or (response.status >= 200 and response.status < 300) or err == error.Canceled or err == error.OutOfMemory) return err;
            return .{ .response = response, .bytes = try a.alloc(u8, 0), .bodyReadError = err };
        };
        return .{ .response = response, .bytes = bytes };
    }
    fn waitTimeout(io: std.Io, milliseconds: f64) std.Io.Cancelable!void {
        try io.sleep(.fromMilliseconds(@intCast(@import("../utils/provider_retry.zig").timerMilliseconds(milliseconds))), .awake);
    }
};
pub const Http = struct {
    client: std.http.Client,
    sessions: @import("websocket_sessions.zig").Cache,
    labelTokenCache: @import("../utils/label_token_cache.zig").Cache,
    anthropicFederationCache: @import("../auth/anthropic_federation.zig").Cache,
    authContext: ?@import("../auth/types.zig").AuthContext = null,
    proxyMutex: std.Io.Mutex = .init,
    proxyPools: std.StringHashMapUnmanaged(*ProxyPool) = .{},
    const ProxyPool = struct { client: std.http.Client, arena: std.heap.ArenaAllocator };
    pub fn init(a: std.mem.Allocator, io: std.Io) @This() {
        return .{ .client = .{ .allocator = a, .io = io }, .sessions = .init(a, io), .labelTokenCache = .init(a), .anthropicFederationCache = .init(a, io) };
    }
    pub fn deinit(self: *@This()) void {
        const a = self.client.allocator;
        self.sessions.deinit();
        self.labelTokenCache.deinit();
        self.anthropicFederationCache.deinit();
        var iterator = self.proxyPools.iterator();
        while (iterator.next()) |entry| {
            const pool = entry.value_ptr.*;
            pool.client.deinit();
            pool.arena.deinit();
            a.destroy(pool);
        }
        self.proxyPools.deinit(a);
        self.client.deinit();
    }
    fn clientFor(self: *@This(), proxy: ?std.Uri) !*std.http.Client {
        const url = proxy orelse return &self.client;
        const a = self.client.allocator;
        const io = self.client.io;
        var temporary = std.heap.ArenaAllocator.init(a);
        defer temporary.deinit();
        const key = try @import("uri.zig").render(temporary.allocator(), url);
        try self.proxyMutex.lock(io);
        defer self.proxyMutex.unlock(io);
        if (self.proxyPools.get(key)) |pool| return &pool.client;
        const pool = try a.create(ProxyPool);
        errdefer a.destroy(pool);
        pool.* = .{ .client = .{ .allocator = a, .io = io }, .arena = .init(a) };
        errdefer pool.arena.deinit();
        errdefer pool.client.deinit();
        const alloc = pool.arena.allocator();
        try @import("proxy.zig").configureHttpProxy(&pool.client, alloc, url);
        if (std.ascii.eqlIgnoreCase(url.scheme, "https")) {
            const now = std.Io.Clock.real.now(io);
            try pool.client.ca_bundle.rescan(a, io, now);
            pool.client.now = now;
        }
        // Plain HTTP uses the agent's absolute request-target form. HTTPS
        // target tunnels are owned separately and never use std fallback.
        if (pool.client.http_proxy) |configured| configured.supports_connect = false;
        if (pool.client.https_proxy) |configured| configured.supports_connect = false;
        const retained = try alloc.dupe(u8, key);
        try self.proxyPools.put(a, retained, pool);
        return &pool.client;
    }
    pub fn transport(self: *@This()) Transport {
        return .{ .context = self, .sendFn = send, .supportsClientCertificates = true, .openWebSocketFn = openWebSocket, .sessions = &self.sessions, .labelTokenCache = &self.labelTokenCache, .anthropicFederationCache = &self.anthropicFederationCache };
    }
    const Upgraded = struct {
        allocator: std.mem.Allocator,
        arena: std.heap.ArenaAllocator,
        request: std.http.Client.Request,
        identity: ?*@import("identity_connection.zig").Connection = null,
        fn release(ctx: *anyopaque) void {
            const self: *@This() = @ptrCast(@alignCast(ctx));
            const a = self.allocator;
            self.request.connection.?.closing = true;
            self.request.reader.state = .ready;
            if (self.identity) |identity| identity.prepareRelease();
            self.request.deinit();
            if (self.identity) |identity| identity.destroy();
            self.arena.deinit();
            a.destroy(self);
        }
        fn shutdown(ctx: *anyopaque) void {
            const self: *@This() = @ptrCast(@alignCast(ctx));
            self.request.connection.?.stream_reader.stream.shutdown(self.request.client.io, .both) catch {};
        }
        fn ioError(ctx: *anyopaque, err: anyerror) anyerror {
            const self: *@This() = @ptrCast(@alignCast(ctx));
            const connection = self.request.connection orelse return err;
            if (self.identity) |identity| return identity.ioError(err);
            if (err == error.ReadFailed) return connection.getReadError() orelse err;
            if (err == error.WriteFailed) return connection.stream_writer.err orelse err;
            return err;
        }
    };
    const PreparedHeaders = struct { overrides: std.http.Client.Request.Headers, extra: []const std.http.Header };
    fn checkUrlCredentials(url: std.Uri) !void {
        for ([_]?std.Uri.Component{ url.user, url.password }) |component| if (component) |value| {
            const text = switch (value) {
                .raw, .percent_encoded => |bytes| bytes,
            };
            if (text.len > 0) return error.RequestUrlIncludesCredentials;
        };
    }
    fn prepareHeaders(a: std.mem.Allocator, input: t.ProviderHeaders, defaults: std.http.Client.Request.Headers, body_length: ?u64) !PreparedHeaders {
        var overrides = defaults;
        var extra: std.ArrayList(std.http.Header) = .empty;
        errdefer extra.deinit(a);
        for (input) |header| {
            if (std.ascii.eqlIgnoreCase(header.name, "content-length")) if (body_length) |expected| {
                if (header.value) |text| {
                    const declared = std.fmt.parseInt(u64, text, 10) catch return error.InvalidContentLength;
                    if (declared != expected) return error.ContentLengthMismatch;
                }
                continue;
            };
            if (std.ascii.eqlIgnoreCase(header.name, "transfer-encoding") and header.value != null) return error.UnsupportedTransferEncodingHeader;
            const value: std.http.Client.Request.Headers.Value = if (header.value) |text| .{ .override = try httpFieldBytes(a, text) } else .omit;
            if (std.ascii.eqlIgnoreCase(header.name, "host")) overrides.host = value else if (std.ascii.eqlIgnoreCase(header.name, "authorization")) overrides.authorization = value else if (std.ascii.eqlIgnoreCase(header.name, "user-agent")) overrides.user_agent = value else if (std.ascii.eqlIgnoreCase(header.name, "connection")) overrides.connection = value else if (std.ascii.eqlIgnoreCase(header.name, "accept-encoding")) overrides.accept_encoding = value else if (std.ascii.eqlIgnoreCase(header.name, "content-type")) overrides.content_type = value else if (value == .override) try extra.append(a, .{ .name = try httpFieldBytes(a, header.name), .value = value.override });
        }
        return .{ .overrides = overrides, .extra = try extra.toOwnedSlice(a) };
    }
    fn openWebSocket(ctx: *anyopaque, a: std.mem.Allocator, io: std.Io, original: std.Uri, input_headers: t.ProviderHeaders, proxy: ?std.Uri) !@import("websocket.zig").Connection {
        const self: *@This() = @ptrCast(@alignCast(ctx));
        try io.checkCancel();
        const active = try a.create(Upgraded);
        errdefer a.destroy(active);
        active.allocator = a;
        active.arena = .init(a);
        errdefer active.arena.deinit();
        const alloc = active.arena.allocator();
        active.identity = null;
        errdefer if (active.identity) |identity| identity.destroy();
        var url = try std.Uri.parse(try @import("uri.zig").render(alloc, original));
        try checkUrlCredentials(url);
        if (std.mem.eql(u8, url.scheme, "wss")) url.scheme = "https" else if (std.mem.eql(u8, url.scheme, "ws")) url.scheme = "http";
        if (url.fragment != null or (!std.mem.eql(u8, url.scheme, "http") and !std.mem.eql(u8, url.scheme, "https"))) return error.InvalidWebSocketUrl;
        var nonce: [16]u8 = undefined;
        try io.randomSecure(&nonce);
        var key: [24]u8 = undefined;
        _ = std.base64.standard.Encoder.encode(&key, &nonce);
        var caller_headers: std.ArrayList(t.ProviderHeader) = .empty;
        for (input_headers) |header| {
            var reserved = false;
            for ([_][]const u8{ "upgrade", "connection", "sec-websocket-key", "sec-websocket-version", "sec-websocket-extensions", "sec-websocket-protocol" }) |name| if (std.ascii.eqlIgnoreCase(header.name, name)) {
                reserved = true;
                break;
            };
            if (!reserved) try caller_headers.append(alloc, header);
        }
        var prepared = try prepareHeaders(alloc, caller_headers.items, .{ .authorization = .omit, .accept_encoding = .omit }, null);
        prepared.overrides.connection = .{ .override = "Upgrade" };
        var headers: std.ArrayList(std.http.Header) = .empty;
        try headers.appendSlice(alloc, prepared.extra);
        try headers.appendSlice(alloc, &.{ .{ .name = "upgrade", .value = "websocket" }, .{ .name = "sec-websocket-version", .value = "13" }, .{ .name = "sec-websocket-key", .value = try alloc.dupe(u8, &key) } });
        if (proxy) |proxy_url| if (std.mem.eql(u8, url.scheme, "https")) {
            active.identity = try @import("identity_connection.zig").Connection.openTunnel(a, io, &self.client, url, proxy_url);
        };
        if (active.identity) |identity| if (identity.rejected_response != null) return error.WebSocketUpgradeRejected;
        const client = if (active.identity) |identity| &identity.client else try self.clientFor(proxy);
        active.request = try client.request(.GET, url, .{ .connection = if (active.identity) |identity| identity.connection else null, .extra_headers = headers.items, .headers = prepared.overrides, .redirect_behavior = .not_allowed });
        if (active.identity) |identity| identity.handoff();
        errdefer {
            if (active.identity) |identity| identity.prepareRelease();
            active.request.connection.?.closing = true;
            active.request.reader.state = .ready;
            active.request.deinit();
        }
        active.request.sendBodiless() catch |err| return Upgraded.ioError(active, err);
        var redirect_buffer: [8192]u8 = undefined;
        const response = active.request.receiveHead(&redirect_buffer) catch |err| return Upgraded.ioError(active, err);
        if (response.head.status != .switching_protocols) return error.WebSocketUpgradeRejected;
        var accepted = false;
        var accept_count: usize = 0;
        var negotiated = false;
        var upgraded = false;
        var connection_upgrade = false;
        var received_headers: std.ArrayList(t.ProviderHeader) = .empty;
        const expected_accept = try @import("websocket.zig").acceptKey(alloc, &key);
        var iterator = response.head.iterateHeaders();
        while (iterator.next()) |header| {
            try received_headers.append(alloc, .{ .name = try headerString(alloc, header.name), .value = try headerString(alloc, header.value) });
            if (std.ascii.eqlIgnoreCase(header.name, "sec-websocket-accept")) {
                accept_count += 1;
                accepted = std.mem.eql(u8, std.mem.trim(u8, header.value, " \t"), expected_accept);
            }
            if (std.ascii.eqlIgnoreCase(header.name, "sec-websocket-extensions") or std.ascii.eqlIgnoreCase(header.name, "sec-websocket-protocol")) negotiated = true;
            if (std.ascii.eqlIgnoreCase(header.name, "upgrade")) upgraded = std.ascii.eqlIgnoreCase(header.value, "websocket");
            if (std.ascii.eqlIgnoreCase(header.name, "connection")) {
                var tokens = std.mem.tokenizeAny(u8, header.value, ", \t");
                while (tokens.next()) |token| if (std.ascii.eqlIgnoreCase(token, "upgrade")) {
                    connection_upgrade = true;
                };
            }
        }
        if (!accepted or accept_count != 1 or negotiated or !upgraded or !connection_upgrade) return error.InvalidWebSocketUpgrade;
        const connection = active.request.connection.?;
        return .{ .reader = connection.reader(), .writer = connection.writer(), .owner = active, .headers = received_headers.items, .releaseFn = Upgraded.release, .shutdownFn = Upgraded.shutdown, .ioErrorFn = Upgraded.ioError };
    }
    const Active = struct {
        allocator: std.mem.Allocator,
        arena: std.heap.ArenaAllocator,
        request: std.http.Client.Request,
        identity: ?*@import("identity_connection.zig").Connection = null,
        response: std.http.Client.Response,
        transfer: [8192]u8 = undefined,
        decompress_buffer: [std.compress.flate.max_window_len]u8 = undefined,
        decompress: std.http.Decompress = undefined,
        fn release(ctx: *anyopaque) void {
            const self: *@This() = @ptrCast(@alignCast(ctx));
            const a = self.allocator;
            if (self.identity) |identity| {
                // This identity connection is request-owned. Closing it also
                // disposes an unread body, without draining after unwrapping.
                if (self.request.connection) |connection| connection.closing = true;
                self.request.reader.state = .ready;
                identity.prepareRelease();
            }
            self.request.deinit();
            if (self.identity) |identity| identity.destroy();
            self.arena.deinit();
            a.destroy(self);
        }
        fn abort(ctx: *anyopaque) void {
            const self: *@This() = @ptrCast(@alignCast(ctx));
            if (self.request.connection) |connection| connection.closing = true;
            self.request.reader.state = .ready;
            release(ctx);
        }
        fn readError(ctx: *anyopaque) ?anyerror {
            const self: *@This() = @ptrCast(@alignCast(ctx));
            if (self.identity) |identity| if (identity.read_failure) |err| return err;
            if (self.response.bodyErr()) |err| return err;
            if (self.request.connection) |connection| if (connection.protocol == .tls) {
                const tls: *std.crypto.tls.Client = @fieldParentPtr("reader", connection.reader());
                if (tls.read_err) |err| return err;
            };
            if (self.request.connection) |connection| if (connection.stream_reader.err) |err| return err;
            return switch (self.decompress) {
                .none => null,
                .flate => |decompress| if (decompress.err) |err| err else null,
                .zstd => |decompress| if (decompress.err) |err| err else null,
            };
        }
        fn ioError(self: *@This(), err: anyerror) anyerror {
            if (self.identity) |identity| return identity.ioError(err);
            const connection = self.request.connection orelse return err;
            if (err == error.ReadFailed) return connection.getReadError() orelse err;
            if (err == error.WriteFailed) return connection.stream_writer.err orelse err;
            return err;
        }
    };
    fn send(ctx: *anyopaque, a: std.mem.Allocator, io: std.Io, input: Request) !Response {
        const self: *@This() = @ptrCast(@alignCast(ctx));
        try io.checkCancel();
        var temporary = std.heap.ArenaAllocator.init(a);
        defer temporary.deinit();
        const alloc = temporary.allocator();
        var request = input;
        var redirects: u32 = 0;
        const method_name = input.methodText();
        if (std.ascii.eqlIgnoreCase(method_name, "CONNECT") or std.ascii.eqlIgnoreCase(method_name, "TRACE") or std.ascii.eqlIgnoreCase(method_name, "TRACK")) return error.ForbiddenFetchMethod;
        request.method = std.meta.stringToEnum(std.http.Method, method_name) orelse return error.CustomHttpMethodUnsupported;
        request.customMethod = null;
        while (true) {
            var response = try self.sendOne(a, io, request);
            var retained = false;
            defer if (!retained) response.deinit();
            if (!request.followRedirects) {
                retained = true;
                return response;
            }
            const redirect = switch (response.status) {
                301, 302, 303, 307, 308 => true,
                else => false,
            };
            const location: ?[]const u8 = blk: {
                if (!redirect) break :blk null;
                for (response.headers) |header| if (std.ascii.eqlIgnoreCase(header.name, "location")) break :blk header.value;
                break :blk null;
            };
            if (location == null) {
                retained = true;
                return response;
            }
            if (redirects == 20) return error.TooManyHttpRedirects;
            redirects += 1;
            try io.checkCancel();
            const base = try @import("uri.zig").render(alloc, request.uri);
            const capacity = try std.math.add(usize, location.?.len, try std.math.add(usize, base.len, 64));
            var buffer = try alloc.alloc(u8, capacity);
            @memcpy(buffer[0..location.?.len], location.?);
            const next = try request.uri.resolveInPlace(location.?.len, &buffer);
            if (!std.ascii.eqlIgnoreCase(next.scheme, "http") and !std.ascii.eqlIgnoreCase(next.scheme, "https")) return error.UnsupportedRedirectScheme;
            var old_host_buffer: [std.Io.net.HostName.max_len]u8 = undefined;
            var new_host_buffer: [std.Io.net.HostName.max_len]u8 = undefined;
            const old_host = try std.Io.net.HostName.fromUri(request.uri, &old_host_buffer);
            const new_host = try std.Io.net.HostName.fromUri(next, &new_host_buffer);
            const old_port = request.uri.port orelse @as(u16, if (std.ascii.eqlIgnoreCase(request.uri.scheme, "https")) 443 else 80);
            const new_port = next.port orelse @as(u16, if (std.ascii.eqlIgnoreCase(next.scheme, "https")) 443 else 80);
            const same_origin = std.ascii.eqlIgnoreCase(request.uri.scheme, next.scheme) and old_port == new_port and std.ascii.eqlIgnoreCase(old_host.bytes, new_host.bytes);
            if (request.bodyStream != null and response.status != 303) return error.StreamingBodyRedirectUnsupported;
            const use_get = ((response.status == 301 or response.status == 302) and request.method == .POST) or (response.status == 303 and request.method != .GET and request.method != .HEAD);
            var headers: std.ArrayList(t.ProviderHeader) = .empty;
            for (request.headers) |header| {
                if (use_get and (std.ascii.eqlIgnoreCase(header.name, "content-encoding") or std.ascii.eqlIgnoreCase(header.name, "content-language") or std.ascii.eqlIgnoreCase(header.name, "content-location") or std.ascii.eqlIgnoreCase(header.name, "content-type") or std.ascii.eqlIgnoreCase(header.name, "content-length"))) continue;
                if (!same_origin and std.ascii.eqlIgnoreCase(header.name, "authorization")) continue;
                try headers.append(alloc, header);
            }
            request.headers = headers.items;
            request.uri = next;
            if (use_get) {
                request.method = .GET;
                request.body = null;
                request.bodyStream = null;
            }
        }
    }
    fn requestWithConnectionTimeout(client: *std.http.Client, io: std.Io, method: std.http.Method, target: std.Uri, options: std.http.Client.RequestOptions, timeout: ?f64) !std.http.Client.Request {
        const milliseconds = timeout orelse return client.request(method, target, options);
        if (!(milliseconds > 0) or options.connection != null) return client.request(method, target, options);
        const Outcome = union(enum) { request: anyerror!std.http.Client.Request, timeout: std.Io.Cancelable!void };
        var buffer: [2]Outcome = undefined;
        var select = std.Io.Select(Outcome).init(io, &buffer);
        // A request acquired concurrently with cancellation owns its connection
        // until this join; release it before caller-owned URI/header storage.
        defer while (select.cancel()) |outcome| switch (outcome) {
            .request => |result| {
                if (result) |request| {
                    var owned = request;
                    if (owned.connection) |connection| connection.closing = true;
                    owned.deinit();
                } else |_| {}
            },
            .timeout => {},
        };
        try select.concurrent(.request, std.http.Client.request, .{ client, method, target, options });
        try select.concurrent(.timeout, Transport.waitTimeout, .{ io, milliseconds });
        return switch (try select.await()) {
            .request => |result| result,
            .timeout => |result| {
                try result;
                return error.ConnectionTimedOut;
            },
        };
    }
    fn sendOne(self: *@This(), a: std.mem.Allocator, io: std.Io, input: Request) !Response {
        try io.checkCancel();
        try checkUrlCredentials(input.uri);
        if (input.method == .CONNECT or input.method == .TRACE) return error.ForbiddenFetchMethod;
        if ((input.body != null or input.bodyStream != null) and (input.method == .GET or input.method == .HEAD)) return error.HttpMethodBodyUnsupported;
        const active = try a.create(Active);
        errdefer a.destroy(active);
        active.allocator = a;
        active.arena = .init(a);
        errdefer active.arena.deinit();
        active.identity = null;
        errdefer if (active.identity) |identity| identity.destroy();
        const alloc = active.arena.allocator();
        var body_length: ?u64 = if (input.body) |body| @intCast(body.len) else if (input.bodyStream) |source| source.length else if (input.method.requestHasBody()) 0 else null;
        if (input.bodyStream != null and body_length == null) for (input.headers) |header| {
            if (std.ascii.eqlIgnoreCase(header.name, "content-length")) if (header.value) |text| {
                body_length = try std.fmt.parseInt(u64, text, 10);
            };
        };
        var prepared = try prepareHeaders(alloc, input.headers, .{ .authorization = .omit, .accept_encoding = if (input.decompressResponse) .{ .override = "gzip, deflate" } else .omit }, body_length);
        var extra: std.ArrayList(std.http.Header) = .empty;
        try extra.appendSlice(alloc, prepared.extra);
        if ((input.body != null or input.bodyStream != null) and !input.method.requestHasBody()) {
            if (body_length) |length| try extra.append(alloc, .{ .name = "content-length", .value = try std.fmt.allocPrint(alloc, "{d}", .{length}) }) else try extra.append(alloc, .{ .name = "transfer-encoding", .value = "chunked" });
        }
        const target = if (input.preserveUriComponents) try t.cloneInto(std.Uri, alloc, input.uri) else try std.Uri.parse(try @import("uri.zig").render(alloc, input.uri));
        const proxy = if (input.proxyResolved) input.proxy else input.proxy orelse try @import("proxy.zig").resolveHttpProxyUrlForTarget(alloc, io, self.authContext, .{}, input.uri);
        const identity_request = input.clientCertificate != null and std.ascii.eqlIgnoreCase(target.scheme, "https");
        const tunnel_request = proxy != null and std.ascii.eqlIgnoreCase(target.scheme, "https");
        var client = if (identity_request or tunnel_request) &self.client else try self.clientFor(proxy);
        if (proxy != null and !tunnel_request) {
            const configured = client.http_proxy.?;
            var index: usize = 0;
            while (index < extra.items.len) {
                const name = extra.items[index].name;
                if (std.ascii.eqlIgnoreCase(name, "proxy-connection") or (configured.authorization != null and std.ascii.eqlIgnoreCase(name, "proxy-authorization"))) {
                    _ = extra.orderedRemove(index);
                } else index += 1;
            }
            try extra.append(alloc, .{ .name = "proxy-connection", .value = "close" });
        }
        var acquired_connection: ?*std.http.Client.Connection = null;
        if (identity_request) {
            const identity = input.clientCertificate.?;
            active.identity = try @import("identity_connection.zig").Connection.open(a, io, &self.client, target, identity.certificate, identity.privateKey, proxy);
            client = &active.identity.?.client;
            acquired_connection = active.identity.?.connection;
            if (prepared.overrides.host == .default) {
                var host = std.Io.Writer.Allocating.init(alloc);
                defer host.deinit();
                try target.writeToStream(&host.writer, .{ .scheme = false, .authentication = false, .path = false, .query = false, .fragment = false });
                prepared.overrides.host = .{ .override = try host.toOwnedSlice() };
            }
        } else if (tunnel_request) {
            active.identity = try @import("identity_connection.zig").Connection.openTunnel(a, io, &self.client, target, proxy.?);
            client = &active.identity.?.client;
            acquired_connection = active.identity.?.connection;
        }
        const rejected = if (active.identity) |identity| identity.takeRejection() else null;
        if (rejected) |response| {
            active.request = response.request;
            active.request.method = input.method;
            active.response = response.response;
            active.response.request = &active.request;
        } else {
            active.request = try requestWithConnectionTimeout(client, io, input.method, target, .{ .connection = acquired_connection, .extra_headers = extra.items, .headers = prepared.overrides, .redirect_behavior = .unhandled }, input.connectionTimeoutMs);
            if (active.identity) |identity| identity.handoff();
        }
        if (identity_request or tunnel_request) active.request.reader.max_head_len = self.client.read_buffer_size;
        errdefer {
            if (active.identity) |identity| identity.prepareRelease();
            if (active.request.connection) |connection| connection.closing = true;
            active.request.reader.state = .ready;
            active.request.deinit();
        }
        if (rejected == null) {
            if (input.bodyStream) |source| {
                var buffer: [8192]u8 = undefined;
                var writer: std.http.BodyWriter = if (input.method.requestHasBody()) blk: {
                    active.request.transfer_encoding = if (body_length) |length| .{ .content_length = length } else .chunked;
                    break :blk active.request.sendBody(&buffer) catch |err| return active.ioError(err);
                } else blk: {
                    active.request.sendBodilessUnflushed() catch |err| return active.ioError(err);
                    break :blk .{ .http_protocol_output = active.request.connection.?.writer(), .state = if (body_length) |length| .{ .content_length = length } else .init_chunked, .writer = .{ .buffer = &buffer, .vtable = if (body_length != null) &.{ .drain = std.http.BodyWriter.contentLengthDrain, .sendFile = std.http.BodyWriter.contentLengthSendFile } else &.{ .drain = std.http.BodyWriter.chunkedDrain, .sendFile = std.http.BodyWriter.chunkedSendFile } } };
                };
                if (body_length) |length| {
                    source.reader.streamExact64(&writer.writer, length) catch |err| return if (err == error.WriteFailed) active.ioError(err) else source.readError(err);
                } else _ = source.reader.streamRemaining(&writer.writer) catch |err| return if (err == error.WriteFailed) active.ioError(err) else source.readError(err);
                writer.end() catch |err| return active.ioError(err);
                active.request.connection.?.flush() catch |err| return active.ioError(err);
            } else if (input.method.requestHasBody()) {
                active.request.sendBodyComplete(try alloc.dupe(u8, input.body orelse &.{})) catch |err| return active.ioError(err);
            } else if (input.body) |body| {
                // std.http.Client's body helper excludes DELETE and OPTIONS, but
                // their Fetch requests can carry a byte body. Compose the standard
                // head and content-length body codecs without encoding HTTP text.
                active.request.sendBodilessUnflushed() catch |err| return active.ioError(err);
                var writer = std.http.BodyWriter{
                    .http_protocol_output = active.request.connection.?.writer(),
                    .state = .{ .content_length = body.len },
                    .writer = .{ .buffer = try alloc.dupe(u8, body), .end = body.len, .vtable = &.{ .drain = std.http.BodyWriter.contentLengthDrain, .sendFile = std.http.BodyWriter.contentLengthSendFile } },
                };
                writer.end() catch |err| return active.ioError(err);
                active.request.connection.?.flush() catch |err| return active.ioError(err);
            } else active.request.sendBodiless() catch |err| return active.ioError(err);
            var redirect: [8192]u8 = undefined;
            while (true) {
                active.response = active.request.receiveHead(&redirect) catch |err| return active.ioError(err);
                if (active.response.head.status == .switching_protocols) return error.UnexpectedHttpUpgrade;
                if (active.response.head.status.class() != .informational) break;
                // 102/103 are interim heads. Their fields do not describe an HTTP
                // body and must not escape as the Fetch-equivalent final response.
                active.request.reader.state = .ready;
            }
        }
        const status: u16 = @backingInt(active.response.head.status);
        var response_headers: std.ArrayList(t.ProviderHeader) = .empty;
        var it = active.response.head.iterateHeaders();
        while (it.next()) |header| {
            try response_headers.append(alloc, .{ .name = try headerString(alloc, header.name), .value = try headerString(alloc, header.value) });
        }
        const reason = try headerString(alloc, active.response.head.reason);
        active.decompress = .{ .none = active.request.connection.?.reader() };
        const body_present = input.method != .HEAD and status != 204 and status != 205 and status != 304;
        const body: *std.Io.Reader = if (!body_present) empty: {
            // HEAD/204/304 terminate at the head even with representation
            // length headers. Do not construct or drain a body/decompressor.
            active.request.reader.state = .ready;
            // 205 is a Fetch null-body status but can carry HTTP body framing.
            // An unread framed representation is closed rather than drained
            // during release, so it cannot stall or poison a pooled socket.
            if (status == 205 and (active.response.head.transfer_encoding == .chunked or active.response.head.content_length != @as(?u64, 0))) active.request.connection.?.closing = true;
            active.decompress = .{ .none = .ending };
            break :empty .ending;
        } else if (input.decompressResponse) active.response.readerDecompressing(&active.transfer, &active.decompress, &active.decompress_buffer) else active.response.reader(&active.transfer);
        return .{ .status = status, .statusText = reason, .headers = response_headers.items, .body = body, .bodyPresent = body_present, .owner = active, .releaseFn = Active.release, .abortFn = Active.abort, .readErrorFn = Active.readError };
    }
};
