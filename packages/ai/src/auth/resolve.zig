const std = @import("std");
const t = @import("types.zig");
const types = @import("../types.zig");
pub const AuthResolutionOverrides = struct { apiKey: ?[]const u8 = null, env: types.ProviderEnv = .{}, minOAuthValidityMs: ?f64 = null, errorMessage: ?*?[]const u8 = null, jsonFields: std.json.Value = .null, jsonAssigned: u64 = 0, jsonOmitted: u64 = 0, jsonRetained: u64 = 0 };
pub fn remember(a: std.mem.Allocator, slot: ?*?[]const u8, err: anyerror, comptime format: []const u8, args: anytype) !void {
    if (err == error.Canceled or err == error.OutOfMemory) return;
    if (slot) |value| {
        const prefix = try std.fmt.allocPrint(a, format, args);
        defer a.free(prefix);
        value.* = try std.fmt.allocPrint(a, "{s}: {s}", .{ prefix, @errorName(err) });
    }
}
fn apiKey(a: std.mem.Allocator, io: std.Io, id: []const u8, handler: t.ApiKeyAuth, context: t.AuthContext, credential: ?t.ApiKeyCredential, slot: ?*?[]const u8) !?t.AuthResult {
    const result = handler.resolve(handler.context, a, io, context, credential) catch |err| {
        try remember(a, slot, err, "API key auth failed for provider {s}", .{id});
        return err;
    };
    try io.checkCancel();
    return result;
}
/// Owns auth operations that outlive a canceled caller. The allocator must
/// support concurrent use; borrowed handler, store, context and transport
/// contexts remain valid through deinit. Deinit requires callers to be quiescent.
pub const AuthResolutionOwner = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    mutex: std.Io.Mutex = .init,
    jobs: std.ArrayList(*Job) = .empty,
    pub fn init(a: std.mem.Allocator, io: std.Io) @This() {
        return .{ .allocator = a, .io = io };
    }
    pub fn deinit(self: *@This()) void {
        for (self.jobs.items) |job| job.canceled.set(self.io);
        for (self.jobs.items) |job| {
            job.future.await(self.io);
            job.destroy(self.allocator);
        }
        self.jobs.deinit(self.allocator);
        self.* = undefined;
    }
    /// Releases completed abandoned operations, including their private arenas.
    pub fn reap(self: *@This()) void {
        self.mutex.lockUncancelable(self.io);
        defer self.mutex.unlock(self.io);
        var index: usize = 0;
        while (index < self.jobs.items.len) {
            const job = self.jobs.items[index];
            if (job.done.isSet() and job.callerGone.load(.acquire)) {
                _ = self.jobs.swapRemove(index);
                job.future.await(self.io);
                job.destroy(self.allocator);
            } else index += 1;
        }
    }
    const Job = struct {
        arena: std.heap.ArenaAllocator,
        io: std.Io,
        id: []const u8 = undefined,
        auth: t.ProviderAuth = undefined,
        credentials: t.CredentialStore,
        context: t.AuthContext,
        overrides: AuthResolutionOverrides = .{},
        transport: @import("../transport/http.zig").Transport,
        canceled: std.Io.Event = .unset,
        done: std.Io.Event = .unset,
        callerGone: std.atomic.Value(bool) = .init(false),
        future: std.Io.Future(void) = undefined,
        result: anyerror!?t.AuthResult = error.Canceled,
        errorMessage: ?[]const u8 = null,
        fn destroy(self: *@This(), a: std.mem.Allocator) void {
            self.arena.deinit();
            a.destroy(self);
        }
        fn work(self: *@This()) anyerror!?t.AuthResult {
            return resolveProviderAuthInner(self.arena.allocator(), self.io, self.id, self.auth, self.credentials, self.context, self.overrides, self.transport);
        }
        fn run(self: *@This()) void {
            defer self.done.set(self.io);
            const Outcome = union(enum) { work: anyerror!?t.AuthResult, canceled: std.Io.Cancelable!void };
            var buffer: [2]Outcome = undefined;
            var select = std.Io.Select(Outcome).init(self.io, &buffer);
            defer select.cancelDiscard();
            select.concurrent(.work, work, .{self}) catch |err| {
                self.result = err;
                return;
            };
            select.concurrent(.canceled, std.Io.Event.wait, .{ &self.canceled, self.io }) catch |err| {
                self.result = err;
                return;
            };
            const outcome = select.await() catch |err| {
                self.result = err;
                return;
            };
            switch (outcome) {
                .work => |result| self.result = result,
                // Joining the canceled worker happens here, independently of
                // the caller. Active refresh/persistence keeps its protection.
                .canceled => |result| {
                    result catch {};
                    self.result = error.Canceled;
                },
            }
        }
    };
};
fn cloneEnv(a: std.mem.Allocator, env: types.ProviderEnv) !types.ProviderEnv {
    var result: types.ProviderEnv = .{};
    for (env.keys(), env.values()) |key, value| try result.put(a, try a.dupe(u8, key), try a.dupe(u8, value));
    return result;
}
pub fn resolveProviderAuth(a: std.mem.Allocator, io: std.Io, owner: *AuthResolutionOwner, id: []const u8, auth: t.ProviderAuth, credentials: t.CredentialStore, context: t.AuthContext, overrides: AuthResolutionOverrides, transport: @import("../transport/http.zig").Transport) !?t.AuthResult {
    if (overrides.errorMessage) |slot| slot.* = null;
    try io.checkCancel();
    owner.reap();
    const job = try owner.allocator.create(AuthResolutionOwner.Job);
    job.* = .{ .arena = .init(owner.allocator), .io = owner.io, .credentials = credentials, .context = context, .transport = transport };
    var launched = false;
    errdefer if (!launched) job.destroy(owner.allocator);
    const retained = job.arena.allocator();
    job.id = try retained.dupe(u8, id);
    job.auth = try types.cloneInto(t.ProviderAuth, retained, auth);
    job.context.inputs = try types.cloneInto(t.CredentialInputs, retained, context.inputs);
    // The caller's diagnostic pointer and map backing storage must never be
    // retained by a worker after the public cancellation race has returned.
    var copied_overrides = overrides;
    copied_overrides.errorMessage = null;
    copied_overrides.env = .{};
    job.overrides = try types.cloneInto(AuthResolutionOverrides, retained, copied_overrides);
    job.overrides.errorMessage = &job.errorMessage;
    job.overrides.env = try cloneEnv(retained, overrides.env);
    {
        try owner.mutex.lock(io);
        defer owner.mutex.unlock(owner.io);
        try owner.jobs.append(owner.allocator, job);
        errdefer _ = owner.jobs.pop();
        job.future = try owner.io.concurrent(AuthResolutionOwner.Job.run, .{job});
        launched = true;
    }
    defer {
        job.callerGone.store(true, .release);
        owner.reap();
    }
    job.done.wait(io) catch |err| {
        job.canceled.set(owner.io);
        return err;
    };
    io.checkCancel() catch |err| {
        job.canceled.set(owner.io);
        return err;
    };
    if (overrides.errorMessage) |slot| if (job.errorMessage) |message| {
        slot.* = try a.dupe(u8, message);
    };
    const result = (try job.result) orelse return null;
    return .{ .auth = try types.cloneInto(t.ModelAuth, a, result.auth), .source = if (result.source) |source| try a.dupe(u8, source) else null, .env = try cloneEnv(a, result.env) };
}
fn resolveProviderAuthInner(a: std.mem.Allocator, io: std.Io, id: []const u8, auth: t.ProviderAuth, credentials: t.CredentialStore, context: t.AuthContext, overrides: AuthResolutionOverrides, transport: @import("../transport/http.zig").Transport) !?t.AuthResult {
    if (overrides.errorMessage) |slot| slot.* = null;
    try io.checkCancel();
    var overlay = Overlay{ .base = context, .scopedEnv = overrides.env };
    const ctx = t.AuthContext{ .inputs = context.inputs, .context = &overlay, .envFn = Overlay.env, .fileExistsFn = Overlay.exists, .vertexAdcExistsCache = context.vertexAdcExistsCache, .credentialCommandFn = if (context.credentialCommandFn != null) Overlay.command else null, .homeDirFn = if (context.homeDirFn != null) Overlay.home else null };
    if (@import("../json/value.zig").fieldPresent(overrides, "apiKey")) if (auth.apiKey) |handler| {
        var credential = t.ApiKeyCredential{ .key = overrides.apiKey, .env = try @import("helpers.zig").envValue(a, overrides.env) };
        @import("../json/value.zig").markPresent(&credential, "key");
        return apiKey(a, io, id, handler, ctx, credential, overrides.errorMessage);
    };
    const read = credentials.read(a, io, id) catch |err| {
        try remember(a, overrides.errorMessage, err, "Credential store read failed for {s}", .{id});
        return err;
    };
    try io.checkCancel();
    if (read) |stored| switch (stored) {
        .api_key => |key| {
            const handler = auth.apiKey orelse return null;
            var value = key;
            if (overrides.env.count() > 0) {
                const json = @import("../json/value.zig");
                var env = try json.objectSpread(a, key.env orelse .null);
                for (overrides.env.keys(), overrides.env.values()) |name, text| try json.put(a, &env, name, json.str(text));
                value.env = env;
            }
            return apiKey(a, io, id, handler, ctx, value, overrides.errorMessage);
        },
        .oauth => |initial| {
            const handler = auth.oauth orelse return null;
            var credential = initial;
            var original_protection: ?std.Io.CancelProtection = null;
            defer if (original_protection) |protection| {
                _ = io.swapCancelProtection(protection);
            };
            const validity = @import("../utils/numbers.zig").maximum(300_000, overrides.minOAuthValidityMs orelse 0);
            if (@as(f64, @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds())) + validity >= initial.expires) {
                var refresh = Refresh{ .oauth = handler, .validity = validity, .transport = transport, .providerId = id, .errorAllocator = a, .errorMessage = overrides.errorMessage, .authContext = context };
                const post = credentials.modify(a, io, id, .{ .context = &refresh, .call = Refresh.call, .completeOnCancel = true }) catch |err| {
                    if (overrides.errorMessage == null or overrides.errorMessage.?.* == null) try remember(a, overrides.errorMessage, err, "Credential store modify failed for {s}", .{id});
                    return err;
                };
                // Once the transaction callback has started, the source keeps
                // observing refresh and auth derivation after caller abort.
                // The outer owned job races that work with caller cancellation.
                if (refresh.started) original_protection = io.swapCancelProtection(.blocked);
                try io.checkCancel();
                const current = post orelse return null;
                if (current != .oauth) return null;
                credential = current.oauth;
                if (@import("../json/value.zig").fieldPresent(overrides, "minOAuthValidityMs") and @as(f64, @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds())) + validity >= credential.expires) {
                    if (overrides.errorMessage) |slot| slot.* = try std.fmt.allocPrint(a, "OAuth refresh returned a token that expires too soon for {s}", .{id});
                    return error.OAuthTokenExpiresTooSoon;
                }
            }
            // The source toAuth callback takes no caller AbortSignal, even
            // when the stored token did not need a refresh.
            if (original_protection == null) original_protection = io.swapCancelProtection(.blocked);
            const resolved = handler.toAuth(handler.context, a, io, credential) catch |err| {
                try remember(a, overrides.errorMessage, err, "OAuth auth derivation failed for {s}", .{id});
                return err;
            };
            try io.checkCancel();
            return .{ .auth = resolved, .source = "OAuth" };
        },
    };
    const handler = auth.apiKey orelse return null;
    return apiKey(a, io, id, handler, ctx, null, overrides.errorMessage);
}
const Refresh = struct {
    oauth: t.OAuthAuth,
    validity: f64,
    transport: @import("../transport/http.zig").Transport,
    authContext: ?t.AuthContext = null,
    started: bool = false,
    providerId: []const u8,
    errorAllocator: std.mem.Allocator,
    errorMessage: ?*?[]const u8 = null,
    refreshFailureMessage: ?[]const u8 = null,
    fn renew(self: *@This(), a: std.mem.Allocator, io: std.Io, credential: t.OAuthCredential) !t.OAuthCredential {
        return self.oauth.refreshCredentialWithContext(a, io, self.transport, credential, self.authContext, &self.refreshFailureMessage);
    }
    fn call(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, current: ?t.Credential) !?t.Credential {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        self.started = true;
        const c = current orelse return null;
        if (c != .oauth) return null;
        if (!(@as(f64, @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds())) + self.validity >= c.oauth.expires)) return null;
        const Outcome = union(enum) { credential: anyerror!t.OAuthCredential, timeout: std.Io.Cancelable!void };
        var buffer: [2]Outcome = undefined;
        var select = std.Io.Select(Outcome).init(io, &buffer);
        defer select.cancelDiscard();
        try select.concurrent(.credential, renew, .{ self, a, io, c.oauth });
        try select.concurrent(.timeout, timeout, .{io});
        return switch (try select.await()) {
            .credential => |result| .{ .oauth = result catch |err| {
                if (err != error.Canceled and err != error.OutOfMemory) {
                    if (self.errorMessage) |slot| slot.* = try std.fmt.allocPrint(self.errorAllocator, "OAuth refresh failed for {s}: {s}", .{ self.providerId, self.refreshFailureMessage orelse @errorName(err) });
                }
                return err;
            } },
            .timeout => |result| {
                try result;
                select.cancelDiscard();
                try remember(self.errorAllocator, self.errorMessage, error.OAuthRefreshTimeout, "OAuth refresh failed for {s}", .{self.providerId});
                return error.OAuthRefreshTimeout;
            },
        };
    }
    fn timeout(io: std.Io) std.Io.Cancelable!void {
        try io.sleep(.fromMilliseconds(15_000), .awake);
    }
};
pub fn refreshStoredOAuthCredential(a: std.mem.Allocator, io: std.Io, id: []const u8, oauth: t.OAuthAuth, credentials: t.CredentialStore, transport: @import("../transport/http.zig").Transport, validity: f64) !?t.Credential {
    return refreshStoredOAuthCredentialWithContext(a, io, id, oauth, credentials, transport, validity, null);
}
pub fn refreshStoredOAuthCredentialWithContext(a: std.mem.Allocator, io: std.Io, id: []const u8, oauth: t.OAuthAuth, credentials: t.CredentialStore, transport: @import("../transport/http.zig").Transport, validity: f64, context: ?t.AuthContext) !?t.Credential {
    return refreshStoredOAuthCredentialWithError(a, io, id, oauth, credentials, transport, validity, context, null);
}
pub fn refreshStoredOAuthCredentialWithError(a: std.mem.Allocator, io: std.Io, id: []const u8, oauth: t.OAuthAuth, credentials: t.CredentialStore, transport: @import("../transport/http.zig").Transport, validity: f64, context: ?t.AuthContext, error_message: ?*?[]const u8) !?t.Credential {
    if (error_message) |slot| slot.* = null;
    var refresh = Refresh{ .oauth = oauth, .validity = validity, .transport = transport, .providerId = id, .errorAllocator = a, .authContext = context, .errorMessage = error_message };
    const post = credentials.modify(a, io, id, .{ .context = &refresh, .call = Refresh.call, .completeOnCancel = true }) catch |err| {
        if (error_message == null or error_message.?.* == null) try remember(a, error_message, err, "Credential store modify failed for {s}", .{id});
        return err;
    };
    const current = post orelse return null;
    return if (current == .oauth) current else null;
}
const Overlay = struct {
    base: t.AuthContext,
    scopedEnv: types.ProviderEnv,
    fn env(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, name: []const u8) !?[]const u8 {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        if (self.scopedEnv.get(name)) |v| if (v.len > 0) return try a.dupe(u8, v);
        return self.base.env(a, io, name);
    }
    fn exists(ctx: ?*anyopaque, io: std.Io, path: []const u8) !bool {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        return self.base.fileExistsFn(self.base.context, io, path);
    }
    fn home(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io) !?[]const u8 {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        return self.base.homeDirFn.?(self.base.context, a, io);
    }
    fn command(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, input: t.CredentialCommand) !std.json.Value {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        return self.base.credentialCommandFn.?(self.base.context, a, io, input);
    }
};
