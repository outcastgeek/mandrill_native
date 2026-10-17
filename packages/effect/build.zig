//! Build targets for the Effect package, examples, and verification programs.
const std = @import("std");

/// Register the Effect module, runnable example, and package verification steps.
/// Parameters:
/// - `b: *std.Build`: Mutable Zig build graph supplied by the Zig build runner.
/// Returns (`void`): No value; the build graph receives the declared steps.
pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});
    const effect = b.addModule("effect", .{
        .root_source_file = b.path("src/root.zig"),
        .target = target,
        .optimize = optimize,
    });
    const docs_object = b.addObject(.{
        .name = "effect-docs",
        .root_module = effect,
    });
    const install_docs = b.addInstallDirectory(.{
        .source_dir = docs_object.getEmittedDocs(),
        .install_dir = .prefix,
        .install_subdir = "docs",
    });
    const docs_evidence = b.addSystemCommand(&.{ "python3", "-B" });
    docs_evidence.addFileArg2(b.path("tests/dsl_gate_evidence.py"), .{});
    docs_evidence.addArgs(&.{ "docs", @tagName(optimize) });
    docs_evidence.addDirectoryArg2(docs_object.getEmittedDocs(), .{});
    docs_evidence.step.dependOn(&install_docs.step);
    b.step("docs", "Generate Effect API documentation in zig-out/docs").dependOn(&docs_evidence.step);
    const tests = b.addTest(.{ .root_module = effect });
    const test_step = b.step("test", "Run typed outcome and representation tests");
    test_step.dependOn(boundedTests(b, tests));
    const dsl_step = b.step("dsl-test", "Run all DSL correctness, public-consumer, and coverage gates");
    const dsl_coverage = b.addSystemCommand(&.{ "python3", "-B" });
    dsl_coverage.addFileArg2(b.path("tests/check_dsl_coverage.py"), .{});
    dsl_coverage.addArgs(&.{ "--execution", @tagName(optimize), "--self-test" });
    dsl_coverage.step.dependOn(&docs_evidence.step);
    const coverage_step = b.step("dsl-coverage", "Validate complete DSL acceptance evidence registration");
    coverage_step.dependOn(&dsl_coverage.step);
    dsl_step.dependOn(coverage_step);
    const provider_step = b.step("provider-test", "Run external facility, native I/O and mixed-provider lifecycle tests");
    const examples_step = b.step("example", "Run public consumer examples");
    const dsl_sources = @import("tests/dsl_execution.zon");
    for ([_][]const u8{ "proofs/void-binding.zig", "proofs/runtime-binding.zig", "proofs/codegen.zig", "proofs/facilities.zig", "proofs/concurrency-contracts.zig", "proofs/mixed-pipeline.zig", "proofs/lifecycle-contracts.zig", "proofs/new-compositions.zig", "proofs/reference-update.zig" } ++ @as([dsl_sources.len][]const u8, dsl_sources)) |source| {
        const consumer_tests = b.addTest(.{ .root_module = b.createModule(.{
            .root_source_file = b.path(source),
            .target = target,
            .optimize = optimize,
            .imports = &.{.{ .name = "effect", .module = effect }},
        }) });
        const is_recipe = std.mem.startsWith(u8, source, "examples/dsl/");
        const is_dsl = std.mem.startsWith(u8, source, "proofs/dsl-") or is_recipe;
        if (is_recipe) {
            const recipe = b.addExecutable(.{ .name = b.fmt("dsl-recipe-{s}", .{std.fs.path.stem(source)}), .root_module = b.createModule(.{
                .root_source_file = b.path(source),
                .target = target,
                .optimize = optimize,
                .imports = &.{.{ .name = "effect", .module = effect }},
            }) });
            const run_recipe = b.addRunArtifact(recipe);
            dsl_step.dependOn(&run_recipe.step);
            examples_step.dependOn(&run_recipe.step);
        }
        const run_consumer = if (is_dsl) registeredDslTests(b, consumer_tests, source, optimize) else boundedTests(b, consumer_tests);
        test_step.dependOn(run_consumer);
        if (is_dsl) {
            dsl_step.dependOn(run_consumer);
            dsl_coverage.step.dependOn(run_consumer);
        }
        if (std.mem.eql(u8, source, "proofs/facilities.zig") or std.mem.eql(u8, source, "proofs/concurrency-contracts.zig") or std.mem.eql(u8, source, "proofs/mixed-pipeline.zig") or std.mem.eql(u8, source, "proofs/lifecycle-contracts.zig")) provider_step.dependOn(run_consumer);
    }

    const leak_check = b.addSystemCommand(&.{ "python3", "-B" });
    leak_check.addFileArg2(b.path("tests/check_leak_detection.py"), .{});
    leak_check.addArg(b.graph.zig_exe);
    test_step.dependOn(&leak_check.step);
    b.step("leak-check", "Prove allocator leaks fail the native test runner in debug and safe").dependOn(&leak_check.step);

    const example = b.addExecutable(.{
        .name = "effect-port-example",
        .root_module = b.createModule(.{
            .root_source_file = b.path("examples/port.zig"),
            .target = target,
            .optimize = optimize,
            .imports = &.{.{ .name = "effect", .module = effect }},
        }),
    });
    dsl_step.dependOn(&b.addRunArtifact(example).step);
    examples_step.dependOn(&b.addRunArtifact(example).step);

    const independent_consumer = b.addSystemCommand(&.{ "python3", "-B" });
    independent_consumer.addFileArg2(b.path("tests/stage_consumer.py"), .{});
    independent_consumer.addArg(b.graph.zig_exe);
    independent_consumer.addArg(b.fmt("{s}", .{@tagName(optimize)}));
    independent_consumer.addDirectoryArg2(b.path("."), .{});
    dsl_step.dependOn(&independent_consumer.step);
    dsl_coverage.step.dependOn(&independent_consumer.step);
    b.step("consumer", "Build and execute the independent public-module package")
        .dependOn(&independent_consumer.step);

    const fmt = b.addFmt(.{ .paths = b.pathList(&.{ "src", "examples", "proofs", "tests", "build.zig" }), .check = true });
    b.step("fmt", "Check Zig formatting").dependOn(&fmt.step);

    const negative = b.addSystemCommand(&.{ "python3", "-B" });
    negative.addFileArg2(b.path("tests/check_compile_fail.py"), .{});
    negative.addArg(b.graph.zig_exe);
    dsl_step.dependOn(&negative.step);
    dsl_coverage.step.dependOn(&negative.step);
    b.step("compile-fail", "Check invalid consumers fail for the intended reason").dependOn(&negative.step);

    const codegen = b.addObject(.{
        .name = "effect-codegen",
        // This proof inspects LLVM's optimized output. The compiler's native
        // development backend is still exercised by the normal test target.
        .use_llvm = true,
        .root_module = b.createModule(.{
            .root_source_file = b.path("proofs/codegen.zig"),
            .target = target,
            .optimize = .fast,
            .imports = &.{.{ .name = "effect", .module = b.createModule(.{
                .root_source_file = b.path("src/root.zig"),
                .target = target,
                .optimize = .fast,
            }) }},
        }),
    });
    const assembly = b.addInstallFile(codegen.getEmittedAsm(), "proofs/effect-codegen.s");
    const optimized_ir = codegen.getEmittedLlvmIr();
    const installed_ir = b.addInstallFile(optimized_ir, "proofs/effect-codegen.ll");
    const check_codegen = b.addSystemCommand(&.{ "python3", "-B" });
    check_codegen.addFileArg2(b.path("tests/check_codegen.py"), .{});
    check_codegen.addFileArg2(optimized_ir, .{});
    dsl_coverage.step.dependOn(&check_codegen.step);
    const codegen_step = b.step("codegen", "Verify static composition matches handwritten optimized Zig");
    codegen_step.dependOn(&assembly.step);
    codegen_step.dependOn(&installed_ir.step);
    codegen_step.dependOn(&check_codegen.step);

    const performance = b.addSystemCommand(&.{ "python3", "-B" });
    performance.addFileArg2(b.path("tests/check_dsl_performance.py"), .{});
    performance.addArg(b.graph.zig_exe);
    const fast_effect = b.createModule(.{ .root_source_file = b.path("src/root.zig"), .target = target, .optimize = .fast });
    const performance_tests = b.addTest(.{ .use_llvm = true, .root_module = b.createModule(.{
        .root_source_file = b.path("proofs/dsl-performance.zig"),
        .target = target,
        .optimize = .fast,
        .imports = &.{.{ .name = "effect", .module = fast_effect }},
    }) });
    // Complete verification before serial compile/runtime measurements.
    performance.step.dependOn(registeredDslTests(b, performance_tests, "proofs/dsl-performance.zig", .fast));
    performance.step.dependOn(&check_codegen.step);
    b.step("dsl-perf", "Measure DSL allocations, execution, and stage/binding/branch/field scaling").dependOn(&performance.step);
}

fn boundedTests(b: *std.Build, binary: *std.Build.Step.Compile) *std.Build.Step {
    const run = b.addSystemCommand(&.{ "python3", "-B" });
    run.addFileArg2(b.path("tests/run_bounded.py"), .{});
    run.addArg("120"); // Test-only subprocess watchdog; not a runtime library policy.
    run.addArtifactArg2(binary, .{});
    return &run.step;
}

fn registeredDslTests(b: *std.Build, binary: *std.Build.Step.Compile, source: []const u8, optimize: std.builtin.OptimizeMode) *std.Build.Step {
    const run = b.addSystemCommand(&.{ "python3", "-B" });
    run.addFileArg2(b.path("tests/run_dsl.py"), .{});
    run.addArg(source);
    run.addArg(@tagName(optimize));
    run.addArtifactArg2(binary, .{});
    return &run.step;
}
