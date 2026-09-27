//! Reusable Cargo/staticlib integration for Zig-owned programs.
//!
//! Rust libraries may import symbols exported by Zig. The final Zig link owns
//! symbol resolution, so Rust -> Zig -> Rust call chains require no special
//! trampoline and have no build-system depth limit.

const std = @import("std");

pub const Context = struct {
    b: *std.Build,
    target: std.Build.ResolvedTarget,
    optimize: std.builtin.OptimizeMode,
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
    /// Rust crate name as emitted on disk (`-` normalized to `_`).
    crate_name: []const u8,
    manifest_name: []const u8 = "Cargo.toml",
    features: []const []const u8 = &.{},
    no_default_features: bool = false,
    cargo_args: []const []const u8 = &.{},
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
        artifact.root_module.addObjectFile(self.archive);
        linkRuntime(self.context, artifact);
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
    const profile = if (context.optimize == .Debug) "debug" else "release";
    const target_dir = b.fmt(".zig-cache/rust/{s}", .{options.name});
    const target_dir_absolute = b.path(target_dir).getPath(b);
    const manifest = b.fmt("{s}/{s}", .{ options.project_dir, options.manifest_name });

    const cargo = b.addSystemCommand(&.{ context.cargo_exe, "build", "--manifest-path", manifest, "--target", rust_target, "--target-dir", target_dir_absolute });
    cargo.setCwd(b.path("."));
    if (context.optimize != .Debug) cargo.addArg("--release");
    if (options.no_default_features) cargo.addArg("--no-default-features");
    if (options.features.len != 0) {
        cargo.addArg("--features");
        cargo.addArg(joinComma(b, options.features));
    }
    cargo.addArgs(options.cargo_args);

    const linker = context.support_dir.path(b, "zcc").getPath(b);
    cargo.setEnvironmentVariable("ZIG_RUST_TARGET", rust_target);
    cargo.setEnvironmentVariable("ZIG_RUST_ZIG", b.graph.zig_exe);
    cargo.setEnvironmentVariable("ZIG_RUST_ANDROID_API", b.fmt("{d}", .{context.android_api}));
    if (context.android_ndk) |ndk| cargo.setEnvironmentVariable("ZIG_RUST_ANDROID_NDK", ndk);
    // Native Cargo build scripts and proc macros must use the host linker.
    // The target-specific Zig linker is only required for cross compilation.
    if (!context.target.query.isNative()) {
        const requested_triple = context.target.query.zigTriple(b.allocator) catch @panic("out of memory");
        cargo.setEnvironmentVariable("ZIG_RUST_ZIG_TARGET", requested_triple);
        cargo.setEnvironmentVariable(cargoTargetVariable(b, rust_target, "LINKER"), linker);
        cargo.setEnvironmentVariable(ccTargetVariable(b, rust_target), linker);
    }

    const archive_name = b.fmt("lib{s}.a", .{normalizeCrateName(b, options.crate_name)});
    const archive_path = b.fmt("{s}/{s}/{s}/{s}", .{ target_dir, rust_target, profile, archive_name });
    return .{
        .name = options.name,
        .rust_target = rust_target,
        .archive = b.path(archive_path),
        .build = cargo,
        .context = context,
    };
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

fn linkRuntime(context: Context, artifact: *std.Build.Step.Compile) void {
    switch (context.target.result.os.tag) {
        .linux => if (context.target.result.abi == .android) {
            configureAndroid(context, artifact);
            artifact.root_module.linkSystemLibrary("c", .{});
            artifact.root_module.linkSystemLibrary("m", .{});
            artifact.root_module.linkSystemLibrary("dl", .{});
            artifact.root_module.linkSystemLibrary("log", .{});
        } else {
            artifact.root_module.linkSystemLibrary("c", .{});
            artifact.root_module.linkSystemLibrary("m", .{});
            artifact.root_module.linkSystemLibrary("pthread", .{});
            artifact.root_module.linkSystemLibrary("dl", .{});
            artifact.root_module.linkSystemLibrary("unwind", .{});
        },
        .windows => {
            artifact.root_module.linkSystemLibrary("kernel32", .{});
            artifact.root_module.linkSystemLibrary("ws2_32", .{});
            artifact.root_module.linkSystemLibrary("userenv", .{});
            artifact.root_module.linkSystemLibrary("unwind", .{});
            artifact.root_module.linkSystemLibrary("c", .{});
        },
        .macos => artifact.root_module.linkSystemLibrary("c", .{}),
        .ios => {
            configureIos(context, artifact);
            artifact.root_module.linkSystemLibrary("System", .{});
        },
        else => artifact.root_module.linkSystemLibrary("c", .{}),
    }
}

fn configureAndroid(context: Context, artifact: *std.Build.Step.Compile) void {
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
    artifact.setLibCFile(libc_file);
    artifact.root_module.addSystemIncludePath(.{ .cwd_relative = include });
    artifact.root_module.addSystemIncludePath(.{ .cwd_relative = sys_include });
    artifact.root_module.addLibraryPath(.{ .cwd_relative = crt });

    const clang_root = b.fmt("{s}/toolchains/llvm/prebuilt/{s}/lib64/clang", .{ ndk, host });
    if (firstDirectory(b, clang_root)) |version| {
        const unwind_arch = if (context.target.result.cpu.arch == .aarch64) "aarch64" else "x86_64";
        artifact.root_module.addObjectFile(.{ .cwd_relative = b.fmt("{s}/{s}/lib/linux/{s}/libunwind.a", .{ clang_root, version, unwind_arch }) });
    }
}

fn configureIos(context: Context, artifact: *std.Build.Step.Compile) void {
    const b = context.b;
    const sdk_name = if (context.target.result.abi == .simulator) "iphonesimulator" else "iphoneos";
    const output = b.run(&.{ "xcrun", "--sdk", sdk_name, "--show-sdk-path" });
    const sdk = std.mem.trim(u8, output, &std.ascii.whitespace);
    if (sdk.len == 0) @panic("xcrun returned an empty iOS SDK path");
    artifact.root_module.addFrameworkPath(.{ .cwd_relative = b.fmt("{s}/System/Library/Frameworks", .{sdk}) });
    artifact.root_module.addSystemIncludePath(.{ .cwd_relative = b.fmt("{s}/usr/include", .{sdk}) });
    artifact.root_module.addLibraryPath(.{ .cwd_relative = b.fmt("{s}/usr/lib", .{sdk}) });
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
    return b.fmt("CC_{s}", .{upperTarget(b, triple)});
}

fn upperTarget(b: *std.Build, triple: []const u8) []const u8 {
    const value = b.allocator.dupe(u8, triple) catch @panic("out of memory");
    for (value) |*byte| {
        byte.* = if (byte.* == '-') '_' else std.ascii.toUpper(byte.*);
    }
    return value;
}
