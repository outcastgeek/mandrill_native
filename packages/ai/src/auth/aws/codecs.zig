//! Credential acquisition boundaries after removal of the AWS SDK. Profile
//! sections arrive as decoded in-memory trees; filesystem retrieval is banned.
//! The network STS XML decoder remains unfinished under the structured-data
//! parsing restriction. Its absence never disables SigV4 or supplied keys.
const std = @import("std");
pub const Source = enum(u32) { config = 1, credentials = 2 };
pub const Section = enum(u32) { profile = 0, sso_session = 1, services = 2 };

/// Already decoded in-memory sections. No INI/file credential discovery occurs.
pub const Ini = struct {
    profiles: std.json.Value = .null,
    sessions: std.json.Value = .null,
    services: std.json.Value = .null,
    pub fn deinit(self: *@This()) void {
        self.* = undefined;
    }
    fn sectionValue(self: @This(), section: Section, name: []const u8) ?std.json.Value {
        const j = @import("../../json/value.zig");
        return j.get(switch (section) {
            .profile => self.profiles,
            .sso_session => self.sessions,
            .services => self.services,
        }, name);
    }
    pub fn hasSection(self: @This(), io: std.Io, section: Section, name: []const u8) !bool {
        try io.checkCancel();
        return self.sectionValue(section, name) != null;
    }
    pub fn get(self: @This(), a: std.mem.Allocator, io: std.Io, section: Section, name: []const u8, property: []const u8) !?[]const u8 {
        return self.getSubproperty(a, io, section, name, property, null);
    }
    /// Borrows the supplied section tree through the operation lifetime.
    pub fn getValue(self: @This(), io: std.Io, section: Section, name: []const u8, property: []const u8) !?std.json.Value {
        try io.checkCancel();
        return @import("../../json/value.zig").get(self.sectionValue(section, name) orelse return null, property);
    }
    pub fn getSubproperty(self: @This(), a: std.mem.Allocator, io: std.Io, section: Section, name: []const u8, property: []const u8, subproperty: ?[]const u8) !?[]const u8 {
        const j = @import("../../json/value.zig");
        var value = (try self.getValue(io, section, name, property)) orelse return null;
        if (subproperty) |key| value = j.get(value, key) orelse return null;
        return if (j.text(value)) |text| try a.dupe(u8, text) else null;
    }
};
pub const StsCredentials = struct { accessKeyId: []const u8, secretAccessKey: []const u8, sessionToken: []const u8, expiration: i64 };
pub const StsResponse = struct { credentials: ?StsCredentials = null, code: ?[]const u8 = null, message: ?[]const u8 = null, requestId: ?[]const u8 = null };
pub fn decodeSts(a: std.mem.Allocator, io: std.Io, xml: []const u8) !StsResponse {
    _ = a;
    _ = xml;
    try io.checkCancel();
    return error.AwsStsXmlCodecRequired;
}
