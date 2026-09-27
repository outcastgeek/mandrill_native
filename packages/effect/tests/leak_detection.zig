const std = @import("std");

// These fixtures are run separately by check_leak_detection.py. The leaking
// fixture must fail its process even though its own assertions pass.
test "leak detector rejects unreleased allocation" {
    const value = try std.testing.allocator.create(u8);
    value.* = 42;
    try std.testing.expectEqual(@as(u8, 42), value.*);
}

test "leak detector accepts complete teardown" {
    const value = try std.testing.allocator.create(u8);
    defer std.testing.allocator.destroy(value);
    value.* = 42;
    try std.testing.expectEqual(@as(u8, 42), value.*);
}
