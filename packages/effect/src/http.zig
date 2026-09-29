//! Managed HTTP requests over a caller-provided transport and I/O environment.
const diagnostics = @import("internal/diagnostics.zig");
const std = @import("std");
const outcomes = @import("outcome.zig");
const Cancellation = @import("internal/runtime.zig").Cancellation;
const native_operation = @import("internal/native_operation.zig");

/// Typed failure channel combining domain and provider errors.
pub const Failure = union(enum) { transport: outcomes.Defect, http: std.http.Status, decode: outcomes.Defect };
/// Typed request payload or request record for this operation.
pub const Request = struct {
    uri: []const u8,
    method: std.http.Method,
    headers: []const std.http.Header,
    body: ?[]const u8,
};
/// HTTP transport interface selected by the caller. The callback receives
/// borrowed provider state, a request, and an optional cancellation token;
/// it returns a typed `Exit(NativeResponse, Failure)`.
pub const Transport = struct {
    userdata: *anyopaque,
    /// Parameters: borrowed provider context, request, and cancellation token.
    /// Returns a native response on success or a typed transport/HTTP failure.
    send: *const fn (*anyopaque, Request, ?*const Cancellation) outcomes.Exit(NativeResponse, Failure),
};
/// Provider-owned response handle. The vtable determines how response bytes
/// are read, cancellation is forwarded, and the handle is disposed.
pub const NativeResponse = struct {
    userdata: *anyopaque,
    status: std.http.Status,
    vtable: *const VTable,
    /// Typed function-pointer interface for this capability. Implement every
    /// hook with the exact signature below; keep the table valid while the
    /// capability or pending work can invoke it.
    pub const VTable = struct {
        /// Read the selected value.
        /// Parameters (callback order):
        /// - `1: *anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
        /// - `2: ?*const Cancellation`: optional borrowed cancellation token.
        /// Returns (`outcomes.Exit(outcomes.OwnedSlice(u8), Failure)`): typed terminal Exit, including defect and interruption outcomes.
        read: *const fn (*anyopaque, ?*const Cancellation) outcomes.Exit(outcomes.OwnedSlice(u8), Failure),
        // Cancellation only forwards a request; it never joins or disposes an
        // operation owned by another caller. Disposal follows all active reads.
        /// Request cancellation.
        /// Parameters (callback order):
        /// - `1: *anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
        /// Returns (`?outcomes.Defect`): `null` when cancellation was forwarded;
        /// a defect explains why the provider could not forward it.
        cancel: *const fn (*anyopaque) ?outcomes.Defect,
        /// Release a native resource.
        /// Parameters (callback order):
        /// - `1: *anyopaque`: opaque provider or handle context; keep it live until the hook and any retained work complete.
        /// Returns (`outcomes.Exit(void, Failure)`): typed terminal Exit, including defect and interruption outcomes.
        dispose: *const fn (*anyopaque) outcomes.Exit(void, Failure),
    };
};

