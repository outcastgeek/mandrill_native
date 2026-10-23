const std = @import("std");
const regexp = @import("build/regexp.zig");
pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});
    const support = b.dependency("native_build", .{});
    const module = b.addModule("ai", .{ .root_source_file = b.path("root.zig"), .target = target, .optimize = optimize });
    module.addImport("native_c_abi", support.module("c_abi"));
    regexp.add(b, module, target, optimize);
    const cli = b.addExecutable(.{ .name = "pi-ai", .root_module = b.createModule(.{ .root_source_file = b.path("src/cli.zig"), .target = target, .optimize = optimize, .imports = &.{.{ .name = "ai", .module = module }} }) });
    b.installArtifact(cli);
    const run = b.addRunArtifact(cli);
    run.addPassthruArgs();
    b.step("run", "Run the native pi-ai CLI").dependOn(&run.step);
    const unit = b.addTest(.{ .root_module = b.createModule(.{ .root_source_file = b.path("tests/root.zig"), .target = target, .optimize = optimize, .imports = &.{.{ .name = "ai", .module = module }} }) });
    const run_tests = b.addRunArtifact(unit);
    b.step("test", "Run offline parity and lifecycle tests").dependOn(&run_tests.step);
    b.step("check", "Compile the public package and offline tests").dependOn(&unit.step);
    const example_step = b.step("examples", "Compile native examples without running requests");
    inline for (.{ "completion", "streaming", "tools", "custom_provider", "auth", "compatibility" }) |name| {
        const executable = b.addExecutable(.{ .name = "ai-" ++ name, .root_module = b.createModule(.{ .root_source_file = b.path("examples/" ++ name ++ ".zig"), .target = target, .optimize = optimize, .imports = &.{.{ .name = "ai", .module = module }} }) });
        example_step.dependOn(&executable.step);
    }
    const catalog = b.addExecutable(.{ .name = "ai-catalog", .root_module = b.createModule(.{ .root_source_file = b.path("tools/catalog.zig"), .target = target, .optimize = optimize, .imports = &.{.{ .name = "ai", .module = module }} }) });
    b.step("catalog-build", "Compile native catalog tooling without running it").dependOn(&catalog.step);
    const run_catalog = b.addRunArtifact(catalog);
    run_catalog.addPassthruArgs();
    b.step("catalog", "Read or explicitly import a catalog snapshot").dependOn(&run_catalog.step);
}
