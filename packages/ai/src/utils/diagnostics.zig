const std = @import("std");
const t = @import("../types.zig");
const j = @import("../json/value.zig");
pub const DiagnosticErrorInfo = struct {
    name: ?[]const u8 = null,
    message: []const u8,
    stack: ?[]const u8 = null,
    code: ?std.json.Value = null,
    jsonFields: std.json.Value = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
pub const AssistantMessageDiagnostic = struct {
    type: []const u8,
    timestamp: f64,
    @"error": ?DiagnosticErrorInfo = null,
    details: ?std.json.Value = null,
    jsonFields: std.json.Value = .null,
    jsonAssigned: u64 = 0,
    jsonOmitted: u64 = 0,
    jsonRetained: u64 = 0,
    pub const jsonToValue = j.Record(@This()).jsonToValue;
    pub const jsonStringify = j.Record(@This()).jsonStringify;
    pub const jsonParse = j.Record(@This()).jsonParse;
    pub const jsonParseFromValue = j.Record(@This()).jsonParseFromValue;
};
/// A host exception retains the fields the pinned helper reads from Error.
/// Borrowed inputs remain alive until the diagnostic is serialized or copied.
pub const Exception = struct { name: []const u8 = "Error", message: []const u8, stack: ?[]const u8 = null, code: ?std.json.Value = null };
pub const ThrownValue = union(enum) { native: anyerror, exception: Exception, text: []const u8, value: std.json.Value, undefined_value };
fn writeValueText(a: std.mem.Allocator, writer: *std.Io.Writer, value: std.json.Value) anyerror!void {
    switch (value) {
        .null => try writer.writeAll("null"),
        .bool => |boolean| try writer.writeAll(if (boolean) "true" else "false"),
        .string => |text| try writer.writeAll(text),
        .integer, .float, .number_string => try writer.writeAll(try @import("numbers.zig").toString(a, j.number(value) orelse return error.InvalidJsonNumber)),
        .object => |object| {
            // An own JSON member shadows Object.prototype.toString without
            // being callable; ordinary String(object) then cannot convert it.
            if (object.contains("toString")) return error.CannotConvertObjectToPrimitive;
            try writer.writeAll("[object Object]");
        },
        .array => |array| for (array.items, 0..) |item, index| {
            if (index > 0) try writer.writeByte(',');
            // Array.toString joins null entries as empty strings, recursively
            // applying ordinary scalar conversion to the remaining elements.
            if (item != .null) try writeValueText(a, writer, item);
        },
    }
}
/// Text formatting follows String(value), not JSON.stringify(value). Returned
/// text may borrow the supplied exception/string; allocated text uses a.
pub fn formatThrownValue(a: std.mem.Allocator, value: ThrownValue) ![]const u8 {
    return switch (value) {
        .native => |err| @errorName(err),
        .exception => |err| if (err.message.len > 0) err.message else err.name,
        .text => |text| text,
        .undefined_value => "undefined",
        .value => |json| text: {
            var temporary = std.heap.ArenaAllocator.init(a);
            defer temporary.deinit();
            var output = std.Io.Writer.Allocating.init(a);
            defer output.deinit();
            writeValueText(temporary.allocator(), &output.writer, json) catch |err| switch (err) {
                error.WriteFailed => return error.OutOfMemory,
                else => return err,
            };
            break :text try output.toOwnedSlice();
        },
    };
}
pub fn extractDiagnosticError(a: std.mem.Allocator, value: ThrownValue) !DiagnosticErrorInfo {
    return switch (value) {
        .native => |err| .{ .name = @errorName(err), .message = @errorName(err) },
        .exception => |err| .{
            .name = if (err.name.len > 0) err.name else null,
            .message = if (err.message.len > 0) err.message else err.name,
            .stack = err.stack,
            .code = if (err.code) |code| switch (code) {
                .string, .integer, .float, .number_string => code,
                else => null,
            } else null,
        },
        else => .{ .name = "ThrownValue", .message = try formatThrownValue(a, value) },
    };
}
pub fn createAssistantMessageDiagnostic(a: std.mem.Allocator, io: std.Io, kind: []const u8, value: ThrownValue, details: ?std.json.Value) !AssistantMessageDiagnostic {
    return .{ .type = kind, .timestamp = @as(f64, @floatFromInt(std.Io.Clock.real.now(io).toMilliseconds())), .@"error" = try extractDiagnosticError(a, value), .details = details };
}
pub fn appendAssistantMessageDiagnostic(a: std.mem.Allocator, message: *t.AssistantMessage, diagnostic: AssistantMessageDiagnostic) !void {
    var values: std.ArrayList(std.json.Value) = .empty;
    try values.appendSlice(a, message.diagnostics orelse &.{});
    try values.append(a, try @import("../json/value.zig").from(a, diagnostic));
    message.diagnostics = try values.toOwnedSlice(a);
}
