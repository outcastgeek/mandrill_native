//! Static image catalog compatibility reads from the pinned provider snapshot.
const all = @import("providers/all.zig");
pub const BuiltinImageProvider = enum { openrouter };
pub const getImageModel = all.getBuiltinImageModel;
pub const getImageModels = all.getBuiltinImageModels;
pub fn getImageProviders() []const []const u8 {
    return &.{"openrouter"};
}
