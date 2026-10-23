const std = @import("std");
const m = @import("../models.zig");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
pub const openRouterCost = @import("openrouter_api.zig").openRouterCost;
pub const OPENROUTER_BASE_URL = "https://openrouter.ai/api/v1";

const CatalogResponse = struct { status: u16, body: ?std.json.Value = null };
fn catalogResponse(context: m.RefreshModelsContext, endpoint: std.Uri, key: []const u8) !CatalogResponse {
    const a = context.operation.allocator;
    var response = try context.operation.transport.send(a, context.operation.io, .{
        .method = .GET,
        .uri = endpoint,
        .headers = &.{ .{ .name = "accept", .value = "application/json" }, .{ .name = "authorization", .value = try std.fmt.allocPrint(a, "Bearer {s}", .{key}) } },
    });
    var consumed = false;
    defer if (consumed) response.deinit() else response.abort();
    if (response.status < 200 or response.status >= 300) return .{ .status = response.status };
    const raw = try response.readAll(a, .unlimited);
    defer a.free(raw);
    consumed = true;
    const text = try std.fmt.allocPrint(a, "{f}", .{std.unicode.fmtUtf8(raw)});
    defer a.free(text);
    var reader = std.Io.Reader.fixed(text);
    if (std.mem.startsWith(u8, text, &.{ 0xef, 0xbb, 0xbf })) reader.toss(3);
    return .{ .status = response.status, .body = try j.parseFromSliceLeaky(std.json.Value, a, reader.buffered(), .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) };
}
fn setCatalogLimit(a: std.mem.Allocator, model: *t.Model, comptime name: []const u8, value: std.json.Value) !void {
    if (j.number(value)) |number| {
        @field(model, name) = number;
        j.markPresent(model, name);
    } else {
        // The source uses || and preserves a truthy nonnumeric wire value.
        // Detach metadata before changing a copy of a borrowed catalog model.
        model.jsonFields = try j.clone(a, model.jsonFields);
        try j.retainField(a, model, name, value);
    }
}

/// Explicit personalization; constructing a provider does not make this request.
/// All returned allocations belong to context.operation.allocator.
pub fn personalizeOpenRouterCatalog(catalog: []const t.Model, context: m.RefreshModelsContext) ![]const t.Model {
    const credential = context.credential orelse return catalog;
    const key = switch (credential) {
        .oauth => |value| value.access,
        .api_key => |value| value.key orelse return catalog,
    };
    if (key.len == 0) return catalog;
    const a = context.operation.allocator;
    const io = context.operation.io;
    const base = std.mem.trimEnd(u8, context.baseUrl orelse OPENROUTER_BASE_URL, "/");
    const endpoint = try @import("../transport/uri.zig").parseHttpUrl(a, try std.fmt.allocPrint(a, "{s}/models/user", .{base}));
    const Outcome = union(enum) { response: anyerror!CatalogResponse, timeout: std.Io.Cancelable!void };
    var buffer: [2]Outcome = undefined;
    var select = std.Io.Select(Outcome).init(io, &buffer);
    defer select.cancelDiscard();
    try select.concurrent(.response, catalogResponse, .{ context, endpoint, key });
    try select.concurrent(.timeout, struct {
        fn sleep(clock_io: std.Io) std.Io.Cancelable!void {
            try clock_io.sleep(.fromMilliseconds(15_000), .awake);
        }
    }.sleep, .{io});
    const response = switch (try select.await()) {
        .response => |result| try result,
        .timeout => |result| {
            try result;
            return error.RequestTimeout;
        },
    };
    if (response.status == 401 or response.status == 403 or response.status == 404) return catalog;
    if (response.status < 200 or response.status >= 300) {
        if (context.operation.errorMessage) |slot| slot.* = try std.fmt.allocPrint(a, "OpenRouter model list request failed: {d}", .{response.status});
        return error.OpenRouterModelListRequestFailed;
    }
    const data = j.get(response.body orelse .null, "data") orelse .null;
    if (data != .array) {
        if (context.operation.errorMessage) |slot| slot.* = "Invalid OpenRouter model list response";
        return error.InvalidOpenRouterModelListResponse;
    }
    var allowed: std.StringHashMapUnmanaged(std.json.Value) = .{};
    defer allowed.deinit(a);
    for (data.array.items) |entry| if (j.field(entry, "id")) |id| {
        try allowed.put(a, id, entry);
    };
    var result: std.ArrayList(t.Model) = .empty;
    for (catalog) |model| {
        if (model.modelType() != .chat) {
            try result.append(a, model);
            continue;
        }
        const entry = allowed.get(model.id) orelse continue;
        var updated = model;
        const top = j.get(entry, "top_provider") orelse .null;
        const top_context = j.get(top, "context_length") orelse .null;
        const context_length = j.get(entry, "context_length") orelse .null;
        const max_tokens = j.get(top, "max_completion_tokens") orelse .null;
        if (j.truthy(top_context)) try setCatalogLimit(a, &updated, "contextWindow", top_context) else if (j.truthy(context_length)) try setCatalogLimit(a, &updated, "contextWindow", context_length);
        if (j.truthy(max_tokens)) try setCatalogLimit(a, &updated, "maxTokens", max_tokens);
        if (j.get(entry, "pricing")) |pricing| if (j.truthy(pricing)) {
            updated.cost = try openRouterCost(a, pricing);
            j.markPresent(&updated, "cost");
        };
        try result.append(a, updated);
    }
    return result.toOwnedSlice(a);
}
pub fn openrouterProvider(a: std.mem.Allocator) !m.Provider {
    var provider = try @import("base.zig").create(a, .{ .id = "openrouter", .name = "OpenRouter", .baseUrl = "https://openrouter.ai/api/v1", .authName = "OpenRouter API key", .env = &.{"OPENROUTER_API_KEY"}, .apis = &.{ "anthropic-messages", "openai-completions", "openrouter-images", "typesafe-system-one" } }, @embedFile("../../data/openrouter.json"));
    provider.auth.oauth = @import("../oauth.zig").openRouterOAuth;
    return provider;
}
