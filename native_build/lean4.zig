//! Reusable Zig/Lean 4 build integration.
//!
//! Lake emits project C and a static archive through Zig compiler proxies.

const std = @import("std");

const lean_revision = "d8b18978322de05a8f3dba51ef03cf5461676c17";
pub const Tools = struct {
    lake_exe: []const u8,
    lean_exe: []const u8,
    leanc_exe: []const u8,
    version: []const u8,
    target_triple: []const u8,
    target_cpu: []const u8,
    cc: *std.Build.Step.Compile,
    ar: *std.Build.Step.Compile,
    install_cc: *std.Build.Step.InstallArtifact,
    install_ar: *std.Build.Step.InstallArtifact,
    cc_path: []const u8,
    ar_path: []const u8,
    verify: *std.Build.Step,
};

pub const Context = struct {
    support_dir: std.Build.LazyPath,
    b: *std.Build,
    target: std.Build.ResolvedTarget,
    optimize: std.builtin.OptimizeMode,
    tools: Tools,
    runtime_module: *std.Build.Module,
    runtime_dir: ?[]const u8,
};

pub const LibraryOptions = struct {
    name: []const u8,
    /// Directory containing lakefile.lean.
    project_dir: []const u8,
    /// Lake static-library facet.
    lake_target: []const u8,
    /// Archive path beneath Lake's configured targetDir.
    archive_relative: []const u8,
    /// C files beneath targetDir/ir required by the final foreign build.
    generated_sources: []const []const u8,
};

pub const ForeignLibrary = struct {
    artifact: *std.Build.Step.Compile,
};

pub const Library = struct {
    name: []const u8,
    context: Context,
    project_dir: []const u8,
    build_dir: []const u8,
    generated_c_dir: std.Build.LazyPath,
    archive: std.Build.LazyPath,
    lake_build: *std.Build.Step.Run,
    foreign: ?ForeignLibrary,

    /// Links the selected Lean project and the matching prebuilt Lean runtime
    /// into a final native artifact.
    pub fn link(self: Library, artifact: *std.Build.Step.Compile) void {
        const project_archive = if (self.foreign) |foreign| foreign.artifact.getEmittedBin() else self.archive;
        artifact.root_module.addObjectFile(project_archive);
        artifact.step.dependOn(&self.lake_build.step);
        const runtime_dir = self.context.runtime_dir orelse {
            artifact.step.dependOn(&self.context.b.addFail("cross-target Lean final links require -Dlean-runtime-dir").step);
            return;
        };
        const toolchain_lib = std.fs.path.dirname(runtime_dir) orelse @panic("invalid Lean runtime directory");
        for (self.runtimeArchives().?) |archive| artifact.root_module.addObjectFile(archive);
        artifact.root_module.addLibraryPath(.{ .cwd_relative = self.context.b.fmt("{s}/libc", .{toolchain_lib}) });
        artifact.root_module.link_libcpp = true;
    }
    /// Runtime link inputs shared by Zig artifacts and Native Image final links.
    pub fn runtimeArchives(self: Library) ?[]const std.Build.LazyPath {
        const runtime_dir = self.context.runtime_dir orelse return null;
        const toolchain_lib = std.fs.path.dirname(runtime_dir) orelse @panic("invalid Lean runtime directory");
        const paths = self.context.b.allocator.alloc(std.Build.LazyPath, 6) catch @panic("out of memory");
        paths[0] = .{ .cwd_relative = self.context.b.fmt("{s}/libInit.a", .{runtime_dir}) };
        paths[1] = .{ .cwd_relative = self.context.b.fmt("{s}/libleanrt.a", .{runtime_dir}) };
        for ([_][]const u8{ "gmp", "uv", "ssl", "crypto" }, 2..) |name, i|
            paths[i] = .{ .cwd_relative = self.context.b.fmt("{s}/lib{s}.a", .{ toolchain_lib, name }) };
        return paths;
    }
};

pub fn create(
    b: *std.Build,
    target: std.Build.ResolvedTarget,
    optimize: std.builtin.OptimizeMode,
) Context {
    return createWithSupport(b, target, optimize, b.path("native_build"));
}

