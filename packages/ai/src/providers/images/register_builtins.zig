//! Native direct-import facade. Registration uses an explicit registry owner.
pub const generateImagesOpenRouter = @import("../../api/openrouter_images.zig").generateImages;
pub const registerBuiltInImagesApiProviders = @import("../../images.zig").Registry.registerBuiltInImagesApiProviders;
