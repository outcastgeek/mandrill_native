//! Provider model schemas.
const std = @import("std");
const s = @import("../schema/type.zig");
const Type = s.Type;
const j = @import("../json/value.zig");
const h = @import("schema_helpers.zig");
const t = @import("../types.zig");
pub const THINKING_LEVELS: []const []const u8 = &.{ "minimal", "low", "medium", "high", "xhigh", "max" };
pub const MODEL_THINKING_LEVELS: []const []const u8 = &.{ "off", "minimal", "low", "medium", "high", "xhigh", "max" };
pub const MODEL_INPUT_MODALITIES: []const []const u8 = &.{ "text", "image" };
pub const CACHE_RETENTIONS: []const []const u8 = &.{ "none", "short", "long" };
pub const MODEL_PROMPT_CACHE_RETENTIONS: []const []const u8 = &.{ "short", "long" };
pub const ThinkingLevel = t.ThinkingLevel;
pub const ModelThinkingLevel = t.ThinkingLevel;
pub const ThinkingLevelMap = t.ThinkingLevelMap;
pub const ModelInputModality = t.ModelInputModality;
pub const CacheRetention = t.CacheRetention;
pub const ModelPromptCache = t.ModelPromptCache;
pub const ModelCostRates = t.ModelCostRates;
pub const ModelCostTier = t.ModelCostTier;
pub const ModelCost = t.ModelCost;
pub const ModelImageResizeOptions = t.ModelImageResizeOptions;
pub const ModelImageInputLimits = t.ModelImageInputLimits;
pub const ModelInputLimits = t.ModelInputLimits;
pub fn ThinkingLevelSchema(a: std.mem.Allocator) !s.TSchema {
    return h.enumeration(a, THINKING_LEVELS);
}
pub fn ModelThinkingLevelSchema(a: std.mem.Allocator) !s.TSchema {
    return h.enumeration(a, MODEL_THINKING_LEVELS);
}
pub fn ModelInputModalitySchema(a: std.mem.Allocator) !s.TSchema {
    return h.enumeration(a, MODEL_INPUT_MODALITIES);
}
pub fn CacheRetentionSchema(a: std.mem.Allocator) !s.TSchema {
    return h.enumeration(a, CACHE_RETENTIONS);
}
pub fn ThinkingLevelMapSchema(a: std.mem.Allocator) !s.TSchema {
    const item = try Type.Union(a, &.{ try Type.String(a, .{}), try Type.Null(a) });
    var fields: [MODEL_THINKING_LEVELS.len]h.Field = undefined;
    for (MODEL_THINKING_LEVELS, &fields) |name, *field| field.* = .{ .name = name, .schema = item };
    return h.object(a, &fields, .{ .description = "Provider value sent for each thinking level. null marks a level unsupported. Levels the model does not map use the provider's default value through high; xhigh and max are unsupported." });
}
pub fn ModelPromptCacheSchema(a: std.mem.Allocator) !s.TSchema {
    const item = try Type.Number(a, .{ .extra = try j.from(a, .{ .exclusiveMinimum = 0 }) });
    return h.object(a, &.{ .{ .name = "short", .schema = item }, .{ .name = "long", .schema = item } }, .{ .description = "Best-effort prompt cache lifetime in seconds for the short and long retention tiers. Pi warms only tiers with a known lifetime." });
}
fn costFields(a: std.mem.Allocator) ![4]h.Field {
    return .{
        .{ .name = "input", .schema = try Type.Number(a, .{ .description = "Input cost in USD per million tokens." }), .required = true },
        .{ .name = "output", .schema = try Type.Number(a, .{ .description = "Output cost in USD per million tokens." }), .required = true },
        .{ .name = "cacheRead", .schema = try Type.Number(a, .{ .description = "Cache-read cost in USD per million tokens." }), .required = true },
        .{ .name = "cacheWrite", .schema = try Type.Number(a, .{ .description = "Cache-write cost in USD per million tokens." }), .required = true },
    };
}
pub fn ModelCostRatesSchema(a: std.mem.Allocator) !s.TSchema {
    const fields = try costFields(a);
    return h.object(a, &fields, .{});
}
pub fn ModelCostTierSchema(a: std.mem.Allocator) !s.TSchema {
    const rates = try costFields(a);
    var fields: [5]h.Field = undefined;
    fields[0] = .{ .name = "inputTokensAbove", .schema = try Type.Number(a, .{ .description = "Use this tier when total request input exceeds this token count." }), .required = true };
    @memcpy(fields[1..], &rates);
    return h.object(a, &fields, .{});
}
pub fn ModelCostSchema(a: std.mem.Allocator) !s.TSchema {
    var fields: [5]h.Field = undefined;
    const rates = try costFields(a);
    @memcpy(fields[0..4], &rates);
    fields[4] = .{ .name = "tiers", .schema = try Type.Array(a, try ModelCostTierSchema(a), .{ .description = "Request-wide pricing tiers. The highest matching input threshold applies to the full request." }) };
    return h.object(a, &fields, .{});
}
fn resizeOptions(a: std.mem.Allocator, options: s.Options) !s.TSchema {
    const positive = try Type.Integer(a, .{ .extra = try j.from(a, .{ .minimum = 1 }) });
    return h.object(a, &.{
        .{ .name = "maxWidth", .schema = positive },                                                                                                                            .{ .name = "maxHeight", .schema = positive },
        .{ .name = "maxBytes", .schema = try Type.Integer(a, .{ .description = "Maximum base64-encoded payload size in bytes.", .extra = try j.from(a, .{ .minimum = 1 }) }) }, .{ .name = "jpegQuality", .schema = try Type.Integer(a, .{ .extra = try j.from(a, .{ .minimum = 1, .maximum = 100 }) }) },
    }, options);
}
pub fn ModelImageResizeOptionsSchema(a: std.mem.Allocator) !s.TSchema {
    return resizeOptions(a, .{});
}
pub fn ModelImageInputLimitsSchema(a: std.mem.Allocator) !s.TSchema {
    return h.object(a, &.{
        .{ .name = "resize", .schema = try resizeOptions(a, .{ .description = "Cache-safe resize profile applied before a new image enters conversation history." }) },
        .{ .name = "maxPerMessage", .schema = try Type.Integer(a, .{ .description = "Maximum images accepted in one provider message.", .extra = try j.from(a, .{ .minimum = 1 }) }) },
        .{ .name = "maxPerRequest", .schema = try Type.Integer(a, .{ .description = "Maximum images accepted across one provider request.", .extra = try j.from(a, .{ .minimum = 1 }) }) },
    }, .{});
}
pub fn ModelInputLimitsSchema(a: std.mem.Allocator) !s.TSchema {
    return h.object(a, &.{
        .{ .name = "maxRequestBytes", .schema = try Type.Integer(a, .{ .description = "Maximum serialized provider request size in bytes.", .extra = try j.from(a, .{ .minimum = 1 }) }) },
        .{ .name = "images", .schema = try ModelImageInputLimitsSchema(a) },
    }, .{});
}
