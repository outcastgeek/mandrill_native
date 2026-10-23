const std = @import("std");
const m = @import("../models.zig");
pub fn googleVertexProvider(a: std.mem.Allocator) !m.Provider {
    var provider = try @import("base.zig").create(a, .{ .id = "google-vertex", .name = "Google Vertex AI", .baseUrl = null, .authName = "", .env = &.{}, .apis = &.{"google-vertex"} }, @embedFile("../../data/google-vertex.json"));
    provider.auth.apiKey = @import("provider_auth.zig").vertex();
    return provider;
}
