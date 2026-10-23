const std = @import("std");
pub fn add(b: *std.Build, module: *std.Build.Module, target: std.Build.ResolvedTarget, optimize: std.lang.Optimize) void {
    const source = b.lazyDependency("quickjs", .{}) orelse return;
    const library = b.addLibrary(.{ .name = "ai-regexp", .linkage = .static, .root_module = b.createModule(.{ .target = target, .optimize = optimize, .link_libc = true }) });
    library.root_module.addIncludePath(source.path(""));
    library.root_module.addCSourceFiles(.{ .root = source.path(""), .files = &.{ "libregexp.c", "libunicode.c", "cutils.c" }, .flags = &.{ "-std=c11", "-D_GNU_SOURCE" } });
    module.linkLibrary(library);
}
