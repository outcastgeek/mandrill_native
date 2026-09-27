//! Reusable Node-API addon composition for Zig-owned native libraries.

const std = @import("std");

pub const Options = struct {
    support_dir: ?std.Build.LazyPath = null,
    c_abi_module: ?*std.Build.Module = null,
    name: []const u8,
    root_source_file: std.Build.LazyPath,
    target: std.Build.ResolvedTarget,
    optimize: std.builtin.OptimizeMode,
    /// Required consumer choice. The helper supplies no implicit ABI version.
    napi_version: u32,
    /// Directory containing node_api.h from the npm node-api-headers package.
    headers: std.Build.LazyPath,
    /// Shared process-lifetime `std.Io` execution module used by async,
    /// parallel and streaming helpers.
    execution_module: *std.Build.Module,
    /// npm node-api-headers definition files used to create the Windows host
    /// import library. Required only for Windows targets.
    windows_definitions: ?[2]std.Build.LazyPath = null,
    install_sub_path: ?[]const u8 = null,
};

pub const Addon = struct {
    artifact: *std.Build.Step.Compile,
    module: *std.Build.Module,
    api_module: *std.Build.Module,
    runtime_module: *std.Build.Module,
    c_abi_module: *std.Build.Module,
    stream_module: *std.Build.Module,
    install: *std.Build.Step.InstallArtifact,
};

pub fn addAddon(b: *std.Build, options: Options) Addon {
    if (options.napi_version == 0)
        @panic("Node-API consumers must select a positive napi_version");

    const support = options.support_dir orelse b.path("native_build");
    const api_options = b.addOptions();
    api_options.addOption([]const u8, "napi_version_text", b.fmt("{d}", .{options.napi_version}));

    const api_module = b.createModule(.{
        .root_source_file = support.path(b, "node_api/c.zig"),
        .target = options.target,
        .optimize = options.optimize,
    });
    api_module.addOptions("node_api_options", api_options);
    api_module.addSystemIncludePath(options.headers);

    const runtime_module = b.createModule(.{
        .root_source_file = support.path(b, "node_api/runtime.zig"),
        .target = options.target,
        .optimize = options.optimize,
    });
    const c_abi_module = options.c_abi_module orelse b.createModule(.{
        .root_source_file = support.path(b, "interop/c_abi.zig"),
        .target = options.target,
        .optimize = options.optimize,
    });
    runtime_module.addImport("node_api_c", api_module);
    runtime_module.addImport("native_execution", options.execution_module);
    runtime_module.addImport("native_c_abi", c_abi_module);

    const stream_module = b.createModule(.{
        .root_source_file = support.path(b, "node_api/stream.zig"),
        .target = options.target,
        .optimize = options.optimize,
    });
    stream_module.addImport("node_api_runtime", runtime_module);
    stream_module.addImport("native_execution", options.execution_module);

    const module = b.createModule(.{
        .root_source_file = options.root_source_file,
        .target = options.target,
        .optimize = options.optimize,
        .link_libc = true,
        .pic = true,
        .strip = options.optimize != .Debug,
    });
    module.addImport("node_api_c", api_module);
    module.addImport("node_api_runtime", runtime_module);
    module.addImport("node_api_stream", stream_module);
    module.addImport("native_execution", options.execution_module);

    const artifact = b.addLibrary(.{
        .name = options.name,
        .linkage = .dynamic,
        .root_module = module,
    });
    if (options.target.result.os.tag == .macos)
        artifact.linker_allow_shlib_undefined = true;
    if (options.target.result.os.tag == .windows) {
        const definitions = options.windows_definitions orelse
            @panic("Windows Node-API addons require the npm node_api.def and js_native_api.def paths");
        const merge = b.addSystemCommand(&.{
            "node",
            "-e",
            "const fs=require('node:fs');const out=process.argv[3];const symbols=new Set();for(const p of process.argv.slice(1,3)){for(const line of fs.readFileSync(p,'utf8').split(/\\r?\\n/)){const s=line.trim();if(s&&s!=='EXPORTS'&&!s.startsWith('NAME '))symbols.add(s)}}fs.writeFileSync(out,'NAME NODE.EXE\\nEXPORTS\\n'+[...symbols].join('\\n')+'\\n')",
        });
        merge.addFileArg(definitions[0]);
        merge.addFileArg(definitions[1]);
        const merged = merge.addOutputFileArg("node_api_imports.def");
        const dlltool = b.addSystemCommand(&.{ b.graph.zig_exe, "dlltool", "-D", "node.exe", "-d" });
        dlltool.addFileArg(merged);
        dlltool.addArg("-l");
        const import_library = dlltool.addOutputFileArg("node_api_imports.lib");
        dlltool.addArgs(&.{ "-m", if (options.target.result.cpu.arch == .aarch64) "arm64" else "i386:x86-64" });
        artifact.root_module.addObjectFile(import_library);
    }

    const file_name = options.install_sub_path orelse b.fmt("{s}.node", .{options.name});
    const install = b.addInstallArtifact(artifact, .{ .dest_sub_path = file_name });
    return .{
        .artifact = artifact,
        .module = module,
        .api_module = api_module,
        .runtime_module = runtime_module,
        .c_abi_module = c_abi_module,
        .stream_module = stream_module,
        .install = install,
    };
}
