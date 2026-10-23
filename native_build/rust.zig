//! Reusable Cargo/staticlib integration for Zig-owned programs.
//!
//! Rust libraries may import symbols exported by Zig. The final Zig link owns
//! symbol resolution, so Rust -> Zig -> Rust call chains require no special
//! trampoline and have no build-system depth limit.

const std = @import("std");

pub const Context = struct {
    b: *std.Build,
    target: std.Build.ResolvedTarget,
    optimize: std.lang.Optimize,
    support_dir: std.Build.LazyPath,
    cargo_exe: []const u8,
    rustup_exe: []const u8,
    android_ndk: ?[]const u8,
    android_api: u16,
};

pub const LibraryOptions = struct {
    /// Stable build-graph name. It need not match the Cargo package name.
    name: []const u8,
    /// Directory containing Cargo.toml, relative to the build root.
    project_dir: []const u8,
    /// Generated/staged projects retain build-graph dependencies.
    project_path: ?std.Build.LazyPath = null,
    /// Rust crate name as emitted on disk (`-` normalized to `_`).
    crate_name: []const u8,
    manifest_name: []const u8 = "Cargo.toml",
    features: []const []const u8 = &.{},
    no_default_features: bool = false,
    cargo_args: []const []const u8 = &.{},
    /// Select an already-installed toolchain without rustup's implicit install
    /// behavior when a rust-toolchain file is encountered by the cargo shim.
    toolchain: ?[]const u8 = null,
};

pub const Library = struct {
    name: []const u8,
    rust_target: []const u8,
    archive: std.Build.LazyPath,
    build: *std.Build.Step.Run,
    context: Context,

    /// Attach the Rust archive and its platform runtime dependencies to any
    /// Zig compile artifact. Zig callbacks referenced by Rust are resolved by
    /// the final link.
    pub fn link(self: Library, artifact: *std.Build.Step.Compile) void {
        artifact.step.dependOn(&self.build.step);
        if(self.linkModule(artifact.root_module)) |libc| artifact.setLibCFile(libc);
    }
    /// Imported modules propagate their archive, generated-path dependencies,
    /// and system libraries to each consumer. Android's artifact-level libc
    /// file is returned for the final compile step to attach.
    pub fn linkModule(self: Library,module: *std.Build.Module) ?std.Build.LazyPath {
        module.addObjectFile(self.archive);
        return linkRuntimeModule(self.context,module);
    }
};

pub fn create(
    b: *std.Build,
    target: std.Build.ResolvedTarget,
    optimize: std.lang.Optimize,
) Context {
    return createWithSupport(b, target, optimize, b.path("native_build"));
}

pub fn createWithSupport(b: *std.Build, target: std.Build.ResolvedTarget, optimize: std.lang.Optimize, support_dir: std.Build.LazyPath) Context {
    return .{
        .b = b,
        .support_dir = support_dir,
        .target = target,
        .optimize = optimize,
        .cargo_exe = b.option([]const u8, "cargo", "Path to Cargo") orelse "cargo",
        .rustup_exe = b.option([]const u8, "rustup", "Path to rustup") orelse "rustup",
        .android_ndk = b.option([]const u8, "android-ndk", "Android NDK root used by Rust and Zig"),
        .android_api = b.option(u16, "android-api", "Android API level (minimum 24)") orelse 24,
    };
}

pub fn addLibrary(context: Context, options: LibraryOptions) Library {
    const b = context.b;
    const rust_target = targetTriple(context.target.result);
    const profile = if (context.optimize == .debug) "debug" else "release";
    const cargo = b.addSystemCommand(&.{"sh"});
    cargo.addFileArg2(context.support_dir.path(b, "cargo-build"), .{});
    cargo.addArg(context.cargo_exe);
    cargo.addDirectoryArg2(options.project_path orelse b.path(options.project_dir), .{});
    const target_dir = cargo.addOutputDirectoryArg2(options.name, .{});
    cargo.addDirectoryArg2(context.support_dir, .{});
    cargo.addFileArg2(.zig_exe, .{});
    cargo.addArg(rust_target);
    cargo.addArg(if (context.target.query.isNative()) "" else context.target.query.zigTriple(b.allocator) catch @panic("out of memory"));
    cargo.addArg(context.rustup_exe);
    cargo.addArg(options.toolchain orelse "-");
    cargo.addArgs(&.{ "--manifest-path", options.manifest_name });
    if (context.optimize != .debug) cargo.addArg("--release");
    if (options.no_default_features) cargo.addArg("--no-default-features");
    if (options.features.len != 0) {
        cargo.addArg("--features");
        cargo.addArg(joinComma(b, options.features));
    }
    cargo.addArgs(options.cargo_args);

    cargo.setEnvironmentVariable("ZIG_RUST_TARGET", rust_target);
    cargo.setEnvironmentVariable("ZIG_RUST_ZIG", b.graph.zig_exe);
    setTargetEnvironment(context,cargo);
    // Native Cargo build scripts and proc macros must use the host linker.
    // The target-specific Zig linker is only required for cross compilation.
    const archive_name = b.fmt("lib{s}.a", .{normalizeCrateName(b, options.crate_name)});
    return .{
        .name = options.name,
        .rust_target = rust_target,
        .archive = target_dir.path(b, b.fmt("{s}/{s}/{s}", .{ rust_target, profile, archive_name })),
        .build = cargo,
        .context = context,
    };
}
/// Keep CMake and Cargo compiler wrappers on the final Zig target's SDK and
/// deployment settings, including C build scripts invoked by Rust crates.
pub fn setTargetEnvironment(context: Context,command: *std.Build.Step.Run) void {
    const b=context.b;
    command.setEnvironmentVariable("ZIG_RUST_ANDROID_API",b.fmt("{d}",.{context.android_api}));
    if(context.android_ndk) |ndk| command.setEnvironmentVariable("ZIG_RUST_ANDROID_NDK",ndk);
    switch(context.target.result.os.tag) {
        .macos,.ios=>|tag| {
            const version=context.target.result.os.version_range.semver.min;
            const deployment=b.fmt("{d}.{d}.{d}",.{version.major,version.minor,version.patch});
            command.setEnvironmentVariable("ZIG_RUST_APPLE_DEPLOYMENT_TARGET",deployment);
            command.setEnvironmentVariable(if(tag == .macos) "MACOSX_DEPLOYMENT_TARGET" else "IPHONEOS_DEPLOYMENT_TARGET",deployment);
        },
        else=>{},
    }
}

