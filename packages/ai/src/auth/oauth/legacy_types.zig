const std = @import("std");
const auth = @import("../types.zig");
const j = @import("../../json/value.zig");
pub const OAuthCredentials = auth.OAuthCredential;
pub const OAuthPrompt = struct { message: []const u8, placeholder: ?[]const u8 = null, allowEmpty: ?bool = null };
pub const OAuthAuthInfo = struct { url: []const u8, instructions: ?[]const u8 = null };
pub const OAuthDeviceCodeInfo = struct { userCode: []const u8, verificationUri: []const u8, intervalSeconds: ?f64 = null, expiresInSeconds: ?f64 = null };
pub const OAuthSelectOption = struct { id: []const u8, label: []const u8 };
pub const OAuthSelectPrompt = struct { message: []const u8, options: []const OAuthSelectOption };
pub const OAuthLoginCallbacks = struct {
    context: ?*anyopaque,
    onAuth: *const fn (?*anyopaque, std.Io, OAuthAuthInfo) anyerror!void,
    onDeviceCode: *const fn (?*anyopaque, std.Io, OAuthDeviceCodeInfo) anyerror!void,
    onPrompt: *const fn (?*anyopaque, std.mem.Allocator, std.Io, OAuthPrompt) anyerror![]const u8,
    onProgress: ?*const fn (?*anyopaque, std.Io, []const u8) anyerror!void = null,
    onManualCodeInput: ?*const fn (?*anyopaque, std.mem.Allocator, std.Io) anyerror![]const u8 = null,
    onSelect: *const fn (?*anyopaque, std.mem.Allocator, std.Io, OAuthSelectPrompt) anyerror!?[]const u8,
    errorMessage: ?*?[]const u8 = null,
    pub fn interaction(self: *@This()) auth.Interaction {
        return .{ .context = self, .prompt = prompt, .notify = notify, .errorMessage = self.errorMessage };
    }
    fn prompt(ctx: ?*anyopaque, a: std.mem.Allocator, io: std.Io, value: std.json.Value) ![]const u8 {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        const kind = j.field(value, "type") orelse return error.InvalidAuthPrompt;
        if (std.mem.eql(u8, kind, "select")) {
            const selected = try self.onSelect(self.context, a, io, try std.json.parseFromValueLeaky(OAuthSelectPrompt, a, value, .{ .ignore_unknown_fields = true }));
            return selected orelse error.LoginCancelled;
        }
        if (std.mem.eql(u8, kind, "manual_code")) if (self.onManualCodeInput) |call| return call(self.context, a, io);
        return self.onPrompt(self.context, a, io, try std.json.parseFromValueLeaky(OAuthPrompt, a, value, .{ .ignore_unknown_fields = true }));
    }
    fn notify(ctx: ?*anyopaque, io: std.Io, value: std.json.Value) !void {
        const self: *@This() = @ptrCast(@alignCast(ctx.?));
        const kind = j.field(value, "type") orelse return error.InvalidAuthEvent;
        if (std.mem.eql(u8, kind, "auth_url")) return self.onAuth(self.context, io, .{ .url = j.field(value, "url") orelse return error.InvalidAuthEvent, .instructions = j.field(value, "instructions") });
        if (std.mem.eql(u8, kind, "device_code")) return self.onDeviceCode(self.context, io, .{ .userCode = j.field(value, "userCode") orelse return error.InvalidAuthEvent, .verificationUri = j.field(value, "verificationUri") orelse return error.InvalidAuthEvent, .intervalSeconds = if (j.get(value, "intervalSeconds")) |n| j.number(n) else null, .expiresInSeconds = if (j.get(value, "expiresInSeconds")) |n| j.number(n) else null });
        if (self.onProgress) |call| return call(self.context, io, j.field(value, "message") orelse "");
    }
};
