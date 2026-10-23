//! App-owned loopback HTTP listener. std.http parses all HTTP framing; std.Uri
//! parses the callback target and decodes percent-encoded components.
const std = @import("std");
const uri = @import("../../transport/uri.zig");
const j = @import("../../json/value.zig");
const auth = @import("../types.zig");
fn callbackAddress(a: std.mem.Allocator, io: std.Io, host: []const u8, port: u16) !std.Io.net.IpAddress {
    if (std.Io.net.IpAddress.parse(host, port)) |address| return address else |_| {}
    // Node GetAddrInfo applies Ada IDNA before the platform lookup. Retain
    // the configured spelling in redirectUri; this owner ends after lookup.
    const ascii = try @import("../../transport/host_idna.zig").toAscii(a, host);
    defer a.free(ascii);
    const name = try std.Io.net.HostName.init(ascii);
    // The standard lookup contract guarantees a complete lookup fits this
    // queue without a concurrent consumer. No resolver task outlives start().
    var results: [16]std.Io.net.HostName.LookupResult = undefined;
    var queue = std.Io.Queue(std.Io.net.HostName.LookupResult).init(&results);
    try name.lookup(io, &queue, .{ .port = port });
    var first: ?std.Io.net.IpAddress = null;
    while (true) {
        const result = queue.getOne(io) catch |err| switch (err) {
            error.Closed => break,
            else => return err,
        };
        switch (result) {
            .canonical_name => {},
            .address => |address| {
                if (first == null) first = address;
                const link_local = switch (address) {
                    .ip4 => false,
                    .ip6 => |ip6| ip6.isLinkLocal(),
                };
                if (!link_local) return address;
            },
        }
    }
    return first orelse error.NoAddressReturned;
}
fn callbackTarget(a: std.mem.Allocator, text: []const u8) !std.Uri {
    const base = std.Uri{ .scheme = "http", .host = .{ .raw = "localhost" }, .path = .{ .raw = "/" } };
    return uri.resolveHttpUrl(a, base, text);
}
pub fn CallbackServer(comptime T: type) type {
    return struct {
        allocator: std.mem.Allocator,
        io: std.Io,
        server: std.Io.net.Server,
        redirectUri: []const u8,
        options: Options,
        future: ?std.Io.Future(void) = null,
        mutex: std.Io.Mutex = .init,
        condition: std.Io.Condition = .init,
        claimed: bool = false,
        settled: bool = false,
        result: ?T = null,
        failure: ?anyerror = null,
        arena: std.heap.ArenaAllocator,
        completionArena: std.heap.ArenaAllocator,
        deadline: ?std.Io.Timestamp = null,
        failureMessage: ?[]const u8 = null,
        timeoutMessage: ?[]const u8 = null,
        const Self = @This();
        pub const QueryResult = union(enum) {
            retry: []const u8,
            failed: struct { err: anyerror, message: []const u8, responseMessage: []const u8, responseDetails: ?[]const u8 = null },
            complete: struct { value: T, message: []const u8 },
        };
        pub const Options = struct {
            providerName: []const u8,
            host: []const u8,
            port: u16,
            path: []const u8,
            redirectHost: ?[]const u8 = null,
            state: ?[]const u8 = null,
            timeoutMs: ?u64 = null,
            context: ?*anyopaque = null,
            complete: ?*const fn (?*anyopaque, std.mem.Allocator, std.Io, []const u8) anyerror!T = null,
            /// A method-independent query callback supplies its own validation
            /// and terminal/retry decision, as required by registration flows.
            /// Its allocator and URI borrow the request; published decisions
            /// are copied into server storage before the request is released.
            queryHandler: ?*const fn (?*anyopaque, std.mem.Allocator, std.Io, std.Uri) anyerror!QueryResult = null,
            queryResponseHtml: bool = false,
        };
        /// Callback context is borrowed through close(); retained option strings
        /// and callback results belong to this server until close(). Use backing
        /// storage that supports concurrent allocation while the listener runs.
        pub fn start(a: std.mem.Allocator, io: std.Io, options: Options) !*Self {
            try io.checkCancel();
            if (options.complete == null and options.queryHandler == null) return error.MissingOAuthCallbackHandler;
            const address = try callbackAddress(a, io, options.host, options.port);
            var server = try address.listen(io, .{ .reuse_address = true });
            errdefer server.deinit(io);
            const self = try a.create(Self);
            errdefer a.destroy(self);
            self.* = .{ .allocator = a, .io = io, .server = server, .redirectUri = undefined, .options = options, .arena = .init(a), .completionArena = .init(a) };
            errdefer self.arena.deinit();
            errdefer self.completionArena.deinit();
            const alloc = self.arena.allocator();
            self.options.providerName = try alloc.dupe(u8, options.providerName);
            self.options.host = try alloc.dupe(u8, options.host);
            self.options.path = try alloc.dupe(u8, options.path);
            self.options.redirectHost = if (options.redirectHost) |host| try alloc.dupe(u8, host) else null;
            self.options.state = if (options.state) |state| try alloc.dupe(u8, state) else null;
            const redirect_host = self.options.redirectHost orelse self.options.host;
            const authority_host = if (std.mem.findScalar(u8, redirect_host, ':') != null) try std.fmt.allocPrint(alloc, "[{s}]", .{redirect_host}) else redirect_host;
            const redirect = std.Uri{ .scheme = "http", .host = .{ .percent_encoded = authority_host }, .port = server.socket.address.getPort(), .path = .{ .raw = self.options.path } };
            self.redirectUri = try uri.render(alloc, redirect);
            if (options.timeoutMs) |milliseconds| {
                self.timeoutMessage = try std.fmt.allocPrint(alloc, "{s} sign-in timed out", .{self.options.providerName});
                self.deadline = std.Io.Clock.awake.now(io).addDuration(.fromNanoseconds(@as(i96, milliseconds) * std.time.ns_per_ms));
            }
            self.future = try io.concurrent(run, .{self});
            return self;
        }
        fn finish(self: *Self, result: ?T, failure: ?anyerror) void {
            self.finishWithMessage(result, failure, if (failure) |err| switch (err) {
                error.Canceled => "Login cancelled",
                error.OAuthCallbackServerClosed => "OAuth callback server closed",
                else => null,
            } else null);
        }
        fn finishWithMessage(self: *Self, result: ?T, failure: ?anyerror, message: ?[]const u8) void {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (self.settled) return;
            self.result = result;
            self.failure = failure;
            self.failureMessage = message;
            self.settled = true;
            self.condition.broadcast(self.io);
        }
        fn retainValue(self: *Self, value: T) !T {
            try self.mutex.lock(self.io);
            defer self.mutex.unlock(self.io);
            return @import("../../types.zig").cloneInto(T, self.arena.allocator(), value);
        }
        fn retainText(self: *Self, text: []const u8) ![]const u8 {
            try self.mutex.lock(self.io);
            defer self.mutex.unlock(self.io);
            return self.arena.allocator().dupe(u8, text);
        }
        fn run(self: *Self) void {
            var timer: ?std.Io.Future(void) = null;
            if (self.deadline != null) timer = self.io.concurrent(expire, .{self}) catch |err| {
                self.finish(null, err);
                if (err == error.Canceled) self.io.recancel();
                return;
            };
            defer if (timer) |*future| future.cancel(self.io);
            var handlers: std.Io.Group = .init;
            defer handlers.cancel(self.io);
            self.serve(&handlers) catch |err| {
                self.finish(null, err);
                if (err == error.Canceled) self.io.recancel();
            };
        }
        fn expire(self: *Self) void {
            self.mutex.lock(self.io) catch |err| {
                if (err == error.Canceled) self.io.recancel();
                return;
            };
            defer self.mutex.unlock(self.io);
            while (!self.settled) self.condition.waitTimeout(self.io, &self.mutex, .{ .deadline = .{ .raw = self.deadline.?, .clock = .awake } }) catch |err| {
                if (err == error.Canceled) {
                    self.io.recancel();
                    return;
                }
                if (self.settled) return;
                self.failure = error.OAuthCallbackTimeout;
                self.failureMessage = self.timeoutMessage;
                self.settled = true;
                self.condition.broadcast(self.io);
                return;
            };
        }
        fn serve(self: *Self, handlers: *std.Io.Group) !void {
            while (true) try self.acceptOne(handlers);
        }
        fn acceptOne(self: *Self, handlers: *std.Io.Group) !void {
            const connection = try self.server.accept(self.io);
            errdefer connection.close(self.io);
            const handler = try self.allocator.create(Handler);
            errdefer self.allocator.destroy(handler);
            handler.* = .{ .owner = self, .connection = connection, .arena = .init(self.allocator) };
            try handlers.concurrent(self.io, Handler.run, .{handler});
        }
        const Handler = struct {
            owner: *Self,
            connection: std.Io.net.Stream,
            arena: std.heap.ArenaAllocator,
            fn run(handler: *@This()) std.Io.Cancelable!void {
                const owner = handler.owner;
                defer {
                    handler.connection.close(owner.io);
                    handler.arena.deinit();
                    owner.allocator.destroy(handler);
                }
                handler.handle() catch |err| {
                    if (err == error.Canceled) {
                        owner.finish(null, err);
                        return error.Canceled;
                    }
                    if (err == error.OutOfMemory) owner.finish(null, err);
                };
            }
            fn handle(handler: *@This()) !void {
                const self = handler.owner;
                const a = handler.arena.allocator();
                var input_buffer: [8192]u8 = undefined;
                var output_buffer: [4096]u8 = undefined;
                var input = handler.connection.reader(self.io, &input_buffer);
                var output = handler.connection.writer(self.io, &output_buffer);
                var http = std.http.Server.init(&input.interface, &output.interface);
                var request = try http.receiveHead();
                const target = callbackTarget(a, request.head.target) catch |err| {
                    if (self.options.queryResponseHtml) return respondPage(a, &request, .internal_server_error, "Internal error while processing the callback.", null);
                    return err;
                };
                const path = switch (target.path) {
                    .raw, .percent_encoded => |text| text,
                };
                if ((self.options.queryHandler == null and request.head.method != .GET) or !std.mem.eql(u8, path, self.options.path)) {
                    if (self.options.queryHandler != null and !self.options.queryResponseHtml) try respondQuery(&request, .not_found, "Callback route not found.") else try respond(a, &request, .not_found, "Callback route not found.");
                    return;
                }
                if (self.options.queryHandler) |handle_query| {
                    const decision = try handle_query(self.options.context, a, self.io, target);
                    switch (decision) {
                        .retry => |message| if (self.options.queryResponseHtml) try respondPage(a, &request, .bad_request, message, null) else try respondQuery(&request, .bad_request, message),
                        .failed => |failure| {
                            const detail = try self.retainText(failure.message);
                            defer self.finishWithMessage(null, failure.err, detail);
                            if (self.options.queryResponseHtml) try respondPage(a, &request, .bad_request, failure.responseMessage, failure.responseDetails) else try respondQuery(&request, .bad_request, failure.responseMessage);
                        },
                        .complete => |completion| {
                            const value = try self.retainValue(completion.value);
                            defer self.finish(value, null);
                            if (self.options.queryResponseHtml) try respondPage(a, &request, .ok, completion.message, null) else try respondQuery(&request, .ok, completion.message);
                        },
                    }
                    return;
                }
                const query: std.Uri.Component = target.query orelse .{ .percent_encoded = "" };
                const state = try uri.queryValue(a, query, "state");
                if (self.options.state) |expected| if (state == null or !std.mem.eql(u8, state.?, expected)) {
                    try respond(a, &request, .bad_request, "State mismatch.");
                    return;
                };
                try self.mutex.lock(self.io);
                const handled = self.claimed or self.settled;
                self.mutex.unlock(self.io);
                if (handled) {
                    try respond(a, &request, .conflict, "This sign-in has already been handled.");
                    return;
                }
                if (try uri.queryValue(a, query, "error")) |failure| if (failure.len > 0) {
                    const description = (try uri.queryValue(a, query, "error_description")) orelse failure;
                    const detail = try self.retainText(try std.fmt.allocPrint(a, "{s} authorization failed: {s}", .{ self.options.providerName, description }));
                    defer self.finishWithMessage(null, error.OAuthAuthorizationFailed, detail);
                    try respondPage(a, &request, .bad_request, try std.fmt.allocPrint(a, "{s} authorization failed.", .{self.options.providerName}), description);
                    return;
                };
                const code = (try uri.queryValue(a, query, "code")) orelse "";
                if (code.len == 0) {
                    try respond(a, &request, .bad_request, "Missing authorization code.");
                    return;
                }
                try self.mutex.lock(self.io);
                const can_claim = !self.claimed and !self.settled;
                if (can_claim) self.claimed = true;
                self.mutex.unlock(self.io);
                if (!can_claim) {
                    try respond(a, &request, .conflict, "This sign-in has already been handled.");
                    return;
                }
                // Only the claimed handler uses this region. Callback-owned
                // diagnostics must remain valid after its connection closes.
                const completion_allocator = self.completionArena.allocator();
                const value = self.options.complete.?(self.options.context, completion_allocator, self.io, code) catch |err| {
                    defer self.finish(null, err);
                    try respondPage(a, &request, .bad_gateway, try std.fmt.allocPrint(a, "{s} sign-in failed.", .{self.options.providerName}), @errorName(err));
                    return;
                };
                const retained = try self.retainValue(value);
                defer self.finish(retained, null);
                try respond(a, &request, .ok, try std.fmt.allocPrint(a, "Signed in to {s}. You may now close this page.", .{self.options.providerName}));
            }
        };
        fn respond(a: std.mem.Allocator, request: *std.http.Server.Request, status: std.http.Status, text: []const u8) !void {
            return respondPage(a, request, status, text, null);
        }
        fn respondPage(a: std.mem.Allocator, request: *std.http.Server.Request, status: std.http.Status, message: []const u8, details: ?[]const u8) !void {
            var temporary = std.heap.ArenaAllocator.init(a);
            defer temporary.deinit();
            const pages = @import("../../utils/oauth_page.zig");
            const html = if (status == .ok) try pages.oauthSuccessHtml(temporary.allocator(), message) else try pages.oauthErrorHtml(temporary.allocator(), message, details);
            try request.respond(html, .{ .status = status, .extra_headers = &.{ .{ .name = "content-type", .value = "text/html; charset=utf-8" }, .{ .name = "cache-control", .value = "no-store" } }, .keep_alive = false });
        }
        fn respondQuery(request: *std.http.Server.Request, status: std.http.Status, text: []const u8) !void {
            try request.respond(text, .{ .status = status, .extra_headers = &.{.{ .name = "content-type", .value = "text/plain; charset=utf-8" }}, .keep_alive = false });
        }
        pub fn wait(self: *Self) !?T {
            try self.mutex.lock(self.io);
            defer self.mutex.unlock(self.io);
            while (!self.settled) try self.condition.wait(self.io, &self.mutex);
            if (self.failure) |err| return err;
            return self.result;
        }
        /// The diagnostic view borrows this server through close().
        pub fn failureText(self: *Self) ?[]const u8 {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            return self.failureMessage;
        }
        pub fn waitWithError(self: *Self, a: std.mem.Allocator, error_message: ?*?[]const u8) !?T {
            return self.wait() catch |err| {
                if (error_message) |slot| if (if (err == error.Canceled) @as(?[]const u8, "Login cancelled") else self.failureText()) |message| {
                    slot.* = try a.dupe(u8, message);
                };
                return err;
            };
        }
        pub fn cancel(self: *Self) void {
            self.mutex.lockUncancelable(self.io);
            defer self.mutex.unlock(self.io);
            if (!self.claimed and !self.settled) {
                self.result = null;
                self.failure = null;
                self.settled = true;
                self.condition.broadcast(self.io);
            }
        }
        /// Single owner calls close after all waiters have joined.
        pub fn stop(self: *Self) void {
            self.finish(null, error.OAuthCallbackServerClosed);
            if (self.future) |*future| {
                future.cancel(self.io);
                self.future = null;
            }
        }
        pub fn close(self: *Self) void {
            self.stop();
            self.server.deinit(self.io);
            self.completionArena.deinit();
            self.arena.deinit();
            const a = self.allocator;
            a.destroy(self);
        }
    };
}
pub fn CallbackOrManual(comptime T: type) type {
    return union(enum) { callback: T, manual: []const u8 };
}
pub fn waitForCallbackOrManualInput(comptime T: type, a: std.mem.Allocator, io: std.Io, interaction: auth.Interaction, callback: ?*CallbackServer(T), message: []const u8, placeholder: []const u8) !CallbackOrManual(T) {
    return waitForCallbackOrManualInputInner(T, a, io, interaction, callback, message, placeholder) catch |err| {
        // The inner select has joined both tasks before the external diagnostic
        // slot is written or a server-owned message is copied into caller storage.
        if (interaction.errorMessage) |slot| {
            const detail = if (err == error.Canceled) @as(?[]const u8, "Login cancelled") else if (callback) |server| server.failureText() else null;
            if (detail) |text| slot.* = try a.dupe(u8, text);
        }
        return err;
    };
}
fn waitForCallbackOrManualInputInner(comptime T: type, a: std.mem.Allocator, io: std.Io, interaction: auth.Interaction, callback: ?*CallbackServer(T), message: []const u8, placeholder: []const u8) !CallbackOrManual(T) {
    const Result = CallbackOrManual(T);
    const Prompt = struct {
        fn call(alloc: std.mem.Allocator, backend: std.Io, handle: auth.Interaction, prompt: std.json.Value) anyerror![]const u8 {
            return handle.prompt(handle.context, alloc, backend, prompt);
        }
        fn owned(operation: auth.PromptOperation, alloc: std.mem.Allocator, backend: std.Io) anyerror![]const u8 {
            return operation.wait(alloc, backend);
        }
        fn wait(server: *CallbackServer(T)) anyerror!?T {
            return server.wait();
        }
    };
    const prompt = try j.from(a, .{ .type = "manual_code", .message = message, .placeholder = placeholder });
    var owned_prompt: ?auth.PromptOperation = if (interaction.startPrompt) |start| try start(interaction.context, a, io, prompt) else null;
    defer if (owned_prompt) |*operation| operation.deinit();
    if (callback == null) return Result{ .manual = if (owned_prompt) |operation| try operation.wait(a, io) else try Prompt.call(a, io, interaction, prompt) };
    const Outcome = union(enum) { callback: anyerror!?T, manual: anyerror![]const u8 };
    var buffer: [2]Outcome = undefined;
    var select = std.Io.Select(Outcome).init(io, &buffer);
    defer {
        if (owned_prompt) |operation| operation.cancel();
        select.cancelDiscard();
    }
    if (owned_prompt) |operation| try select.concurrent(.manual, Prompt.owned, .{ operation, a, io }) else try select.concurrent(.manual, Prompt.call, .{ a, io, interaction, prompt });
    try select.concurrent(.callback, Prompt.wait, .{callback.?});
    switch (try select.await()) {
        .manual => |result| {
            callback.?.cancel();
            const value = try callback.?.wait();
            const input = try result;
            return if (value) |completed| .{ .callback = completed } else .{ .manual = input };
        },
        .callback => |result| {
            if (try result) |value| return .{ .callback = value };
        },
    }
    return switch (try select.await()) {
        .manual => |result| .{ .manual = try result },
        .callback => unreachable,
    };
}
