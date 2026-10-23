const api = @import("api/bedrock_converse_stream.zig");
pub const bedrockProviderModule = struct {
    pub const stream = api.stream;
    pub const streamSimple = api.streamSimple;
};
