const std = @import("std");
const m = @import("../models.zig");
pub fn typesafeProvider(a: std.mem.Allocator) !m.Provider {
    return @import("base.zig").create(a, .{ .id = "typesafe", .name = "TypeSafe", .baseUrl = null, .authName = "TypeSafe API key", .env = &.{"TYPESAFE_API_KEY"}, .apis = &.{"typesafe-system-one"} }, @embedFile("../../data/typesafe.json"));
}
