const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
const numbers = @import("numbers.zig");
const Value = std.json.Value;
const Cost = @FieldType(t.Usage, "cost");
fn normal(model: t.Model, usage: t.Usage) bool {
    if (model.jsonRetained != 0 or usage.jsonRetained != 0 or model.cost.jsonRetained != 0) return false;
    if (!j.fieldPresent(model, "cost") or !j.fieldPresent(usage, "cost")) return false;
    inline for (.{ "input", "output", "cacheRead", "cacheWrite" }) |name| {
        if (!j.fieldPresent(model.cost, name) or !j.fieldPresent(usage, name)) return false;
    }
    for (model.cost.tiers orelse &.{}) |tier| {
        if (tier.jsonRetained != 0) return false;
        inline for (.{ "inputTokensAbove", "input", "output", "cacheRead", "cacheWrite" }) |name| if (!j.fieldPresent(tier, name)) return false;
    }
    return true;
}
fn assigned(usage: *t.Usage) void {
    j.markPresent(usage, "cost");
    inline for (.{ "input", "output", "cacheRead", "cacheWrite", "total" }) |name| j.markPresent(&usage.cost, name);
}
fn numeric(model: t.Model, usage: *t.Usage) Cost {
    const tokens = usage.input + usage.cacheRead + usage.cacheWrite;
    var rates = t.ModelCostRates{ .input = model.cost.input, .output = model.cost.output, .cacheRead = model.cost.cacheRead, .cacheWrite = model.cost.cacheWrite };
    var threshold: f64 = -1;
    for (model.cost.tiers orelse &.{}) |tier| if (tokens > tier.inputTokensAbove and tier.inputTokensAbove > threshold) {
        threshold = tier.inputTokensAbove;
        rates = .{ .input = tier.input, .output = tier.output, .cacheRead = tier.cacheRead, .cacheWrite = tier.cacheWrite };
    };
    const long = usage.cacheWrite1h orelse 0;
    usage.cost.input = rates.input / 1e6 * usage.input;
    usage.cost.output = rates.output / 1e6 * usage.output;
    usage.cost.cacheRead = rates.cacheRead / 1e6 * usage.cacheRead;
    usage.cost.cacheWrite = (rates.cacheWrite * (usage.cacheWrite - long) + rates.input * 2 * long) / 1e6;
    usage.cost.total = usage.cost.input + usage.cost.output + usage.cost.cacheRead + usage.cost.cacheWrite;
    assigned(usage);
    return usage.cost;
}
fn primitive(a: std.mem.Allocator, value: ?Value) !?Value {
    const raw = value orelse return null;
    return switch (raw) {
        .array, .object => j.str(try @import("diagnostics.zig").formatThrownValue(a, .{ .value = raw })),
        else => raw,
    };
}
fn text(a: std.mem.Allocator, value: ?Value) ![]const u8 {
    return @import("diagnostics.zig").formatThrownValue(a, if (value) |raw| .{ .value = raw } else .undefined_value);
}
pub fn addValues(a: std.mem.Allocator, left: ?Value, right: ?Value) !Value {
    const l = try primitive(a, left);
    const r = try primitive(a, right);
    if ((if (l) |v| v == .string else false) or (if (r) |v| v == .string else false)) return j.str(try std.mem.concat(a, u8, &.{ try text(a, l), try text(a, r) }));
    return j.num(try numbers.coerceValue(a, l) + try numbers.coerceValue(a, r));
}
fn greater(a: std.mem.Allocator, left: ?Value, right: ?Value) !bool {
    const l = try primitive(a, left);
    const r = try primitive(a, right);
    if (l != null and r != null and l.? == .string and r.? == .string) {
        const lu = try std.unicode.wtf8ToWtf16LeAlloc(a, l.?.string);
        defer a.free(lu);
        const ru = try std.unicode.wtf8ToWtf16LeAlloc(a, r.?.string);
        defer a.free(ru);
        for (lu) |*unit| unit.* = std.mem.littleToNative(u16, unit.*);
        for (ru) |*unit| unit.* = std.mem.littleToNative(u16, unit.*);
        return std.mem.order(u16, lu, ru) == .gt;
    }
    return try numbers.coerceValue(a, l) > try numbers.coerceValue(a, r);
}
/// Numeric inputs require no allocation. Retained JSON values use the supplied
/// allocator for source coercion/comparison; scratch data does not escape.
pub fn calculateCost(a: std.mem.Allocator, model: t.Model, usage: *t.Usage) !Cost {
    if (normal(model, usage.*)) return numeric(model, usage);
    var temporary = std.heap.ArenaAllocator.init(a);
    defer temporary.deinit();
    const scratch = temporary.allocator();
    const raw_cost = (try j.fieldValue(scratch, model, "cost")) orelse return error.InvalidModelCost;
    if (raw_cost == .null) return error.InvalidModelCost;
    const original_cost = (try j.fieldValue(scratch, usage.*, "cost")) orelse return error.InvalidUsageCost;
    if (original_cost != .object) return error.InvalidUsageCost;
    const input = try j.fieldValue(scratch, usage.*, "input");
    const output = try j.fieldValue(scratch, usage.*, "output");
    const read = try j.fieldValue(scratch, usage.*, "cacheRead");
    const write = try j.fieldValue(scratch, usage.*, "cacheWrite");
    const input_tokens = try addValues(scratch, try addValues(scratch, input, read), write);
    var rates = raw_cost;
    var matched: ?Value = j.num(-1);
    if (j.getNonNull(raw_cost, "tiers")) |tiers| {
        // A string is iterable in the source, but its character entries have
        // no inputTokensAbove property and cannot select a tier.
        if (tiers != .array and tiers != .string) return error.InvalidModelCostTiers;
        for (if (tiers == .array) tiers.array.items else &.{}) |tier| {
            if (tier == .null) return error.InvalidModelCostTier;
            const threshold = j.get(tier, "inputTokensAbove");
            if (try greater(scratch, input_tokens, threshold) and try greater(scratch, threshold, matched)) {
                rates = tier;
                matched = threshold;
            }
        }
    }
    const raw_long = try j.fieldValue(scratch, usage.*, "cacheWrite1h");
    const long = try numbers.coerceValue(scratch, if (raw_long) |v| (if (v == .null) j.num(0) else v) else j.num(0));
    const short = try numbers.coerceValue(scratch, write) - long;
    const input_rate = try numbers.coerceValue(scratch, j.get(rates, "input"));
    usage.cost.input = input_rate / 1e6 * try numbers.coerceValue(scratch, input);
    j.markPresent(usage, "cost");
    j.markPresent(&usage.cost, "input");
    const output_rate = try numbers.coerceValue(scratch, j.get(rates, "output"));
    usage.cost.output = output_rate / 1e6 * try numbers.coerceValue(scratch, output);
    j.markPresent(&usage.cost, "output");
    const read_rate = try numbers.coerceValue(scratch, j.get(rates, "cacheRead"));
    usage.cost.cacheRead = read_rate / 1e6 * try numbers.coerceValue(scratch, read);
    j.markPresent(&usage.cost, "cacheRead");
    const write_rate = try numbers.coerceValue(scratch, j.get(rates, "cacheWrite"));
    usage.cost.cacheWrite = (write_rate * short + input_rate * 2 * long) / 1e6;
    j.markPresent(&usage.cost, "cacheWrite");
    usage.cost.total = usage.cost.input + usage.cost.output + usage.cost.cacheRead + usage.cost.cacheWrite;
    j.markPresent(&usage.cost, "total");
    return usage.cost;
}