pub fn addTargetInstallStep(context: Context, name: []const u8) *std.Build.Step {
    const rust_target = targetTriple(context.target.result);
    const command = context.b.addSystemCommand(&.{ context.rustup_exe, "target", "add", rust_target });
    const step = context.b.step(name, "Install the selected Rust standard-library target through rustup");
    step.dependOn(&command.step);
    return step;
}

pub fn targetTriple(target: std.Target) []const u8 {
    return switch (target.os.tag) {
        .linux => switch (target.cpu.arch) {
            .x86_64 => if (target.abi == .android) "x86_64-linux-android" else if (target.abi == .musl) "x86_64-unknown-linux-musl" else "x86_64-unknown-linux-gnu",
            .aarch64 => if (target.abi == .android) "aarch64-linux-android" else if (target.abi == .musl) "aarch64-unknown-linux-musl" else "aarch64-unknown-linux-gnu",
            else => @panic("Rust integration supports x86_64 and aarch64 Linux targets"),
        },
        .windows => switch (target.cpu.arch) {
            .x86_64 => "x86_64-pc-windows-gnu",
            .aarch64 => "aarch64-pc-windows-gnullvm",
            else => @panic("Rust integration supports x86_64 and aarch64 Windows targets"),
        },
        .macos => switch (target.cpu.arch) {
            .x86_64 => "x86_64-apple-darwin",
            .aarch64 => "aarch64-apple-darwin",
            else => @panic("Rust integration supports x86_64 and aarch64 macOS targets"),
        },
        .ios => switch (target.cpu.arch) {
            .x86_64 => "x86_64-apple-ios",
            .aarch64 => if (target.abi == .simulator) "aarch64-apple-ios-sim" else "aarch64-apple-ios",
            else => @panic("Rust integration supports x86_64 and aarch64 iOS targets"),
        },
        else => @panic("unsupported Rust target OS"),
    };
}

pub fn addRuntimeLinkArgs(context: Context, command: *std.Build.Step.Run) void {
    switch (context.target.result.os.tag) {
        .linux => if (context.target.result.abi == .android)
            command.addArgs(&.{ "-lc", "-lm", "-ldl", "-llog" })
        else
            command.addArgs(&.{ "-lc", "-lm", "-lpthread", "-ldl", "-lunwind" }),
        .windows => command.addArgs(&.{ "-lkernel32", "-lws2_32", "-luserenv", "-lunwind", "-lc" }),
        .macos => command.addArgs(&.{"-lc"}),
        .ios => command.addArgs(&.{"-lSystem"}),
        else => command.addArgs(&.{"-lc"}),
    }
}

fn linkRuntimeModule(context: Context, module: *std.Build.Module) ?std.Build.LazyPath {
    var libc_file: ?std.Build.LazyPath=null;
    switch (context.target.result.os.tag) {
        .linux => if (context.target.result.abi == .android) {
            libc_file=configureAndroidModule(context, module);
            module.linkSystemLibrary("c", .{});
            module.linkSystemLibrary("m", .{});
            module.linkSystemLibrary("dl", .{});
            module.linkSystemLibrary("log", .{});
        } else {
            module.linkSystemLibrary("c", .{});
            module.linkSystemLibrary("m", .{});
            module.linkSystemLibrary("pthread", .{});
            module.linkSystemLibrary("dl", .{});
            module.linkSystemLibrary("unwind", .{});
        },
        .windows => {
            module.linkSystemLibrary("kernel32", .{});
            module.linkSystemLibrary("ws2_32", .{});
            module.linkSystemLibrary("userenv", .{});
            module.linkSystemLibrary("unwind", .{});
            module.linkSystemLibrary("c", .{});
        },
        .macos => module.linkSystemLibrary("c", .{}),
        .ios => {
            configureIosModule(context, module);
            module.linkSystemLibrary("System", .{});
        },
        else => module.linkSystemLibrary("c", .{}),
    }
    return libc_file;
}

