const std = @import("std");
pub const OpenRouterPricing = std.json.Value;
pub const OpenRouterPricingOverride = std.json.Value;
pub const OpenRouterModel = std.json.Value;
const t = @import("../types.zig");
const j = @import("../json/value.zig");

fn perMillion(a: std.mem.Allocator, value: ?std.json.Value, fallback: f64) !f64 {
    const supplied = value orelse return fallback;
    if (supplied == .null) return fallback;
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    const text = try @import("../utils/diagnostics.zig").formatThrownValue(scratch, .{ .value = supplied });
    const rate = (try @import("../utils/numbers.zig").parseFloat(scratch, null, text)) orelse return fallback;
    if (!(rate >= 0)) return fallback;
    const million = rate * 1_000_000;
    if (!std.math.isFinite(million) or million >= 1e21) return million;
    // Number(toFixed(6)) rounds the original binary64 value, with positive
    // ties upward. f128 holds its exact product with 10^6 (at most 90 bits
    // in this range), avoiding a binary64 multiplication's premature rounding.
    const rounded = @floor(@as(f128, million) * 1_000_000 + 0.5);
    return @floatCast(rounded / 1_000_000);
}
/// A decoded OpenRouter pricing object. Time-dependent overrides cannot be
/// represented by ModelCost and are omitted, as in the pinned implementation.
pub fn openRouterCost(a: std.mem.Allocator, pricing: std.json.Value) !t.ModelCost {
    var result = t.ModelCost{
        .input = try perMillion(a, j.get(pricing, "prompt"), 0),
        .output = try perMillion(a, j.get(pricing, "completion"), 0),
        .cacheRead = try perMillion(a, j.get(pricing, "input_cache_read"), 0),
        .cacheWrite = try perMillion(a, j.get(pricing, "input_cache_write"), 0),
    };
    var tiers: std.ArrayList(t.ModelCostTier) = .empty;
    if (j.getNonNull(pricing, "overrides")) |overrides| {
        if (overrides != .array) return error.InvalidOpenRouterPricingOverrides;
        for (overrides.array.items) |override| {
            if (override == .null) return error.InvalidOpenRouterPricingOverride;
            if (j.get(override, "utc_start") != null or j.get(override, "utc_end") != null or j.get(override, "utc_days") != null) continue;
            const above = j.get(override, "min_prompt_tokens") orelse continue;
            var tier = t.ModelCostTier{
                .inputTokensAbove = j.number(above) orelse 0,
                .input = try perMillion(a, j.get(override, "prompt"), result.input),
                .output = try perMillion(a, j.get(override, "completion"), result.output),
                .cacheRead = try perMillion(a, j.get(override, "input_cache_read"), result.cacheRead),
                .cacheWrite = try perMillion(a, j.get(override, "input_cache_write"), result.cacheWrite),
            };
            if (j.number(above) == null) try j.retainField(a, &tier, "inputTokensAbove", above);
            try tiers.append(a, tier);
        }
    }
    if (tiers.items.len > 0) result.tiers = try tiers.toOwnedSlice(a) else tiers.deinit(a);
    return result;
}
