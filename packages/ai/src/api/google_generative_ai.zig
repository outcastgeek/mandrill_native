const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const uri = @import("../transport/uri.zig");
pub const GoogleOptions = @import("google_shared.zig").GoogleOptions;
pub const buildPayload = @import("google_shared.zig").buildSdkPayload;
pub fn endpoint(a: std.mem.Allocator, model: t.Model, options: t.StreamOptions, payload: std.json.Value) !std.Uri {
    const id = (@import("google_shared.zig").validateRequestModel(payload) catch |err| {
        if (options.errorMessage) |slot| slot.* = if (err == error.InvalidGoogleModel) "invalid model parameter" else "model is required and must be a string";
        return err;
    }) orelse {
        if (options.errorMessage) |slot| slot.* = "Cannot convert undefined or null to object";
        return error.MissingGoogleUrlModel;
    };
    const shared = @import("google_shared.zig");
    const http_options = shared.httpOptions(payload);
    const configured = model.baseUrl.len > 0;
    const raw_base = try shared.patchHttpOption(a, j.str(if (configured) model.baseUrl else "https://generativelanguage.googleapis.com/"), http_options, "baseUrl");
    const base = j.text(raw_base) orelse {
        if (options.errorMessage) |slot| slot.* = "httpOptions.baseUrl.endsWith is not a function";
        return error.InvalidGoogleBaseUrl;
    };
    const raw_version = try shared.patchHttpOption(a, j.str(if (configured) "" else "v1beta"), http_options, "apiVersion");
    var parts: std.ArrayList([]const u8) = .empty;
    defer parts.deinit(a);
    // These are the SDK's whole-string operations, not URI component escaping.
    // The standard URI parser interprets the assembled URL exactly once.
    try parts.append(a, if (std.mem.endsWith(u8, base, "/")) base[0 .. base.len - 1] else base);
    if (j.truthy(raw_version)) try parts.append(a, try @import("../utils/diagnostics.zig").formatThrownValue(a, .{ .value = raw_version }));
    const transformed = if (std.mem.startsWith(u8, id, "models/") or std.mem.startsWith(u8, id, "tunedModels/")) id else try std.fmt.allocPrint(a, "models/{s}", .{id});
    try parts.append(a, try std.fmt.allocPrint(a, "{s}:streamGenerateContent?alt=sse", .{transformed}));
    var result = uri.parseHttpUrl(a, try std.mem.join(a, "/", parts.items)) catch |err| {
        if (options.errorMessage) |slot| slot.* = "Invalid URL";
        return err;
    };
    var query: std.ArrayList(uri.Pair) = .empty;
    defer query.deinit(a);
    const original = if (result.query) |value| try uri.decodeFormPairs(a, value) else &.{};
    const first = for (original) |pair| {
        if (std.mem.eql(u8, pair.name, "alt")) break pair.value;
    } else null;
    if (first == null or !std.mem.eql(u8, first.?, "sse")) {
        var assigned = false;
        for (original) |pair| {
            if (std.mem.eql(u8, pair.name, "alt")) {
                if (!assigned) {
                    try query.append(a, .{ .name = "alt", .value = "sse" });
                    assigned = true;
                }
            } else try query.append(a, pair);
        }
        if (!assigned) try query.append(a, .{ .name = "alt", .value = "sse" });
        result.query = .{ .percent_encoded = try uri.encodeForm(a, query.items) };
    }
    return result;
}

pub fn googleGenerativeAIApi() @import("../models.zig").ApiImplementation {
    return @import("driver.zig").implementation("google-generative-ai");
}
pub fn stream(operation: @import("../models.zig").OperationContext, model: t.Model, context: t.TranscriptContext, options: GoogleOptions) !*@import("../utils/event_stream.zig").AssistantMessageEventStream {
    if (!std.mem.eql(u8, model.api, "google-generative-ai")) return error.MismatchedApi;
    var arena = std.heap.ArenaAllocator.init(operation.allocator);
    defer arena.deinit();
    return @import("driver.zig").stream(null, operation, model, context, try options.streamOptions(arena.allocator()));
}
pub fn streamSimple(operation: @import("../models.zig").OperationContext, model: t.Model, context: t.TranscriptContext, options: t.SimpleStreamOptions) !*@import("../utils/event_stream.zig").AssistantMessageEventStream {
    if (!std.mem.eql(u8, model.api, "google-generative-ai")) return error.MismatchedApi;
    return @import("driver.zig").streamSimple(null, operation, model, context, options);
}
