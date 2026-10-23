const std = @import("std");
const t = @import("../types.zig");
const m = @import("../models.zig");
const catalog = @import("../model_catalog.zig");
pub const Definition = struct { id: []const u8, name: []const u8, baseUrl: ?[]const u8 = null, authName: []const u8, env: []const []const u8, apis: []const []const u8 };
pub fn implementation(comptime api: []const u8) m.ApiImplementation {
    if (comptime std.mem.eql(u8, api, "openrouter-images")) return @import("../api/openrouter_images.zig").implementation();
    if (comptime std.mem.eql(u8, api, "typesafe-system-one") or std.mem.eql(u8, api, "cloudflare-workers-ai-system-one") or std.mem.eql(u8, api, "openai-decisions") or std.mem.eql(u8, api, "llama-cpp-classify")) return @import("../api/classifiers.zig").implementation(api);
    return @import("../api/driver.zig").implementation(api);
}
pub fn create(a: std.mem.Allocator, comptime definition: Definition, data: []const u8) !m.Provider {
    var loaded = try catalog.Catalog.decode(a, data);
    defer loaded.deinit();
    const apis = try a.alloc(m.ApiImplementation, definition.apis.len);
    inline for (definition.apis, 0..) |api, index| {
        const cloudflare = comptime std.mem.eql(u8, definition.id, "cloudflare-ai-gateway") or std.mem.eql(u8, definition.id, "cloudflare-workers-ai");
        const original = implementation(api);
        apis[index] = if (cloudflare)
            (if (original.classify != null) try @import("cloudflare_stream.zig").cloudflareClassifier(a, original) else try @import("cloudflare_stream.zig").cloudflareStreams(a, original))
        else
            original;
    }
    return m.createProvider(.{ .id = definition.id, .name = definition.name, .baseUrl = definition.baseUrl, .auth = @import("../auth/types.zig").ProviderAuth{ .apiKey = @import("../auth/helpers.zig").envApiKeyAuth(definition.authName, definition.env) }, .models = try t.cloneInto([]const t.Model, a, loaded.models), .apis = apis });
}
