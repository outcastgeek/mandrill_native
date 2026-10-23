const std = @import("std");
const builtin = @import("builtin");
pub fn getPiUserAgent(a: std.mem.Allocator) ![]const u8 {
    const platform = switch (builtin.os.tag) {
        .macos => "darwin",
        .windows => "win32",
        .illumos => "sunos",
        else => @tagName(builtin.os.tag),
    };
    const arch = switch (builtin.cpu.arch) {
        .x86_64 => "x64",
        .aarch64, .aarch64_be => "arm64",
        .x86 => "ia32",
        .arm, .armeb, .thumb, .thumbeb => "arm",
        .powerpc, .powerpcle => "ppc",
        .powerpc64, .powerpc64le => "ppc64",
        .loongarch64 => "loong64",
        else => @tagName(builtin.cpu.arch),
    };
    const release = if (comptime builtin.os.tag == .windows) blk: {
        var version = std.mem.zeroes(std.os.windows.RTL_OSVERSIONINFOW);
        version.dwOSVersionInfoSize = @sizeOf(@TypeOf(version));
        if (std.os.windows.ntdll.RtlGetVersion(&version) != .SUCCESS) return error.SystemVersionUnavailable;
        break :blk try std.fmt.allocPrint(a, "{d}.{d}.{d}", .{ version.dwMajorVersion, version.dwMinorVersion, version.dwBuildNumber });
    } else if (comptime builtin.os.tag != .wasi and builtin.os.tag != .freestanding) blk: {
        const system = std.posix.uname();
        break :blk try a.dupe(u8, std.mem.sliceTo(&system.release, 0));
    } else "unknown";
    defer if (comptime builtin.os.tag != .wasi and builtin.os.tag != .freestanding) a.free(release);
    return std.fmt.allocPrint(a, "pi ({s} {s}; {s})", .{ platform, release, arch });
}
