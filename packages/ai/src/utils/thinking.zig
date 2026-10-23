const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
pub fn supports(model: t.Model, level: t.ThinkingLevel) bool {
    if (!model.reasoning) return level == .off;
    const mapped = if (model.thinkingLevelMap) |m| j.get(m, @tagName(level)) else null;
    if (mapped) |v| return v != .null;
    return level != .xhigh and level != .max;
}
pub fn getSupportedThinkingLevels(a: std.mem.Allocator, model: t.Model) ![]const t.ThinkingLevel {
    var result: std.ArrayList(t.ThinkingLevel) = .empty;
    inline for (std.meta.tags(t.ThinkingLevel)) |level| if (supports(model, level)) {
        try result.append(a, level);
    };
    return result.toOwnedSlice(a);
}
pub fn clampThinkingLevel(model: t.Model, requested: t.ThinkingLevel) t.ThinkingLevel {
    const levels = std.meta.tags(t.ThinkingLevel);
    const requested_index: usize = @backingInt(requested);
    for (levels[requested_index..]) |level| if (supports(model, level)) return level;
    var index = requested_index;
    while (index > 0) {
        index -= 1;
        const level = levels[index];
        if (supports(model, level)) return level;
    }
    inline for (std.meta.tags(t.ThinkingLevel)) |level| if (supports(model, level)) return level;
    return .off;
}
pub fn providerValue(model: t.Model, level: t.ThinkingLevel) ?[]const u8 {
    if (model.thinkingLevelMap) |m| if (j.get(m, @tagName(level))) |v| return j.text(v);
    return if (supports(model, level)) @tagName(level) else null;
}
