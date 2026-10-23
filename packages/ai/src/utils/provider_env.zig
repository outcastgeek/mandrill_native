//! Scoped provider values precede the application's ambient snapshot.
//! Native consumers supply that snapshot through AuthContext; no module
//! import reads an environment or installs a process-global cache.
const std = @import("std");
const t = @import("../types.zig");
const auth = @import("../auth/types.zig");
pub fn getProviderEnvValue(a: std.mem.Allocator, io: std.Io, context: ?auth.AuthContext, name: []const u8, overrides: t.ProviderEnv) !?[]const u8 {
    try io.checkCancel();
    if (overrides.get(name)) |value| if (value.len > 0) return try a.dupe(u8, value);
    const host = context orelse return null;
    const resolved = try host.env(a, io, name);
    errdefer if (resolved) |value| a.free(value);
    try io.checkCancel();
    const value = resolved orelse return null;
    if (value.len == 0) {
        a.free(value);
        return null;
    }
    return value;
}