pub fn createWithSupport(b: *std.Build, target: std.Build.ResolvedTarget, optimize: std.builtin.OptimizeMode, support_dir: std.Build.LazyPath) Context {
    const requested_triple = target.query.zigTriple(b.allocator) catch @panic("out of memory");
    const target_triple = (if (target.query.isNative()) target.result.zigTriple(b.allocator) else b.allocator.dupe(u8, requested_triple)) catch @panic("out of memory");
    const target_cpu = target.query.serializeCpuAlloc(b.allocator) catch @panic("out of memory");
    const lake = b.option([]const u8, "lake", "Path to Lake") orelse "lake";
    const lean = b.option([]const u8, "lean", "Path to Lean") orelse "lean";
    const leanc = b.option([]const u8, "leanc", "Path to leanc") orelse "leanc";
    const version = b.option([]const u8, "lean-version", "Required Lean version") orelse "4.33.0";
    const runtime_dir = b.option([]const u8, "lean-runtime-dir", "Directory containing target-compatible libInit.a and libleanrt.a") orelse if (target.query.isNative()) blk: {
        const prefix_output = b.run(&.{ lean, "--print-prefix" });
        const prefix = std.mem.trim(u8, prefix_output, &std.ascii.whitespace);
        if (prefix.len == 0) @panic("lean --print-prefix returned an empty path");
        break :blk b.fmt("{s}/lib/lean", .{prefix});
    } else null;

    const cc_name = b.fmt("lean-zig-cc-{s}", .{requested_triple});
    const ar_name = b.fmt("lean-zig-ar-{s}", .{requested_triple});
    const cc = addProxy(b, support_dir, cc_name, "cc", requested_triple, target_cpu);
    const ar = addProxy(b, support_dir, ar_name, "ar", "", "");
    const install_cc = b.addInstallArtifact(cc, .{});
    const install_ar = b.addInstallArtifact(ar, .{});

    const verifier = b.addExecutable(.{
        .name = "verify-lean-toolchain",
        .root_module = b.createModule(.{
            .root_source_file = support_dir.path(b, "lean4/verify_toolchain.zig"),
            .target = b.graph.host,
            .optimize = .ReleaseSafe,
        }),
    });
    const run_verifier = b.addRunArtifact(verifier);
    run_verifier.addArgs(&.{ lake, lean, leanc, version });

    return .{
        .b = b,
        .support_dir = support_dir,
        .target = target,
        .optimize = optimize,
        .tools = .{
            .lake_exe = lake,
            .lean_exe = lean,
            .leanc_exe = leanc,
            .version = version,
            .target_triple = target_triple,
            .target_cpu = target_cpu,
            .cc = cc,
            .ar = ar,
            .install_cc = install_cc,
            .install_ar = install_ar,
            .cc_path = b.pathFromRoot(b.getInstallPath(.bin, cc_name)),
            .ar_path = b.pathFromRoot(b.getInstallPath(.bin, ar_name)),
            .verify = &run_verifier.step,
        },
        .runtime_module = b.createModule(.{
            .root_source_file = support_dir.path(b, "lean4/runtime.zig"),
            .target = target,
            .optimize = optimize,
        }),
        .runtime_dir = runtime_dir,
    };
}

pub fn addLibrary(context: Context, options: LibraryOptions) Library {
    const b = context.b;
    const build_dir = b.path(b.fmt(".zig-cache/lean4/{s}/{s}/{s}/{s}", .{
        options.name,
        context.tools.target_triple,
        context.tools.target_cpu,
        @tagName(context.optimize),
    })).getPath(b);
    const lake_build = b.addSystemCommand(&.{ context.tools.lake_exe, "-R", "-K", b.fmt("targetDir={s}", .{build_dir}), "build", options.lake_target });
    lake_build.setCwd(context.b.path(options.project_dir));
    lake_build.setEnvironmentVariable("LEAN_CC", context.tools.cc_path);
    lake_build.setEnvironmentVariable("LEAN_AR", context.tools.ar_path);
    lake_build.step.dependOn(&context.tools.install_cc.step);
    lake_build.step.dependOn(&context.tools.install_ar.step);
    lake_build.step.dependOn(context.tools.verify);

    var result = Library{
        .name = options.name,
        .context = context,
        .project_dir = options.project_dir,
        .build_dir = build_dir,
        .generated_c_dir = .{ .cwd_relative = b.fmt("{s}/ir", .{build_dir}) },
        .archive = .{ .cwd_relative = b.fmt("{s}/{s}", .{ build_dir, options.archive_relative }) },
        .lake_build = lake_build,
        .foreign = null,
    };
    if (!context.target.query.isNative()) result.foreign = .{
        .artifact = addForeignProjectLibrary(result, options.generated_sources),
    };
    return result;
}

fn addForeignProjectLibrary(library: Library, sources: []const []const u8) *std.Build.Step.Compile {
    const b = library.context.b;
    const lean4 = b.dependency("lean4", .{});
    const platform_target = b.fmt("-DLEAN_PLATFORM_TARGET=\"{s}\"", .{library.context.tools.target_triple});
    const module = b.createModule(.{
        .target = library.context.target,
        .optimize = library.context.optimize,
        .link_libc = true,
        .pic = true,
        .strip = library.context.optimize != .Debug,
    });
    module.addIncludePath(library.context.support_dir.path(b, "lean4/overlay/include"));
    module.addIncludePath(lean4.path("src/include"));
    module.addCSourceFiles(.{
        .root = library.generated_c_dir,
        .files = sources,
        .flags = &.{ "-ffunction-sections", "-fdata-sections", "-DLEAN_EXPORTING", platform_target },
        .language = .c,
    });
    const artifact = b.addLibrary(.{ .name = b.fmt("{s}-lean-{s}", .{ library.name, library.context.tools.target_triple }), .root_module = module });
    artifact.step.dependOn(&library.lake_build.step);
    return artifact;
}

fn addProxy(b: *std.Build, support_dir: std.Build.LazyPath, name: []const u8, subcommand: []const u8, triple: []const u8, cpu: []const u8) *std.Build.Step.Compile {
    const options = b.addOptions();
    options.addOption([]const u8, "zig_exe", b.graph.zig_exe);
    options.addOption([]const u8, "subcommand", subcommand);
    options.addOption([]const u8, "target_triple", triple);
    options.addOption([]const u8, "target_cpu", cpu);
    const module = b.createModule(.{
        .root_source_file = support_dir.path(b, "lean4/lean_zig_tool.zig"),
        .target = b.graph.host,
        .optimize = .ReleaseSafe,
    });
    module.addOptions("build_options", options);
    return b.addExecutable(.{ .name = name, .root_module = module });
}

pub fn revision() []const u8 {
    return lean_revision;
}
