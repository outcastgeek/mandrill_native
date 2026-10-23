const std = @import("std");
pub const OPENAI_PROMPT_CACHE_KEY_MAX_LENGTH = 64;
/// Array.from(string) counts Unicode code points, not bytes or UTF-16 units.
/// The returned slice continues to borrow the caller's key.
pub fn clampOpenAIPromptCacheKey(key: ?[]const u8) !?[]const u8 {
    const input = key orelse return null;
    var iterator = (try std.unicode.Wtf8View.init(input)).iterator();
    var count: usize = 0;
    while (count < OPENAI_PROMPT_CACHE_KEY_MAX_LENGTH) : (count += 1) {
        _ = iterator.nextCodepointSlice() orelse return input;
    }
    return input[0..iterator.i];
}