/// Typed response payload or response record for this operation.
pub const Response = struct {
    const Self = @This();
    allocator: std.mem.Allocator,
    io: std.Io,
    native: NativeResponse = undefined,
    mutex: std.Io.Mutex = .init,
    reading: std.Io.Mutex = .init,
    closing: std.Io.Mutex = .init,
    changed: std.Io.Condition = .init,
    lifetime: Cancellation = .{},
    registration: Cancellation.Registration = .{ .data = null, .forward = cancelNative },
    cancel_failure: ?outcomes.Defect = null,
    active: usize = 0,
    closed: bool = false,
    disposed: bool = false,
    body: ?outcomes.Exit(outcomes.OwnedSlice(u8), Failure) = null,
    result: outcomes.Exit(void, Failure) = .{ .success = {} },
    metadata: *Metadata,
    const Metadata = struct {
        allocator: std.mem.Allocator,
        storage: outcomes.CleanupStorage(Failure),
        fn destroy(data: *anyopaque) void {
            const self: *@This() = @ptrCast(@alignCast(data));
            self.allocator.destroy(self);
        }
    };

    /// Acquire a resource from the selected capability.
    /// Parameters:
    /// - `env: anytype`: Borrowed caller environment supplying allocator, I/O, and selected capabilities.
    /// - `transport: Transport`: Caller-supplied capability or scoped state; no concrete backend is selected
    ///   here.
    /// - `request: Request`: HTTP request borrowed through response acquisition and cleanup.
    /// - `token: ?*const Cancellation`: Cancellation token observed by this operation; its owner must outlive pending
    ///   work.
    /// Returns (`outcomes.Exit(*Self, Failure)`): Carries `*Self` on `.success` or `Failure` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
    pub fn acquire(env: anytype, transport: Transport, request: Request, token: ?*const Cancellation) outcomes.Exit(*Self, Failure) {
        const response = env.allocator.create(Self) catch return .{ .die = outcomes.Defect.out_of_memory };
        const metadata = env.allocator.create(Metadata) catch {
            env.allocator.destroy(response);
            return .{ .die = outcomes.Defect.out_of_memory };
        };
        metadata.* = .{ .allocator = env.allocator, .storage = .{ .allocator = env.allocator, .owner = metadata, .release = Metadata.destroy } };
        response.* = .{ .allocator = env.allocator, .io = env.io, .metadata = metadata };
        const received = transport.send(transport.userdata, request, token);
        if (received != .success) {
            env.allocator.destroy(metadata);
            env.allocator.destroy(response);
            return outcomes.castFailure(*Self, received);
        }
        response.native = received.success;
        response.registration.data = response;
        response.lifetime.register(&response.registration);
        return .{ .success = response };
    }
    /// Read the HTTP response status.
    /// Parameters:
    /// - `self: *const Self`: Borrowed receiver; the caller keeps the referenced object alive through this call, and this pointer cannot mutate it.
    /// Returns (`std.http.Status`): The HTTP status parsed from the response.
    pub fn status(self: *const Self) std.http.Status {
        return self.native.status;
    }
    fn cancelNative(data: ?*anyopaque) void {
        const self: *Self = @ptrCast(@alignCast(data.?));
        self.cancel_failure = self.native.vtable.cancel(self.native.userdata);
    }
    /// Read response bytes while observing cancellation.
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// - `token: ?*const Cancellation`: Cancellation token observed by this operation; its owner must outlive pending
    ///   work.
    /// Returns (`outcomes.Exit(outcomes.OwnedSlice(u8), Failure)`): Carries `outcomes.OwnedSlice(u8)` on `.success` or `Failure` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
    pub fn readToken(self: *Self, token: ?*const Cancellation) outcomes.Exit(outcomes.OwnedSlice(u8), Failure) {
        self.mutex.lockUncancelable(self.io);
        if (self.closed) {
            self.mutex.unlock(self.io);
            return .interrupted;
        }
        self.active += 1;
        self.mutex.unlock(self.io);
        defer {
            self.mutex.lockUncancelable(self.io);
            self.active -= 1;
            self.changed.broadcast(self.io);
            self.mutex.unlock(self.io);
        }
        self.reading.lock(self.io) catch return .interrupted;
        defer self.reading.unlock(self.io);
        var local: Cancellation = .{};
        const Link = struct {
            fn cancel(data: ?*anyopaque) void {
                const cancellation: *Cancellation = @ptrCast(@alignCast(data.?));
                cancellation.request();
            }
        };
        var lifetime: Cancellation.Registration = .{ .data = &local, .forward = Link.cancel };
        var caller: Cancellation.Registration = .{ .data = self, .forward = struct {
            fn abort(data: ?*anyopaque) void {
                const response: *Self = @ptrCast(@alignCast(data.?));
                response.lifetime.request();
            }
        }.abort };
        self.lifetime.register(&lifetime);
        if (token) |origin| origin.register(&caller);
        defer {
            caller.unregister();
            lifetime.unregister();
        }
        if (local.isRequested()) return .interrupted;
        if (self.body == null) self.body = self.native.vtable.read(self.native.userdata, &local);
        if (local.isRequested()) return .interrupted;
        return self.body.?.clone();
    }
    /// Close this scoped resource.
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`outcomes.Exit(void, Failure)`): Carries no value on `.success` or `Failure` on `.fail`; `.die`, `.interrupt`, and `.cleanup` preserve other terminal causes.
    pub fn close(self: *Self) outcomes.Exit(void, Failure) {
        const protection = self.io.swapCancelProtection(.blocked);
        defer _ = self.io.swapCancelProtection(protection);
        self.closing.lockUncancelable(self.io);
        defer self.closing.unlock(self.io);
        if (self.disposed) return self.result.clone();
        self.mutex.lockUncancelable(self.io);
        self.closed = true;
        self.mutex.unlock(self.io);
        self.lifetime.request();
        self.mutex.lockUncancelable(self.io);
        while (self.active != 0) self.changed.waitUncancelable(self.io, &self.mutex);
        self.mutex.unlock(self.io);
        self.registration.unregister();
        if (self.cancel_failure) |defect| self.metadata.storage.append(&self.result, .{ .release_defect = defect });
        self.metadata.storage.appendExit(&self.result, self.native.vtable.dispose(self.native.userdata));
        self.disposed = true;
        return self.result.clone();
    }
    /// Release resources owned by this value.
    /// Parameters:
    /// - `self: *Self`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`void`): no value.
    pub fn deinit(self: *Self) void {
        std.debug.assert(self.disposed and self.active == 0);
        if (self.body) |*body| body.deinit();
        const retained = self.metadata.storage.retained();
        self.result.deinit();
        if (!retained) self.allocator.destroy(self.metadata);
        self.allocator.destroy(self);
    }
};

