const std = @import("std");

pub fn build(b: *std.Build) void {
    const leak = b.option(bool, "leak", "Run the deliberately leaking fixture") orelse false;
    const fixture = b.addTest(.{
        .root_module = b.createModule(.{
            .root_source_file = b.path("leak_detection.zig"),
            .target = b.graph.host,
            .optimize = b.standardOptimizeOption(.{}),
        }),
        .filters = &.{if (leak) "leak detector rejects unreleased allocation" else "leak detector accepts complete teardown"},
    });
    b.default_step.dependOn(&b.addRunArtifact(fixture).step);
}
