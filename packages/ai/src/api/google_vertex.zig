const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const c = @import("common.zig");
const uri = @import("../transport/uri.zig");
pub const GoogleVertexOptions = struct {
    google: @import("google_shared.zig").GoogleOptions = .{},
    project: ?[]const u8 = null,
    location: ?[]const u8 = null,
    pub fn streamOptions(self: @This(), a: std.mem.Allocator) !t.StreamOptions {
        var options = try self.google.streamOptions(a);
        var extra = if (options.extra) |v| try j.clone(a, v) else j.map(a);
        if (self.project) |project| try j.put(a, &extra, "project", j.str(project));
        if (self.location) |location| try j.put(a, &extra, "location", j.str(location));
        options.extra = extra;
        return options;
    }
};
pub const buildPayload = @import("google_shared.zig").buildSdkPayload;
/// A usable key is allocated with `a`; marker/placeholder values return null.
pub fn resolveApiKey(a: std.mem.Allocator, options: t.StreamOptions) !?[]const u8 {
    const key = try @import("../utils/unicode.zig").trimWhitespace(a, options.apiKey orelse return null);
    if (key.len == 0 or std.mem.eql(u8, key, "gcp-vertex-credentials")) {
        a.free(key);
        return null;
    }
    if (key.len > 2 and key[0] == '<' and key[key.len - 1] == '>' and std.mem.findScalar(u8, key[1 .. key.len - 1], '>') == null) {
        a.free(key);
        return null;
    }
    return key;
}
pub fn hasApiKey(a: std.mem.Allocator, options: t.StreamOptions) !bool {
    const key = try resolveApiKey(a, options);
    defer if (key) |text| a.free(text);
    return key != null;
}
fn nonempty(value: ?[]const u8) ?[]const u8 {
    const text = value orelse return null;
    return if (text.len > 0) text else null;
}
fn projectFor(options: t.StreamOptions) ?[]const u8 {
    return nonempty(if (c.extra(options, "project")) |value| j.text(value) else null) orelse nonempty(options.env.get("GOOGLE_CLOUD_PROJECT")) orelse nonempty(options.env.get("GCLOUD_PROJECT"));
}
fn locationFor(options: t.StreamOptions) ?[]const u8 {
    return nonempty(if (c.extra(options, "location")) |value| j.text(value) else null) orelse nonempty(options.env.get("GOOGLE_CLOUD_LOCATION"));
}
pub fn resources(a: std.mem.Allocator, options: t.StreamOptions) !@import("google_wire.zig").Resources {
    if (try hasApiKey(a, options)) return .{};
    return .{ .project = projectFor(options), .location = locationFor(options) };
}
pub fn validateClient(a: std.mem.Allocator, options: t.StreamOptions) !void {
    if (try hasApiKey(a, options)) return;
    if (projectFor(options) == null) {
        if (options.errorMessage) |slot| slot.* = "Vertex AI requires a project ID. Set GOOGLE_CLOUD_PROJECT/GCLOUD_PROJECT or pass project in options.";
        return error.GoogleProjectRequired;
    }
    if (locationFor(options) == null) {
        if (options.errorMessage) |slot| slot.* = "Vertex AI requires a location. Set GOOGLE_CLOUD_LOCATION or pass location in options.";
        return error.GoogleLocationRequired;
    }
}
fn baseUrlIncludesApiVersion(a: std.mem.Allocator, text: []const u8) !bool {
    const parsed = uri.parseHttpUrl(a, text) catch |err| {
        if (err == error.OutOfMemory or err == error.Canceled) return err;
        return @import("../regex/regexp.zig").matches(a, null, "(?:^|/)v[0-9]+(?:beta[0-9]*)?(?:/|$)", text, .{});
    };
    // URL.pathname retains percent escapes: /v%31/ is not a v1 component.
    const path = switch (parsed.path) {
        .raw, .percent_encoded => |value| value,
    };
    return @import("../regex/regexp.zig").matches(a, null, "(?:^|/)v[0-9]+(?:beta[0-9]*)?(?:/|$)", path, .{});
}
pub fn endpoint(a: std.mem.Allocator, model: t.Model, options: t.StreamOptions, payload: std.json.Value) !std.Uri {
    const shared = @import("google_shared.zig");
    const api_key = try hasApiKey(a, options);
    const project = projectFor(options);
    const location = locationFor(options);
    if (!api_key and (project == null or location == null)) try validateClient(a, options);
    const id = (shared.validateRequestModel(payload) catch |err| {
        if (options.errorMessage) |slot| slot.* = if (err == error.InvalidGoogleModel) "invalid model parameter" else "model is required and must be a string";
        return err;
    }) orelse {
        if (options.errorMessage) |slot| slot.* = "Cannot convert undefined or null to object";
        return error.MissingGoogleUrlModel;
    };
    const configured = try @import("../utils/unicode.zig").trimWhitespace(a, model.baseUrl);
    defer a.free(configured);
    const custom = configured.len > 0 and std.mem.find(u8, configured, "{location}") == null;
    const loc = location orelse "global";
    const default_base = if (custom) configured else if (api_key or std.mem.eql(u8, loc, "global")) "https://aiplatform.googleapis.com/" else if (std.mem.eql(u8, loc, "eu") or std.mem.eql(u8, loc, "us")) try std.fmt.allocPrint(a, "https://aiplatform.{s}.rep.googleapis.com/", .{loc}) else try std.fmt.allocPrint(a, "https://{s}-aiplatform.googleapis.com/", .{loc});
    const default_version = if (custom and try baseUrlIncludesApiVersion(a, configured)) "" else "v1";
    const http_options = shared.httpOptions(payload);
    const raw_base = try shared.patchHttpOption(a, j.str(default_base), http_options, "baseUrl");
    const base = j.text(raw_base) orelse {
        if (options.errorMessage) |slot| slot.* = "httpOptions.baseUrl.endsWith is not a function";
        return error.InvalidGoogleBaseUrl;
    };
    const version = try shared.patchHttpOption(a, j.str(default_version), http_options, "apiVersion");
    const scope = try shared.patchHttpOption(a, if (custom) j.str("COLLECTION") else .null, http_options, "baseUrlResourceScope");
    const collection = std.mem.eql(u8, j.text(scope) orelse "", "COLLECTION");
    const transformed = if (std.mem.startsWith(u8, id, "publishers/") or std.mem.startsWith(u8, id, "projects/") or std.mem.startsWith(u8, id, "models/")) id else if (std.mem.findScalar(u8, id, '/')) |separator| value: {
        var rest = std.mem.splitScalar(u8, id[separator + 1 ..], '/');
        break :value try std.fmt.allocPrint(a, "publishers/{s}/models/{s}", .{ id[0..separator], rest.first() });
    } else try std.fmt.allocPrint(a, "publishers/google/models/{s}", .{id});
    var parts: std.ArrayList([]const u8) = .empty;
    defer parts.deinit(a);
    try parts.append(a, if (std.mem.endsWith(u8, base, "/")) base[0 .. base.len - 1] else base);
    if (j.truthy(version)) try parts.append(a, try @import("../utils/diagnostics.zig").formatThrownValue(a, .{ .value = version }));
    if (!collection and !api_key and !std.mem.startsWith(u8, transformed, "projects/")) try parts.append(a, try std.fmt.allocPrint(a, "projects/{s}/locations/{s}", .{ project.?, location.? }));
    try parts.append(a, try std.fmt.allocPrint(a, "{s}:streamGenerateContent?alt=sse", .{transformed}));
    var result = uri.parseHttpUrl(a, try std.mem.join(a, "/", parts.items)) catch |err| {
        if (options.errorMessage) |slot| slot.* = "Invalid URL";
        return err;
    };
    const original = if (result.query) |value| try uri.decodeFormPairs(a, value) else &.{};
    const first = for (original) |pair| {
        if (std.mem.eql(u8, pair.name, "alt")) break pair.value;
    } else null;
    if (first == null or !std.mem.eql(u8, first.?, "sse")) {
        var query: std.ArrayList(uri.Pair) = .empty;
        defer query.deinit(a);
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

pub fn googleVertexApi() @import("../models.zig").ApiImplementation {
    return @import("driver.zig").implementation("google-vertex");
}
pub fn stream(operation: @import("../models.zig").OperationContext, model: t.Model, context: t.TranscriptContext, options: GoogleVertexOptions) !*@import("../utils/event_stream.zig").AssistantMessageEventStream {
    if (!std.mem.eql(u8, model.api, "google-vertex")) return error.MismatchedApi;
    var arena = std.heap.ArenaAllocator.init(operation.allocator);
    defer arena.deinit();
    return @import("driver.zig").stream(null, operation, model, context, try options.streamOptions(arena.allocator()));
}
pub fn streamSimple(operation: @import("../models.zig").OperationContext, model: t.Model, context: t.TranscriptContext, options: t.SimpleStreamOptions) !*@import("../utils/event_stream.zig").AssistantMessageEventStream {
    if (!std.mem.eql(u8, model.api, "google-vertex")) return error.MismatchedApi;
    return @import("driver.zig").streamSimple(null, operation, model, context, options);
}
