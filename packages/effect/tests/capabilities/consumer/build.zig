const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});
    const effect = b.dependency("effect", .{ .target = target, .optimize = optimize }).module("effect");
    const executable = b.addExecutable(.{
        .name = "effect-capability-consumer",
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/main.zig"),
            .target = target,
            .optimize = optimize,
            .imports = &.{.{ .name = "effect", .module = effect }},
        }),
    });
    b.step("run", "Execute an unrelated package using public Effect contracts")
        .dependOn(&b.addRunArtifact(executable).step);
}
