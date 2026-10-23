const std = @import("std");
pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});
    const dependency = b.dependency("ai", .{ .target = target, .optimize = optimize });
    const app = b.addExecutable(.{ .name = "ai-consumer-example", .root_module = b.createModule(.{
        .root_source_file = b.path("main.zig"),
        .target = target,
        .optimize = optimize,
        .imports = &.{.{ .name = "ai", .module = dependency.module("ai") }},
    }) });
    b.installArtifact(app);
}
