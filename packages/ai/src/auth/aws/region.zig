//! Shared Smithy region spelling and host-label validation.
const std = @import("std");
const m = @import("../../models.zig");
pub fn isFipsRegion(value: []const u8) bool {
    return std.mem.startsWith(u8, value, "fips-") or std.mem.endsWith(u8, value, "-fips");
}
pub fn realRegion(operation: m.OperationContext, input: []const u8) ![]const u8 {
    var value = input;
    if (isFipsRegion(input)) {
        if (std.mem.eql(u8, input, "fips-aws-global") or std.mem.eql(u8, input, "aws-fips")) value = "us-east-1" else {
            var pattern = try @import("../../regex/regexp.zig").Regex.init(operation.allocator, operation.io, "fips-(?:dkr-|prod-)?|-fips", .{});
            defer pattern.deinit();
            // The replacement pattern is ASCII-only. Byte captures preserve
            // arbitrary UTF8/WTF8 surroundings without changing code units.
            if (try pattern.execBytes(operation.allocator, input, 0)) |found| {
                var match = found;
                defer match.deinit();
                const span = match.captures[0].?;
                value = try std.mem.concat(operation.allocator, u8, &.{ input[0..span.start], input[span.end..] });
            }
        }
    }
    try checkRegion(operation, value, true);
    return value;
}
pub fn checkRegion(operation: m.OperationContext, value: []const u8, allow_wildcard: bool) !void {
    if (!(allow_wildcard and std.mem.eql(u8, value, "*")) and !(try @import("../../regex/regexp.zig").matches(operation.allocator, operation.io, "^(?!.*-$)(?!-)[a-zA-Z0-9-]{1,63}$", value, .{}))) {
        if (operation.errorMessage) |slot| slot.* = try std.fmt.allocPrint(operation.allocator, "Region not accepted: region=\"{s}\" is not a valid hostname component.", .{value});
        return error.InvalidAwsRegion;
    }
}
