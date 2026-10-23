//! Native host equivalent of the pinned Cloudflare AI binding fetch adapter.
//! Transport/context ownership stays with the host. Creating this interface
//! captures fetchFn immediately and performs no I/O, rewrites or body reads.
const std = @import("std");
const http = @import("../transport/http.zig");
pub const CLOUDFLARE_GATEWAY_BINDING_AUTH_SENTINEL = "cloudflare-gateway-binding";
pub const AiBinding = struct {
    context: *anyopaque,
    aiGatewayLogId: ?[]const u8,
    fetchFn: ?*const fn (*anyopaque, std.mem.Allocator, std.Io, http.Request) anyerror!http.Response = null,
    supportsClientCertificates: bool = false,
};
pub fn createAiBindingFetch(binding: AiBinding) !http.Transport {
    return .{ .context = binding.context, .sendFn = binding.fetchFn orelse return error.AiBindingFetchUnavailable, .supportsClientCertificates = binding.supportsClientCertificates };
}