fn configureAndroidModule(context: Context, module: *std.Build.Module) std.Build.LazyPath {
    const b = context.b;
    const ndk = context.android_ndk orelse @panic("Android Rust targets require -Dandroid-ndk=/absolute/path/to/ndk");
    if (context.android_api < 24) @panic("Android Rust targets require -Dandroid-api=24 or newer");
    const host = switch (@import("builtin").os.tag) {
        .macos => "darwin-x86_64",
        .linux => if (@import("builtin").cpu.arch == .aarch64) "linux-aarch64" else "linux-x86_64",
        else => @panic("Android NDK integration requires a macOS or Linux host"),
    };
    const arch = switch (context.target.result.cpu.arch) {
        .x86_64 => "x86_64-linux-android",
        .aarch64 => "aarch64-linux-android",
        else => unreachable,
    };
    const sysroot = b.fmt("{s}/toolchains/llvm/prebuilt/{s}/sysroot", .{ ndk, host });
    const include = b.fmt("{s}/usr/include", .{sysroot});
    const sys_include = b.fmt("{s}/usr/include/{s}", .{ sysroot, arch });
    const crt = b.fmt("{s}/usr/lib/{s}/{d}", .{ sysroot, arch, context.android_api });
    const files = b.addWriteFiles();
    const libc_file = files.add("android-libc.txt", b.fmt(
        "include_dir={s}\nsys_include_dir={s}\ncrt_dir={s}\nmsvc_lib_dir=\nkernel32_lib_dir=\ngcc_dir=\n",
        .{ include, sys_include, crt },
    ));
    module.addSystemIncludePath(b.graph.cwdRelativePath(include));
    module.addSystemIncludePath(b.graph.cwdRelativePath(sys_include));
    module.addLibraryPath(b.graph.cwdRelativePath(crt));

    const clang_root = b.fmt("{s}/toolchains/llvm/prebuilt/{s}/lib64/clang", .{ ndk, host });
    if (firstDirectory(b, clang_root)) |version| {
        const unwind_arch = if (context.target.result.cpu.arch == .aarch64) "aarch64" else "x86_64";
        module.addObjectFile(b.graph.cwdRelativePath(b.fmt("{s}/{s}/lib/linux/{s}/libunwind.a", .{ clang_root, version, unwind_arch })));
    }
    return libc_file;
}

fn configureIosModule(context: Context, module: *std.Build.Module) void {
    const b = context.b;
    const sdk_name = if (context.target.result.abi == .simulator) "iphonesimulator" else "iphoneos";
    const output = b.run(&.{ "xcrun", "--sdk", sdk_name, "--show-sdk-path" });
    const sdk = std.mem.trim(u8, output, &std.ascii.whitespace);
    if (sdk.len == 0) @panic("xcrun returned an empty iOS SDK path");
    module.addFrameworkPath(b.graph.cwdRelativePath(b.fmt("{s}/System/Library/Frameworks", .{sdk})));
    module.addSystemIncludePath(b.graph.cwdRelativePath(b.fmt("{s}/usr/include", .{sdk})));
    module.addLibraryPath(b.graph.cwdRelativePath(b.fmt("{s}/usr/lib", .{sdk})));
}

fn firstDirectory(b: *std.Build, path: []const u8) ?[]const u8 {
    var directory = std.Io.Dir.openDirAbsolute(b.graph.io, path, .{ .iterate = true }) catch return null;
    defer directory.close(b.graph.io);
    var iterator = directory.iterate();
    while (iterator.next(b.graph.io) catch return null) |entry| {
        if (entry.kind == .directory) return b.allocator.dupe(u8, entry.name) catch @panic("out of memory");
    }
    return null;
}

fn normalizeCrateName(b: *std.Build, name: []const u8) []const u8 {
    const result = b.allocator.dupe(u8, name) catch @panic("out of memory");
    for (result) |*byte| if (byte.* == '-') {
        byte.* = '_';
    };
    return result;
}

fn joinComma(b: *std.Build, values: []const []const u8) []const u8 {
    return std.mem.join(b.allocator, ",", values) catch @panic("out of memory");
}

fn cargoTargetVariable(b: *std.Build, triple: []const u8, suffix: []const u8) []const u8 {
    return b.fmt("CARGO_TARGET_{s}_{s}", .{ upperTarget(b, triple), suffix });
}

fn ccTargetVariable(b: *std.Build, triple: []const u8) []const u8 {
    const value=b.allocator.dupe(u8,triple) catch @panic("out of memory");
    for(value) |*byte| if(byte.* == '-' or byte.* == '.') { byte.*='_'; };
    return b.fmt("CC_{s}", .{value});
}

fn upperTarget(b: *std.Build, triple: []const u8) []const u8 {
    const value = b.allocator.dupe(u8, triple) catch @panic("out of memory");
    for (value) |*byte| {
        byte.* = if (byte.* == '-') '_' else std.ascii.toUpper(byte.*);
    }
    return value;
}
