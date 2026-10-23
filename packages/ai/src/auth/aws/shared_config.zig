//! In-memory profile selection follows @smithy/core/config precedence. Values
//! returned here belong to the operation allocator. Credential files are never read.
const std = @import("std");
const m = @import("../../models.zig");
const t = @import("../../types.zig");
const j = @import("../../json/value.zig");
const codecs = @import("codecs.zig");
pub fn tokenCacheDigest(comptime Hash: type, a: std.mem.Allocator, identifier: []const u8) ![Hash.digest_length]u8 {
    const units = try std.unicode.wtf8ToWtf16LeAlloc(a, identifier);
    defer {
        std.crypto.secureZero(u16, units);
        a.free(units);
    }
    const scalar = try std.fmt.allocPrint(a, "{f}", .{std.unicode.fmtUtf16Le(units)});
    defer {
        std.crypto.secureZero(u8, scalar);
        a.free(scalar);
    }
    var digest: [Hash.digest_length]u8 = undefined;
    Hash.hash(scalar, &digest, .{});
    return digest;
}
pub fn inheritedObjectName(name: []const u8) bool {
    // The pinned runtime exposes these inherited names on ordinary object
    // maps. Callers use this only for their consumed source lookup behavior.
    for ([_][]const u8{ "constructor", "__defineGetter__", "__defineSetter__", "hasOwnProperty", "__lookupGetter__", "__lookupSetter__", "isPrototypeOf", "propertyIsEnumerable", "toString", "valueOf", "__proto__", "toLocaleString" }) |key| if (std.mem.eql(u8, name, key)) return true;
    return false;
}
pub fn env(operation: m.OperationContext, options: t.StreamOptions, name: []const u8) !?[]const u8 {
    return @import("../../utils/provider_env.zig").getProviderEnvValue(operation.allocator, operation.io, operation.authContext, name, options.env);
}
pub fn sdkEnv(operation: m.OperationContext, name: []const u8) !?[]const u8 {
    return if (operation.authContext) |context| try context.rawEnv(operation.allocator, operation.io, name) else null;
}
pub fn explicitProfile(operation: m.OperationContext, options: t.StreamOptions) !?[]const u8 {
    if (@import("../../api/common.zig").extra(options, "profile")) |value| if (j.truthy(value)) {
        return @import("../../utils/diagnostics.zig").formatThrownValue(operation.allocator, .{ .value = value }) catch |err| {
            if (err == error.CannotConvertObjectToPrimitive) if (operation.errorMessage) |slot| {
                slot.* = "Cannot convert object to primitive value";
            };
            return err;
        };
    };
    if (options.env.get("AWS_PROFILE")) |name| if (name.len > 0) return name;
    return null;
}
pub fn profile(operation: m.OperationContext, options: t.StreamOptions) ![]const u8 {
    if (try explicitProfile(operation, options)) |name| return name;
    if (try env(operation, options, "AWS_PROFILE")) |name| if (name.len > 0) return name;
    return "default";
}
pub fn home(operation: m.OperationContext, options: t.StreamOptions) !?[]const u8 {
    if (try env(operation, options, "HOME")) |path| return path;
    if (try env(operation, options, "USERPROFILE")) |path| return path;
    if (try env(operation, options, "HOMEPATH")) |path| return try std.fs.path.join(operation.allocator, &.{ (try env(operation, options, "HOMEDRIVE")) orelse "C:\\", path });
    if (operation.authContext) |context| if (context.homeDirFn) |get| return get(context.context, operation.allocator, operation.io);
    return null;
}
/// OS home selection is distinct from Smithy's cross-platform environment
/// fallback. The host hook supplies native account-home lookup when the
/// platform's own home environment variable is absent.
pub fn osHome(operation: m.OperationContext) !?[]const u8 {
    const key = if (@import("builtin").os.tag == .windows) "USERPROFILE" else "HOME";
    if (try sdkEnv(operation, key)) |path| {
        // libuv preserves an empty Unix HOME. On Windows a present
        // USERPROFILE shorter than three UTF8 bytes fails without fallback.
        if (@import("builtin").os.tag == .windows and path.len < 3) return error.AwsHomeDirectoryUnavailable;
        return path;
    }
    if (operation.authContext) |context| if (context.homeDirFn) |get| return get(context.context, operation.allocator, operation.io);
    return null;
}
pub fn sdkHome(operation: m.OperationContext) !?[]const u8 {
    for ([_][]const u8{ "HOME", "USERPROFILE" }) |key| if (try sdkEnv(operation, key)) |path| if (path.len > 0) return path;
    if (try sdkEnv(operation, "HOMEPATH")) |path| if (path.len > 0) return try std.fmt.allocPrint(operation.allocator, "{s}{s}", .{ (try sdkEnv(operation, "HOMEDRIVE")) orelse (if (@import("builtin").os.tag == .windows) "C:\\" else "C:/"), path });
    return osHome(operation);
}
pub const Files = struct {
    operation: m.OperationContext,
    config: ?codecs.Ini = null,
    credentials: ?codecs.Ini = null,
    pub fn init(operation: m.OperationContext, options: t.StreamOptions) !@This() {
        _ = options;
        try operation.io.checkCancel();
        const inputs = if (operation.authContext) |context| context.inputs else @import("../types.zig").CredentialInputs{};
        return .{ .operation = operation, .config = if (inputs.awsConfig != null or inputs.awsSsoSessions != null or inputs.awsServices != null) .{ .profiles = inputs.awsConfig orelse .null, .sessions = inputs.awsSsoSessions orelse .null, .services = inputs.awsServices orelse .null } else null, .credentials = if (inputs.awsCredentials) |value| .{ .profiles = value } else null };
    }
    pub fn deinit(self: *@This()) void {
        if (self.config) |*value| value.deinit();
        if (self.credentials) |*value| value.deinit();
        self.* = undefined;
    }
    fn lookup(self: @This(), file: ?codecs.Ini, section: codecs.Section, name: []const u8, key: []const u8) !?[]const u8 {
        const result = if (file) |value| try value.get(self.operation.allocator, self.operation.io, section, name, key) else null;
        // Smithy omits empty assignments from its decoded section objects.
        if (result) |text| if (text.len == 0) {
            self.operation.allocator.free(text);
            return null;
        };
        return result;
    }
    fn lookupValue(self: @This(), file: ?codecs.Ini, name: []const u8, key: []const u8) !?std.json.Value {
        const value = if (file) |input| try input.getValue(self.operation.io, .profile, name, key) else null;
        if (value) |member| if (member == .string and member.string.len == 0) return null;
        return value;
    }
    pub fn getValue(self: @This(), name: []const u8, key: []const u8) !?std.json.Value {
        return (try self.lookupValue(self.credentials, name, key)) orelse try self.lookupValue(self.config, name, key);
    }
    /// Object.keys of the merged profile, without copying credential values.
    /// The placeholder tree preserves insertion positions and JS index order.
    pub fn profilePropertyNames(self: @This(), name: []const u8, added: []const []const u8) ![]const []const u8 {
        try self.operation.io.checkCancel();
        const a = self.operation.allocator;
        var keys = j.map(a);
        for ([_]?codecs.Ini{ self.config, self.credentials }) |file| if (file) |input| {
            const profile_value = j.get(input.profiles, name) orelse continue;
            if (profile_value != .object) continue;
            for (try j.objectEntries(a, profile_value.object)) |entry| {
                if (entry.node.* == .string and entry.node.string.len == 0) continue;
                try j.put(a, &keys, entry.key, .null);
            }
        };
        for (added) |key| try j.put(a, &keys, key, .null);
        const entries = try j.objectEntries(a, keys.object);
        const names = try a.alloc([]const u8, entries.len);
        for (entries, names) |entry, *name_ptr| name_ptr.* = entry.key;
        return names;
    }
    /// Credentials overwrite config properties in the credential chain.
    pub fn get(self: @This(), name: []const u8, key: []const u8) !?[]const u8 {
        const value = (try self.getValue(name, key)) orelse return null;
        return if (j.text(value)) |text| try self.operation.allocator.dupe(u8, text) else null;
    }
    /// Client settings merge preferred config properties before selectors
    /// inspect their type. A non-string preferred property still shadows.
    pub fn settingValue(self: @This(), name: []const u8, key: []const u8) !?std.json.Value {
        return (try self.lookupValue(self.config, name, key)) orelse try self.lookupValue(self.credentials, name, key);
    }
    pub fn setting(self: @This(), name: []const u8, key: []const u8) !?[]const u8 {
        const value = (try self.settingValue(name, key)) orelse return null;
        return if (j.text(value)) |text| try self.operation.allocator.dupe(u8, text) else null;
    }
    pub fn session(self: @This(), name: []const u8, key: []const u8) !?[]const u8 {
        return self.lookup(self.config, .sso_session, name, key);
    }
    pub fn serviceEndpoint(self: @This(), name: []const u8, service: []const u8) !?[]const u8 {
        if (try self.setting(name, "services")) |section| {
            const file = self.config orelse return error.AwsServicesSectionMissing;
            if (!try file.hasSection(self.operation.io, .services, section)) return error.AwsServicesSectionMissing;
            if (try file.getSubproperty(self.operation.allocator, self.operation.io, .services, section, service, "endpoint_url")) |value| if (value.len > 0) return value;
        }
        return self.setting(name, "endpoint_url");
    }
};
pub fn setting(operation: m.OperationContext, options: t.StreamOptions, key: []const u8) !?[]const u8 {
    var files = try Files.init(operation, options);
    defer files.deinit();
    return files.setting(try profile(operation, options), key);
}
pub fn settingValue(operation: m.OperationContext, options: t.StreamOptions, key: []const u8) !?std.json.Value {
    var files = try Files.init(operation, options);
    defer files.deinit();
    return files.settingValue(try profile(operation, options), key);
}
/// loadConfig's boolean selector failures continue to the next provider:
/// invalid env -> merged config -> static false. Property-key lookup failures
/// remain outside that selector catch and retain their existing propagation.
pub fn configBoolean(operation: m.OperationContext, options: t.StreamOptions, environment: []const u8, key: []const u8) !bool {
    if (try sdkEnv(operation, environment)) |value| {
        if (std.mem.eql(u8, value, "true")) return true;
        if (std.mem.eql(u8, value, "false")) return false;
    }
    if (try settingValue(operation, options, key)) |value| if (value == .string) return std.mem.eql(u8, value.string, "true");
    return false;
}
