const std = @import("std");
const diagnostics = @import("diagnostics.zig");
pub const ModelsErrorCode = enum { model_source, model_validation, provider, stream, auth, oauth };
/// Native error detail, retained alongside a Zig error union by an application.
pub const ModelsError = struct {
    code: ModelsErrorCode,
    message: []const u8,
    cause: ?anyerror = null,
    causeValue: ?diagnostics.ThrownValue = null,
    pub fn init(a: std.mem.Allocator, code: ModelsErrorCode, message: []const u8, cause: ?anyerror) !@This() {
        return initWithCause(a, code, message, if (cause) |err| .{ .native = err } else null);
    }
    /// Preserve host exception details or thrown JSON values as well as the
    /// native error code. All retained cause data uses the caller's allocator.
    pub fn initWithCause(a: std.mem.Allocator, code: ModelsErrorCode, message: []const u8, cause: ?diagnostics.ThrownValue) !@This() {
        var temporary = std.heap.ArenaAllocator.init(a);
        defer temporary.deinit();
        const scratch = temporary.allocator();
        const detail = if (cause) |value| text: {
            if (value == .undefined_value or (value == .value and value.value == .null)) break :text "";
            break :text try @import("unicode.zig").trimWhitespace(scratch, try diagnostics.formatThrownValue(scratch, value));
        } else "";
        return .{
            .code = code,
            .message = if (detail.len == 0 or std.mem.find(u8, message, detail) != null) try a.dupe(u8, message) else try std.fmt.allocPrint(a, "{s}: {s}", .{ message, detail }),
            .cause = if (cause) |value| (if (value == .native) value.native else null) else null,
            .causeValue = try @import("../types.zig").cloneInto(?diagnostics.ThrownValue, a, cause),
        };
    }
};