/// Native HTTP policy is entirely consumer supplied. Its Client carries the
/// consumer allocator and I/O provider; no hidden client or redirect policy.
pub const Native = struct {
    client: *std.http.Client,
    options: std.http.Client.RequestOptions,
    redirect_buffer_bytes: usize,
    transfer_buffer_bytes: usize,
    maximum_body_bytes: usize,

    /// Expose this implementation as an HTTP transport capability.
    /// Parameters:
    /// - `self: *Native`: Mutable borrowed receiver; this method may change or invalidate its state, as specified in the operation contract.
    /// Returns (`Transport`): A capability adapter referencing the selected implementation; keep its backing state alive while in use.
    pub fn transport(self: *Native) Transport {
        return .{ .userdata = self, .send = send };
    }
    const Owner = struct {
        adapter: *Native,
        request: std.http.Client.Request = undefined,
        response: std.http.Client.Response = undefined,
        redirects: []u8,
        transfers: []u8,
        payload: ?[]u8,
        arena: std.heap.ArenaAllocator,
        io: std.Io,
        cancelled: std.atomic.Value(bool) = .init(false),
        uri: []const u8,
        method: std.http.Method,
        headers: []const std.http.Header,

        fn sendBody(self: *@This(), _: *const @This()) outcomes.Exit(NativeResponse, Failure) {
            const uri = std.Uri.parse(self.uri) catch |err| return transportFailure(NativeResponse, err);
            var options = self.adapter.options;
            options.extra_headers = self.headers;
            self.request = self.adapter.client.request(self.method, uri, options) catch |err| return transportFailure(NativeResponse, err);
            var owned = true;
            defer if (owned) self.request.deinit();
            if (self.payload) |body| {
                self.request.transfer_encoding = .{ .content_length = body.len };
                self.request.sendBodyComplete(body) catch |err| return transportFailure(NativeResponse, err);
            } else self.request.sendBodiless() catch |err| return transportFailure(NativeResponse, err);
            self.response = self.request.receiveHead(self.redirects) catch |err| return transportFailure(NativeResponse, err);
            owned = false;
            return .{ .success = .{ .userdata = self, .status = self.response.head.status, .vtable = &.{ .read = read, .cancel = cancel, .dispose = dispose } } };
        }
        fn read(data: *anyopaque, token: ?*const Cancellation) outcomes.Exit(outcomes.OwnedSlice(u8), Failure) {
            const self: *@This() = @ptrCast(@alignCast(data));
            if (self.cancelled.load(.acquire)) return .interrupted;
            return native_operation.run(self, self, token, readBody);
        }
        fn readBody(self: *@This(), _: *@This()) outcomes.Exit(outcomes.OwnedSlice(u8), Failure) {
            const bytes = self.response.reader(self.transfers).allocRemaining(self.adapter.client.allocator, .limited(self.adapter.maximum_body_bytes)) catch |err| return decodeFailure(err);
            return .{ .success = .{ .items = bytes, .allocator = self.adapter.client.allocator } };
        }
        fn cancel(data: *anyopaque) ?outcomes.Defect {
            const self: *@This() = @ptrCast(@alignCast(data));
            self.cancelled.store(true, .release);
            return null;
        }
        fn dispose(data: *anyopaque) outcomes.Exit(void, Failure) {
            const self: *@This() = @ptrCast(@alignCast(data));
            self.request.deinit();
            destroy(self);
            return .{ .success = {} };
        }
        fn destroy(self: *@This()) void {
            const allocator = self.adapter.client.allocator;
            self.arena.deinit();
            allocator.free(self.redirects);
            allocator.free(self.transfers);
            if (self.payload) |body| allocator.free(body);
            allocator.destroy(self);
        }
    };
    fn send(data: *anyopaque, request: Request, token: ?*const Cancellation) outcomes.Exit(NativeResponse, Failure) {
        const self: *Native = @ptrCast(@alignCast(data));
        if (self.transfer_buffer_bytes == 0 or self.redirect_buffer_bytes == 0) return .{ .fail = .{ .transport = .{ .kind = .invalid_configuration, .message = diagnostics.http_transfer_and_redirect_buffers_must_be_non_empty } } };
        for (request.headers) |header| {
            if (header.name.len == 0 or std.mem.indexOfAny(u8, header.name, ":\r\n") != null or std.mem.indexOfAny(u8, header.value, "\r\n") != null) return .{ .fail = .{ .transport = .{ .kind = .invalid_configuration, .message = diagnostics.invalid_http_header } } };
        }
        const allocator = self.client.allocator;
        const owner = allocator.create(Owner) catch return .{ .die = outcomes.Defect.out_of_memory };
        const redirects = allocator.alloc(u8, self.redirect_buffer_bytes) catch {
            allocator.destroy(owner);
            return .{ .die = outcomes.Defect.out_of_memory };
        };
        const transfers = allocator.alloc(u8, self.transfer_buffer_bytes) catch {
            allocator.free(redirects);
            allocator.destroy(owner);
            return .{ .die = outcomes.Defect.out_of_memory };
        };
        const payload: ?[]u8 = if (request.body) |body| allocator.dupe(u8, body) catch {
            allocator.free(transfers);
            allocator.free(redirects);
            allocator.destroy(owner);
            return .{ .die = outcomes.Defect.out_of_memory };
        } else null;
        var arena = std.heap.ArenaAllocator.init(allocator);
        const memory = arena.allocator();
        const uri = memory.dupe(u8, request.uri) catch {
            arena.deinit();
            if (payload) |body| allocator.free(body);
            allocator.free(transfers);
            allocator.free(redirects);
            allocator.destroy(owner);
            return .{ .die = outcomes.Defect.out_of_memory };
        };
        const headers = memory.alloc(std.http.Header, request.headers.len) catch {
            arena.deinit();
            if (payload) |body| allocator.free(body);
            allocator.free(transfers);
            allocator.free(redirects);
            allocator.destroy(owner);
            return .{ .die = outcomes.Defect.out_of_memory };
        };
        for (request.headers, headers) |header, *owned| {
            owned.* = .{ .name = memory.dupe(u8, header.name) catch {
                arena.deinit();
                if (payload) |body| allocator.free(body);
                allocator.free(transfers);
                allocator.free(redirects);
                allocator.destroy(owner);
                return .{ .die = outcomes.Defect.out_of_memory };
            }, .value = memory.dupe(u8, header.value) catch {
                arena.deinit();
                if (payload) |body| allocator.free(body);
                allocator.free(transfers);
                allocator.free(redirects);
                allocator.destroy(owner);
                return .{ .die = outcomes.Defect.out_of_memory };
            } };
        }
        owner.* = .{ .arena = arena, .adapter = self, .redirects = redirects, .transfers = transfers, .payload = payload, .io = self.client.io, .uri = uri, .method = request.method, .headers = headers };
        const received = native_operation.run(owner, owner, token, Owner.sendBody);
        if (received != .success) Owner.destroy(owner);
        return received;
    }
};
fn transportFailure(comptime A: type, err: anyerror) outcomes.Exit(A, Failure) {
    if (err == error.Canceled) return .interrupted;
    if (err == error.OutOfMemory) return .{ .die = outcomes.Defect.out_of_memory };
    return .{ .fail = .{ .transport = .{ .kind = .native_failure, .message = @errorName(err) } } };
}
fn decodeFailure(err: anyerror) outcomes.Exit(outcomes.OwnedSlice(u8), Failure) {
    if (err == error.Canceled) return .interrupted;
    if (err == error.OutOfMemory) return .{ .die = outcomes.Defect.out_of_memory };
    return .{ .fail = .{ .decode = .{ .kind = .native_failure, .message = @errorName(err) } } };
}
