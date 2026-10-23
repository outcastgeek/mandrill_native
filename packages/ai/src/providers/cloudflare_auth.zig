//! Direct-import Cloudflare auth factories.
const auth = @import("../auth/types.zig");
const provider_auth = @import("provider_auth.zig");
pub fn cloudflareWorkersAIAuth() auth.ApiKeyAuth {
    return provider_auth.cloudflare(false);
}
pub fn cloudflareAIGatewayAuth() auth.ApiKeyAuth {
    return provider_auth.cloudflare(true);
}
