const std = @import("std");
const t = @import("../types.zig");
const m = @import("../models.zig");
const auth = @import("../auth/types.zig");
const j = @import("../json/value.zig");
const uri = @import("../transport/uri.zig");
pub const DEFAULT_RADIUS_GATEWAY = "https://radius.pi.dev";
pub const RadiusGatewayModel = std.json.Value;
pub const RadiusGatewayConfig = std.json.Value;
pub const RadiusOAuthCredential = auth.OAuthCredential;
pub const RadiusProviderOptions = struct { id: []const u8 = "radius", name: []const u8 = "Radius", gateway: []const u8 = DEFAULT_RADIUS_GATEWAY };
pub fn normalizeRadiusGatewayUrl(a: std.mem.Allocator, value: []const u8) ![]const u8 {
    const with_scheme = try @import("../regex/regexp.zig").matches(a, null, "^https?://", value, .{ .ignoreCase = true, .unicode = true });
    const candidate = if (with_scheme) value else try std.fmt.allocPrint(a, "https://{s}", .{value});
    defer if (!with_scheme) a.free(candidate);
    return a.dupe(u8, std.mem.trimEnd(u8, candidate, "/"));
}
fn validModel(raw: std.json.Value) bool {
    if (raw != .object) return false;
    return j.field(raw, "id") != null and j.field(raw, "name") != null and j.boolean(j.get(raw, "reasoning") orelse .null) != null and (j.get(raw, "input") orelse .null) == .array and (j.get(raw, "cost") orelse .null) == .object and j.number(j.get(raw, "contextWindow") orelse .null) != null and j.number(j.get(raw, "maxTokens") orelse .null) != null;
}
fn sanitizeRadiusGatewayConfig(a: std.mem.Allocator, raw: std.json.Value) !?RadiusGatewayConfig {
    const base = j.field(raw, "baseUrl") orelse return null;
    const values = j.get(raw, "models") orelse return null;
    if (values != .array) return null;
    var config = j.map(a);
    var models = j.array(a);
    for (values.array.items) |value| if (validModel(value)) {
        try models.array.append(try j.clone(a, value));
    };
    try j.put(a, &config, "baseUrl", j.str(try a.dupe(u8, base)));
    try j.put(a, &config, "models", models);
    return config;
}
pub fn getRadiusCredentialConfig(a: std.mem.Allocator, credential: ?auth.OAuthCredential) !?RadiusGatewayConfig {
    const value = credential orelse return null;
    return sanitizeRadiusGatewayConfig(a, j.get(value.metadata orelse .null, "gatewayConfig") orelse return null);
}
pub fn getRadiusModelsFromConfig(a: std.mem.Allocator, id: []const u8, config: std.json.Value) ![]const t.Model {
    const base = j.get(config, "baseUrl");
    const values = j.get(config, "models") orelse return error.InvalidRadiusConfig;
    if (values != .array) return error.InvalidRadiusConfig;
    var out: std.ArrayList(t.Model) = .empty;
    for (values.array.items) |raw| {
        var value = try j.objectSpread(a, raw);
        try j.put(a, &value, "api", j.str("pi-messages"));
        try j.put(a, &value, "provider", j.str(id));
        if (base) |present| try j.put(a, &value, "baseUrl", present) else _ = value.object.fetchOrderedRemove("baseUrl");
        // The direct source helper maps without validation. Required native
        // text slots need fallbacks, while metadata retains their raw values
        // or omission; all other fields keep the ordinary Model decoder.
        var input = try j.clone(a, value);
        inline for (.{ "id", "name", "baseUrl" }) |name| if (j.field(input, name) == null) try j.put(a, &input, name, j.str(""));
        var model = try std.json.parseFromValueLeaky(t.Model, a, input, .{ .ignore_unknown_fields = true });
        model.jsonFields = try j.clone(a, value);
        inline for (.{ "id", "name", "baseUrl" }) |name| {
            if (j.get(value, name)) |present| {
                if (present != .string) try j.retainField(a, &model, name, present);
            } else j.markAbsent(&model, name);
        }
        try out.append(a, model);
    }
    return out.toOwnedSlice(a);
}
pub fn getRadiusModels(a: std.mem.Allocator, id: []const u8, credential: ?auth.OAuthCredential) ![]const t.Model {
    const config = (try getRadiusCredentialConfig(a, credential)) orelse return &.{};
    return getRadiusModelsFromConfig(a, id, config);
}
pub fn loadRadiusGatewayConfig(a: std.mem.Allocator, io: std.Io, network: @import("../transport/http.zig").Transport, gateway: []const u8, key: ?[]const u8, error_message: ?*?[]const u8) !std.json.Value {
    var endpoint = try std.Uri.parse(gateway);
    endpoint.path = .{ .raw = "/v1/config" };
    endpoint.query = null;
    endpoint.fragment = null;
    var request_headers: std.ArrayList(t.ProviderHeader) = .empty;
    defer request_headers.deinit(a);
    try request_headers.append(a, .{ .name = "accept", .value = "application/json" });
    if (key) |value| if (value.len > 0) try request_headers.append(a, .{ .name = "authorization", .value = try std.fmt.allocPrint(a, "Bearer {s}", .{value}) });
    const response = try @import("../auth/oauth/http.zig").request(a, io, network, .GET, try uri.render(a, endpoint), null, null, request_headers.items);
    if (!response.ok()) {
        const trimmed = try @import("../utils/unicode.zig").trimWhitespace(a, response.raw);
        defer a.free(trimmed);
        const units = try std.unicode.wtf8ToWtf16LeAlloc(a, trimmed);
        defer a.free(units);
        const prefix = try std.unicode.wtf16LeToWtf8Alloc(a, units[0..@min(512, units.len)]);
        defer a.free(prefix);
        if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Could not load Radius config from {s}: {d}: {s}{s}", .{ gateway, response.status, prefix, if (units.len > 512) "…" else "" });
        return error.RadiusConfigRequestFailed;
    }
    if (response.jsonError) |err| return err;
    return (try sanitizeRadiusGatewayConfig(a, response.body)) orelse {
        if (error_message) |slot| slot.* = try std.fmt.allocPrint(a, "Invalid Radius config from {s}", .{gateway});
        return error.InvalidRadiusConfig;
    };
}
const State = struct {
    allocator: std.mem.Allocator,
    arena: std.heap.ArenaAllocator,
    mutex: std.Io.Mutex = .init,
    options: RadiusProviderOptions,
    current: []const t.Model,
    flow: @import("../auth/oauth/radius.zig").RadiusOAuth,
    fn create(a: std.mem.Allocator, options: RadiusProviderOptions) !*State {
        const self = try a.create(State);
        errdefer a.destroy(self);
        var arena = std.heap.ArenaAllocator.init(a);
        errdefer arena.deinit();
        const alloc = arena.allocator();
        const owned = RadiusProviderOptions{ .id = try alloc.dupe(u8, options.id), .name = try alloc.dupe(u8, options.name), .gateway = try normalizeRadiusGatewayUrl(alloc, options.gateway) };
        var baseline: []const t.Model = &.{};
        if (std.mem.eql(u8, owned.gateway, DEFAULT_RADIUS_GATEWAY)) {
            var catalog = try @import("../model_catalog.zig").Catalog.decode(alloc, @embedFile("../../data/radius.json"));
            defer catalog.deinit();
            const values = try alloc.dupe(t.Model, catalog.models);
            for (values) |*model| model.provider = owned.id;
            baseline = try t.cloneInto([]const t.Model, alloc, values);
        }
        self.* = .{ .allocator = a, .arena = arena, .options = owned, .current = baseline, .flow = @import("../auth/oauth/radius.zig").createRadiusOAuth(.{ .name = owned.name, .gateway = owned.gateway }) };
        return self;
    }
    fn clone(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io) !?*anyopaque {
        const original: *State = @ptrCast(@alignCast(ctx.?));
        try original.mutex.lock(io);
        defer original.mutex.unlock(io);
        const result = try create(a, original.options);
        errdefer destroy(result, a);
        result.current = try t.cloneInto([]const t.Model, result.arena.allocator(), original.current);
        return result;
    }
    fn destroy(ctx: ?*anyopaque, a: std.mem.Allocator) void {
        const self: *State = @ptrCast(@alignCast(ctx.?));
        self.arena.deinit();
        a.destroy(self);
    }
    fn models(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io) ![]const t.Model {
        const self: *State = @ptrCast(@alignCast(ctx.?));
        try self.mutex.lock(io);
        defer self.mutex.unlock(io);
        return t.cloneInto([]const t.Model, a, self.current);
    }
    const Update = struct {
        owner: *State,
        io: std.Io,
        models: []const t.Model,
        fn call(ctx: ?*anyopaque) void {
            const self: *@This() = @ptrCast(@alignCast(ctx.?));
            self.owner.mutex.lockUncancelable(self.io);
            defer self.owner.mutex.unlock(self.io);
            self.owner.current = self.models;
        }
    };
    fn publish(self: *State, context: m.RefreshModelsContext, values: []const t.Model, persist: bool) !bool {
        const owned = blk: {
            try self.mutex.lock(context.operation.io);
            defer self.mutex.unlock(context.operation.io);
            break :blk try t.cloneInto([]const t.Model, self.arena.allocator(), values);
        };
        var update = Update{ .owner = self, .io = context.operation.io, .models = owned };
        return context.publish(.{ .models = owned, .updateContext = &update, .update = Update.call, .persist = if (persist) .{ .write = .{ .models = owned, .checkedAt = @as(f64, @floatFromInt(std.Io.Clock.real.now(context.operation.io).toMilliseconds())) } } else .unchanged });
    }
    fn refresh(ctx: ?*anyopaque, context: m.RefreshModelsContext) !void {
        const self: *State = @ptrCast(@alignCast(ctx.?));
        const a = context.operation.allocator;
        if (context.stored) |stored| {
            var restored: std.ArrayList(t.Model) = .empty;
            for (stored.models) |model| if (std.mem.eql(u8, model.provider, self.options.id)) {
                try restored.append(a, model);
            };
            if (!try self.publish(context, restored.items, false)) return;
        }
        if (context.stored == null) if (context.credential) |credential| if (credential == .oauth) {
            const legacy = try getRadiusModels(a, self.options.id, credential.oauth);
            if (legacy.len > 0 and !try self.publish(context, legacy, true)) return;
        };
        if (!context.allowNetwork) return;
        try context.operation.io.checkCancel();
        const key = if (context.credential) |credential| switch (credential) {
            .api_key => |value| value.key,
            .oauth => |value| value.access,
        } else null;
        const config = try loadRadiusGatewayConfig(a, context.operation.io, context.operation.transport, self.options.gateway, key, context.operation.errorMessage);
        const refreshed = try getRadiusModelsFromConfig(a, self.options.id, config);
        try context.operation.io.checkCancel();
        _ = try self.publish(context, refreshed, true);
    }
    fn login(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: @import("../transport/http.zig").Transport, interaction: auth.Interaction, options: auth.LoginOptions) !auth.OAuthCredential {
        const self: *State = @ptrCast(@alignCast(ctx.?));
        const method = self.flow.authMethod();
        return method.login(method.context, a, io, network, interaction, options);
    }
    fn refreshToken(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: @import("../transport/http.zig").Transport, credential: auth.OAuthCredential) !auth.OAuthCredential {
        const self: *State = @ptrCast(@alignCast(ctx.?));
        const method = self.flow.authMethod();
        return method.refresh(method.context, a, io, network, credential);
    }
    fn refreshTokenWithError(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, network: @import("../transport/http.zig").Transport, credential: auth.OAuthCredential, error_message: ?*?[]const u8) !auth.OAuthCredential {
        const self: *State = @ptrCast(@alignCast(ctx.?));
        return self.flow.authMethod().refreshCredential(a, io, network, credential, error_message);
    }
};
/// The returned factory context belongs to a; Models.register clones it.
/// Call provider.deinitContext(provider.context,a) when releasing the factory.
pub fn radiusProvider(a: std.mem.Allocator, options: RadiusProviderOptions) !m.Provider {
    const state = try State.create(a, options);
    errdefer State.destroy(state, a);
    return m.createProvider(.{ .id = state.options.id, .name = state.options.name, .context = state, .cloneContext = State.clone, .deinitContext = State.destroy, .models = state.current, .getModels = State.models, .getAllModels = State.models, .refreshModels = State.refresh, .auth = auth.ProviderAuth{ .apiKey = @import("../auth/helpers.zig").envApiKeyAuth("Radius API key", &.{"RADIUS_API_KEY"}), .oauth = .{ .name = state.options.name, .context = state, .login = State.login, .refresh = State.refreshToken, .refreshWithError = State.refreshTokenWithError, .toAuth = @import("../auth/oauth/http.zig").auth } }, .apis = try a.dupe(m.ApiImplementation, &.{@import("base.zig").implementation("pi-messages")}) });
}
