const std = @import("std");
pub const rust = @import("rust.zig");
pub const cmake=@import("cmake.zig");
pub fn build(b: *std.Build) void {
    _ = b.addModule("c_abi", .{ .root_source_file = b.path("interop/c_abi.zig") });
}
