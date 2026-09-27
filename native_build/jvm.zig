const std = @import("std");

pub const Tool = struct { classes: std.Build.LazyPath, java: []const u8 };

/// Compile upstream jextract and the Zig frontend adapter with the provided JDK.
pub fn addTool(b: *std.Build, jdk_home: []const u8) Tool {
    const source = b.dependency("jextract", .{});
    const java = b.pathJoin(&.{ jdk_home, "bin", "java" });
    const compile = b.addSystemCommand(&.{java});
    compile.addFileArg(b.path("native_build/jvm/CompileTool.java"));
    compile.addDirectoryArg(source.path(""));
    compile.addDirectoryArg(b.path("native_build/jvm"));
    compile.addFileInput(b.path("native_build/jvm/ZigFrontend.java"));
    compile.addFileInput(b.path("native_build/jvm/ForeignMetadata.java"));
    return .{ .classes = compile.addOutputDirectoryArg("jextract-classes"), .java = java };
}

pub const BindingsOptions = struct {
    header: std.Build.LazyPath,
    package_name: []const u8,
    class_name: []const u8,
    clang_target: []const u8,
    zig_lib_dir: []const u8,
    includes: []const std.Build.LazyPath = &.{},
    defines: []const []const u8 = &.{},
    constants: []const []const u8 = &.{},
    freestanding: bool = false,
};

/// Generate bindings with Zig's frontend and upstream jextract's generator.
pub fn addBindings(b: *std.Build, tool: Tool, options: BindingsOptions) std.Build.LazyPath {
    const generate = b.addSystemCommand(&.{ tool.java, "--enable-native-access=org.openjdk.jextract", "--module-path" });
    generate.addDirectoryArg(tool.classes);
    generate.addArg(b.fmt("-Djextract.target.windows={s}", .{if (std.mem.indexOf(u8, options.clang_target, "windows") != null) "true" else "false"}));
    generate.addArgs(&.{ "-m", "org.openjdk.jextract/org.openjdk.jextract.impl.ZigFrontend" });
    const output = generate.addOutputDirectoryArg("bindings");
    generate.addArgs(&.{ options.package_name, options.class_name });
    generate.addFileArg(options.header);
    generate.addArg(std.mem.join(b.allocator, ",", options.constants) catch @panic("out of memory"));
    generate.addArgs(&.{ b.graph.zig_exe, "clang", "-cc1", "-triple", options.clang_target, "-x", "c", "-nostdsysteminc", "-internal-isystem", b.pathJoin(&.{ options.zig_lib_dir, "include" }) });
    if (options.freestanding) generate.addArg("-ffreestanding");
    for (options.includes) |include| {
        generate.addArg("-I");
        generate.addDirectoryArg(include);
    }
    for (options.includes) |include| trackSources(b, generate, include);
    for (options.defines) |define| {
        generate.addArg("-D");
        generate.addArg(define);
    }
    return output;
}

/// Compile generated bindings into reusable JVM class files.
pub fn compileBindings(b: *std.Build, tool: Tool, generated: std.Build.LazyPath, sources: []const std.Build.LazyPath) std.Build.LazyPath {
    const compile = b.addSystemCommand(&.{tool.java});
    compile.addFileArg(b.path("native_build/jvm/CompileBindings.java"));
    const classes = compile.addOutputDirectoryArg("classes");
    compile.addDirectoryArg(generated.path(b, "java"));
    compile.addDirectoryArg(b.path("native_build/jvm/runtime"));
    trackSources(b, compile, b.path("native_build/jvm/runtime"));
    for (sources) |source| {
        compile.addDirectoryArg(source);
        trackSources(b, compile, source);
    }
    return classes;
}

pub const ImageOptions = struct {
    jdk_home: []const u8,
    classes: std.Build.LazyPath,
    class_path_file: ?std.Build.LazyPath = null,
    metadata: std.Build.LazyPath,
    additional_metadata: []const std.Build.LazyPath = &.{},
    compiler: *std.Build.Step.Compile,
    main_class: []const u8,
    name: []const u8,
    runtime_packages: []const u8,
    static_archive: ?std.Build.LazyPath = null,
    runtime_archives: []const std.Build.LazyPath = &.{},
    args: []const []const u8 = &.{},
};

pub fn addImageCompiler(b: *std.Build, exports: ?std.Build.LazyPath) *std.Build.Step.Compile {
    const options = b.addOptions();
    options.addOption([]const u8, "zig_exe", b.graph.zig_exe);

    const module = b.createModule(.{
        .root_source_file = b.path("native_build/jvm/compiler_proxy.zig"),
        .target = b.graph.host,
        .optimize = .ReleaseSafe,
    });
    module.addOptions("build_options", options);
    module.addAnonymousImport("native_exports", .{ .root_source_file = exports orelse b.addWriteFiles().add("exports.txt", "") });
    return b.addExecutable(.{ .name = "jvm-zig-cc", .root_module = module });
}

/// Native Image uses the supplied Zig compiler proxy for C queries and final linking.
pub fn addImage(b: *std.Build, options: ImageOptions) std.Build.LazyPath {
    const run = b.addSystemCommand(&.{b.pathJoin(&.{ options.jdk_home, "bin", "java" })});
    run.addFileArg(b.path("native_build/jvm/CompileImage.java"));
    run.addArgs(&.{ b.pathJoin(&.{ options.jdk_home, "bin", "native-image" }), "--no-fallback", "--enable-native-access=ALL-UNNAMED", "-Ob", "--parallelism=4", "-J-Xmx4g", "-H:+UnlockExperimentalVMOptions", "-H:-CheckToolchain", "-H:+SharedArenaSupport", "-cp" });
    run.expectExitCode(0);
    run.addDirectoryArg(options.classes);
    if (options.class_path_file) |file| run.addPrefixedFileArg("--class-path-file=", file);
    run.addPrefixedDirectoryArg("-H:ConfigurationFileDirectories=", options.metadata);
    for (options.additional_metadata) |metadata| run.addPrefixedDirectoryArg("-H:ConfigurationFileDirectories=", metadata);
    run.addPrefixedFileArg("-H:CCompilerPath=", options.compiler.getEmittedBin());
    run.addArg(b.fmt("--initialize-at-run-time={s}", .{options.runtime_packages}));
    if (options.static_archive) |archive| {
        run.addPrefixedFileArg("-H:NativeLinkerOption=-Wl,-force_load,", archive);
        for (options.runtime_archives) |runtime| run.addPrefixedFileArg("-H:NativeLinkerOption=", runtime);
    }
    run.addArgs(options.args);
    run.addArg(options.main_class);
    run.addArg("-o");
    return run.addOutputFileArg(options.name);
}

// Directory arguments alone do not hash their contents in Zig's Run cache.
pub fn trackSources(b: *std.Build, run: *std.Build.Step.Run, source: std.Build.LazyPath) void {
    const io = b.graph.io;
    const directory = std.Io.Dir.openDirAbsolute(io, source.getPath(b), .{ .iterate = true }) catch @panic("cannot open JVM source directory");
    defer directory.close(io);
    var walker = directory.walk(b.allocator) catch @panic("out of memory");
    defer walker.deinit();
    while (walker.next(io) catch @panic("cannot enumerate JVM inputs")) |entry| {
        if (entry.kind == .file) run.addFileInput(source.path(b, entry.path));
    }
}
