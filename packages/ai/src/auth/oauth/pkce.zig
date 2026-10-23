const std = @import("std");
pub const PKCE = struct { verifier: [43]u8, challenge: [43]u8 };
pub fn generatePKCE(io: std.Io) !PKCE {
    var random: [32]u8 = undefined;
    try io.randomSecure(&random);
    var result: PKCE = undefined;
    _ = std.base64.url_safe_no_pad.Encoder.encode(&result.verifier, &random);
    var digest: [32]u8 = undefined;
    std.crypto.hash.sha2.Sha256.hash(&result.verifier, &digest, .{});
    _ = std.base64.url_safe_no_pad.Encoder.encode(&result.challenge, &digest);
    return result;
}
