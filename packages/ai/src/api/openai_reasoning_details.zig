//! Standard-JSON shape checks for provider reasoning envelopes. Signatures and
//! encrypted data remain opaque strings; these checks do not authenticate them.
const std = @import("std");
const j = @import("../json/value.zig");
const Value = std.json.Value;
pub fn parseOpenAIReasoningDetails(a: std.mem.Allocator, signature: ?[]const u8) !?Value {
    const input = signature orelse return null;
    if (input.len == 0) return null;
    const value = @import("../json/value.zig").parseFromSliceLeaky(Value, a, input, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |err| {
        if (err == error.OutOfMemory) return err;
        return null;
    };
    if (value != .array or value.array.items.len == 0) return null;
    for (value.array.items) |detail| if (!isOpenAIReasoningDetail(detail)) return null;
    return value;
}
pub fn parseLegacyEncryptedReasoningDetail(a: std.mem.Allocator, signature: ?[]const u8) !?Value {
    const input = signature orelse return null;
    if (input.len == 0) return null;
    const value = @import("../json/value.zig").parseFromSliceLeaky(Value, a, input, .{ .duplicate_field_behavior = .use_last, .allocate = .alloc_always }) catch |err| {
        if (err == error.OutOfMemory) return err;
        return null;
    };
    if (!isOpenAIReasoningDetail(value) or !std.mem.eql(u8, j.field(value, "type") orelse @as([]const u8, ""), "reasoning.encrypted") or (j.field(value, "id") orelse @as([]const u8, "")).len == 0 or (j.field(value, "data") orelse @as([]const u8, "")).len == 0) return null;
    return value;
}
pub fn isOpenAIReasoningDetail(value: Value) bool {
    if (value != .object) return false;
    if (j.get(value, "id")) |id| if (id != .null and id != .string) return false;
    if (j.get(value, "format")) |format| if (format != .string) return false;
    if (j.get(value, "index")) |index| if (j.number(index) == null) return false;
    const kind = j.field(value, "type") orelse return false;
    if (std.mem.eql(u8, kind, "reasoning.summary")) return j.field(value, "summary") != null;
    if (std.mem.eql(u8, kind, "reasoning.encrypted")) return j.field(value, "data") != null;
    if (std.mem.eql(u8, kind, "reasoning.text")) {
        if (j.field(value, "text") == null) return false;
        if (j.get(value, "signature")) |signature| if (signature != .null and signature != .string) return false;
        return true;
    }
    return false;
}
pub fn appendOpenAIReasoningDetail(a: std.mem.Allocator, details: *std.ArrayList(Value), detail: Value) !void {
    const kind = j.field(detail, "type") orelse return;
    if (details.items.len > 0) {
        const last = &details.items[details.items.len - 1];
        if (std.mem.eql(u8, kind, j.field(last.*, "type") orelse @as([]const u8, "")) and (std.mem.eql(u8, kind, "reasoning.text") or std.mem.eql(u8, kind, "reasoning.summary"))) {
            const field = if (std.mem.eql(u8, kind, "reasoning.text")) "text" else "summary";
            try j.put(a, last, field, j.str(try std.mem.concat(a, u8, &.{ j.field(last.*, field).?, j.field(detail, field).? })));
            if (std.mem.eql(u8, kind, "reasoning.text")) {
                if ((j.field(last.*, "signature") orelse @as([]const u8, "")).len == 0) if (j.get(detail, "signature")) |signature| try j.put(a, last, "signature", try j.clone(a, signature));
            }
            for ([_][]const u8{ "id", "format", "index" }) |key| {
                const previous = j.get(last.*, key);
                const missing = previous == null or previous.? == .null or (std.mem.eql(u8, key, "format") and previous.? == .string and previous.?.string.len == 0);
                if (missing) if (j.get(detail, key)) |value| try j.put(a, last, key, try j.clone(a, value));
            }
            return;
        }
    }
    try details.append(a, try j.clone(a, detail));
}
