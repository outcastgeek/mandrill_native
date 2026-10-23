//! Reusable upstream-CMake static-library builds using the shared Zig compiler
//! wrappers and the same native target configuration as the Rust bridge.
const std=@import("std");
const rust=@import("rust.zig");
pub const LibraryOptions=struct {
    name: []const u8,source: std.Build.LazyPath,archive_name: []const u8,
    cmake_exe: []const u8="cmake",
    dependencies: []const Library=&.{},arguments: []const []const u8=&.{},
};
pub const Library=struct {
    prefix: std.Build.LazyPath,archive: std.Build.LazyPath,build: *std.Build.Step.Run,
    pub fn link(self: @This(),artifact: *std.Build.Step.Compile) void {
        artifact.step.dependOn(&self.build.step); self.linkModule(artifact.root_module);
    }
    pub fn linkModule(self: @This(),module: *std.Build.Module) void {
        module.addObjectFile(self.archive);
        module.addIncludePath(self.prefix.path(module.owner,"include"));
    }
};
pub fn addLibrary(context: rust.Context,options: LibraryOptions) Library {
    const b=context.b; const command=b.addSystemCommand(&.{"sh"});
    command.addFileArg2(context.support_dir.path(b,"cmake-build"),.{});
    command.addArg(options.cmake_exe);
    command.addDirectoryArg2(options.source,.{});
    const prefix=command.addOutputDirectoryArg2(options.name,.{});
    command.addDirectoryArg2(context.support_dir,.{}); command.addFileArg2(.zig_exe,.{});
    command.addArg(rust.targetTriple(context.target.result));
    command.addArg(context.target.query.zigTriple(b.allocator) catch @panic("out of memory"));
    command.addArg(if(context.optimize == .debug) "Debug" else "Release");
    command.addArg(switch(context.target.result.os.tag) {.macos=>"Darwin",.ios=>"iOS",.windows=>"Windows",.linux=>if(context.target.result.abi == .android) "Android" else "Linux",else=>@panic("unsupported CMake target")});
    command.addArg(switch(context.target.result.cpu.arch) {.aarch64=>"aarch64",.x86_64=>"x86_64",else=>@panic("unsupported CMake architecture")});
    command.addArg(if(context.target.query.isNative()) "native" else "cross");
    command.addArg(b.fmt("{d}",.{options.dependencies.len}));
    for(options.dependencies) |dependency| {command.step.dependOn(&dependency.build.step); command.addDirectoryArg2(dependency.prefix,.{});}
    command.addArgs(options.arguments);
    rust.setTargetEnvironment(context,command);
    return .{.prefix=prefix,.archive=prefix.path(b,b.fmt("lib/{s}",.{options.archive_name})),.build=command};
}
