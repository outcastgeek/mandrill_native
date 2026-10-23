const std = @import("std");
const m = @import("../models.zig");
pub fn amazonBedrockProvider(a: std.mem.Allocator) !m.Provider {
    var provider = try @import("base.zig").create(a, .{ .id = "amazon-bedrock", .name = "Amazon Bedrock", .baseUrl = null, .authName = "", .env = &.{}, .apis = &.{"bedrock-converse-stream"} }, @embedFile("../../data/amazon-bedrock.json"));
    provider.auth.apiKey = @import("provider_auth.zig").bedrock();
    return provider;
}
