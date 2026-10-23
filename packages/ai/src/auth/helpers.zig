const std = @import("std");
const t = @import("types.zig");
const Transport = @import("../transport/http.zig").Transport;
pub const LazyOAuthInput = struct {
    name: []const u8,
    isSubscription: bool = false,
    loginLabel: ?[]const u8 = null,
    load: struct {
        context: ?*anyopaque = null,
        call: *const fn (?*anyopaque, std.mem.Allocator, std.Io) anyerror!t.OAuthAuth,
    },
};
/// Keep this wrapper at a stable address while its auth interface is in use.
/// Metadata and the loader context are borrowed. The loader can retain native
/// implementation data in its supplied arena, which lives until deinit. Join
/// all auth calls before deinit; loading must not reenter this same wrapper.
pub const LazyOAuth = struct {
    input: LazyOAuthInput,
    arena: std.heap.ArenaAllocator,
    mutex: std.Io.Mutex = .init,
    state: union(enum) { unloaded, loaded: t.OAuthAuth, failed: anyerror } = .unloaded,
    pub fn auth(self: *@This()) t.OAuthAuth {
        return .{ .name = self.input.name, .isSubscription = self.input.isSubscription, .loginLabel = self.input.loginLabel, .context = self, .login = login, .refresh = refresh, .refreshWithError = refreshWithError, .refreshWithContext = refreshWithContext, .toAuth = toAuth };
    }
    pub fn deinit(self: *@This()) void {
        self.arena.deinit();
        self.* = undefined;
    }
    fn loaded(self: *@This(), io: std.Io) !t.OAuthAuth {
        // The source's shared import promise is independent of an individual
        // operation's signal. Cancellation reaches the selected auth method
        // after loading; it cannot cancel or poison another caller's load.
        const protection = io.swapCancelProtection(.blocked);
        defer _ = io.swapCancelProtection(protection);
        self.mutex.lockUncancelable(io);
        defer self.mutex.unlock(io);
        if (self.state == .unloaded) {
            const implementation = self.input.load.call(self.input.load.context, self.arena.allocator(), io) catch |err| {
                self.state = .{ .failed = err };
                return err;
            };
            self.state = .{ .loaded = implementation };
        }
        return switch (self.state) {
            .loaded => |implementation| implementation,
            .failed => |err| err,
            .unloaded => unreachable,
        };
    }
    fn owner(ctx: ?*anyopaque) *@This() {
        return @ptrCast(@alignCast(ctx.?));
    }
    fn login(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: Transport, interaction: t.Interaction, options: t.LoginOptions) !t.OAuthCredential {
        const implementation = try owner(ctx).loaded(io);
        return implementation.login(implementation.context, a, io, network, interaction, options);
    }
    fn refresh(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: Transport, credential: t.OAuthCredential) !t.OAuthCredential {
        return refreshWithContext(ctx, a, io, network, credential, null, null);
    }
    fn refreshWithError(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: Transport, credential: t.OAuthCredential, error_message: ?*?[]const u8) !t.OAuthCredential {
        return refreshWithContext(ctx, a, io, network, credential, null, error_message);
    }
    fn refreshWithContext(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: Transport, credential: t.OAuthCredential, context: ?t.AuthContext, error_message: ?*?[]const u8) !t.OAuthCredential {
        const implementation = try owner(ctx).loaded(io);
        return implementation.refreshCredentialWithContext(a, io, network, credential, context, error_message);
    }
    fn toAuth(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, credential: t.OAuthCredential) !t.ModelAuth {
        const implementation = try owner(ctx).loaded(io);
        return implementation.toAuth(implementation.context, a, io, credential);
    }
};
pub fn lazyOAuth(a: std.mem.Allocator, input: LazyOAuthInput) LazyOAuth {
    return .{ .input = input, .arena = .init(a) };
}
pub fn envApiKeyAuth(comptime name: []const u8, comptime env_vars: []const []const u8) t.ApiKeyAuth {
    return .{ .name = name, .login = struct {
        fn login(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, interaction: t.Interaction) !t.ApiKeyCredential {
            try io.checkCancel();
            const prompt = try @import("../json/value.zig").from(a, .{ .type = "secret", .message = "Enter " ++ name });
            const key = try interaction.prompt(interaction.context, a, io, prompt);
            try io.checkCancel();
            return .{ .key = key };
        }
    }.login, .resolve = struct {
        fn resolve(_: ?*anyopaque, a: std.mem.Allocator, io: std.Io, ctx: t.AuthContext, credential: ?t.ApiKeyCredential) !?t.AuthResult {
            try io.checkCancel();
            if (credential) |c| if (c.key) |key| if (key.len > 0) return .{ .auth = .{ .apiKey = key }, .source = "stored credential", .env = try credentialEnv(a, c.env) };
            inline for (env_vars) |variable| {
                const value = try ctx.env(a, io, variable);
                errdefer if (value) |text| a.free(text);
                try io.checkCancel();
                if (value) |text| if (text.len > 0) return .{ .auth = .{ .apiKey = text }, .source = variable };
                if (value) |text| a.free(text);
            }
            return null;
        }
    }.resolve };
}
pub fn credentialEnv(a: std.mem.Allocator, value: ?std.json.Value) !@import("../types.zig").ProviderEnv {
    var env: @import("../types.zig").ProviderEnv = .{};
    if (value) |v| if (v == .object) {
        var it = v.object.iterator();
        while (it.next()) |entry| if (entry.value_ptr.* == .string) {
            try env.put(a, entry.key_ptr.*, entry.value_ptr.string);
        };
    };
    return env;
}
pub fn envValue(a: std.mem.Allocator, env: @import("../types.zig").ProviderEnv) !std.json.Value {
    const j = @import("../json/value.zig");
    var value = j.map(a);
    for (env.keys(), env.values()) |key, text| try j.put(a, &value, key, j.str(text));
    return value;
}
