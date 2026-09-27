const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});
    const effect = b.addModule("effect", .{
        .root_source_file = b.path("src/root.zig"),
        .target = target,
        .optimize = optimize,
    });
    const tests = b.addTest(.{ .root_module = effect });
    const test_step = b.step("test", "Run typed outcome and representation tests");
    test_step.dependOn(boundedTests(b, tests));
    const provider_step = b.step("provider-test", "Run external facility, native I/O and mixed-provider lifecycle tests");
    for ([_][]const u8{ "proofs/void-binding.zig", "proofs/runtime-binding.zig", "proofs/codegen.zig", "proofs/facilities.zig", "proofs/concurrency-contracts.zig", "proofs/mixed-pipeline.zig", "proofs/lifecycle-contracts.zig" }) |source| {
        const consumer_tests = b.addTest(.{ .root_module = b.createModule(.{
            .root_source_file = b.path(source),
            .target = target,
            .optimize = optimize,
            .imports = &.{.{ .name = "effect", .module = effect }},
        }) });
        const run_consumer = boundedTests(b, consumer_tests);
        test_step.dependOn(run_consumer);
        if (std.mem.eql(u8, source, "proofs/facilities.zig") or std.mem.eql(u8, source, "proofs/concurrency-contracts.zig") or std.mem.eql(u8, source, "proofs/mixed-pipeline.zig") or std.mem.eql(u8, source, "proofs/lifecycle-contracts.zig")) provider_step.dependOn(run_consumer);
    }

    const leak_check = b.addSystemCommand(&.{"python3"});
    leak_check.addFileArg(b.path("tests/check_leak_detection.py"));
    leak_check.addArg(b.graph.zig_exe);
    test_step.dependOn(&leak_check.step);
    b.step("leak-check", "Prove allocator leaks fail the native test runner in Debug and ReleaseSafe").dependOn(&leak_check.step);

    const example = b.addExecutable(.{
        .name = "effect-port-example",
        .root_module = b.createModule(.{
            .root_source_file = b.path("examples/port.zig"),
            .target = target,
            .optimize = optimize,
            .imports = &.{.{ .name = "effect", .module = effect }},
        }),
    });
    b.step("example", "Run an external consumer of the Effect module").dependOn(&b.addRunArtifact(example).step);

    const fmt = b.addFmt(.{ .paths = &.{ "src", "examples", "proofs", "tests", "build.zig" }, .check = true });
    b.step("fmt", "Check Zig formatting").dependOn(&fmt.step);

    const negative = b.addSystemCommand(&.{"python3"});
    negative.addFileArg(b.path("tests/check_compile_fail.py"));
    negative.addArg(b.graph.zig_exe);
    b.step("compile-fail", "Check invalid consumers fail for the intended reason").dependOn(&negative.step);

    const codegen = b.addObject(.{
        .name = "effect-codegen",
        // This proof inspects LLVM's optimized output. The compiler's native
        // development backend is still exercised by the normal test target.
        .use_llvm = true,
        .root_module = b.createModule(.{
            .root_source_file = b.path("proofs/codegen.zig"),
            .target = target,
            .optimize = .ReleaseFast,
            .imports = &.{.{ .name = "effect", .module = b.createModule(.{
                .root_source_file = b.path("src/root.zig"),
                .target = target,
                .optimize = .ReleaseFast,
            }) }},
        }),
    });
    const assembly = b.addInstallFile(codegen.getEmittedAsm(), "proofs/effect-codegen.s");
    const optimized_ir = codegen.getEmittedLlvmIr();
    const installed_ir = b.addInstallFile(optimized_ir, "proofs/effect-codegen.ll");
    const check_codegen = b.addSystemCommand(&.{"python3"});
    check_codegen.addFileArg(b.path("tests/check_codegen.py"));
    check_codegen.addFileArg(optimized_ir);
    const codegen_step = b.step("codegen", "Verify static composition matches handwritten optimized Zig");
    codegen_step.dependOn(&assembly.step);
    codegen_step.dependOn(&installed_ir.step);
    codegen_step.dependOn(&check_codegen.step);
}

fn boundedTests(b: *std.Build, binary: *std.Build.Step.Compile) *std.Build.Step {
    const run = b.addSystemCommand(&.{"python3"});
    run.addFileArg(b.path("tests/run_bounded.py"));
    run.addArg("120"); // Test-only subprocess watchdog; not a runtime library policy.
    run.addArtifactArg(binary);
    return &run.step;
}
